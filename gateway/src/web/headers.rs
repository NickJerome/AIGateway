//! `Headers`: lowercased storage, combined `get`, Set-Cookie never merged.

use rquickjs::Symbol;
use rquickjs::class::{JsClass, Trace, Tracer, Writable};
use rquickjs::function::{Opt, This};
use rquickjs::{
    Array, Class, Coerced, Constructor, Ctx, Exception, FromJs, Function, IntoJs, JsLifetime,
    Object, Result, Value,
};
use std::string::String as StdString;

/// Header storage. Names are lowercased on the way in; values are kept
/// verbatim. Duplicate names are legal; `get` combines every name except
/// Set-Cookie.
pub struct Headers {
    list: Vec<(StdString, StdString)>,
}

impl Headers {
    pub(crate) fn new() -> Self {
        Headers { list: Vec::new() }
    }

    /// Rebuild from a raw list (Request/Response clone and slot extraction).
    pub(crate) fn from_list(list: Vec<(StdString, StdString)>) -> Self {
        Headers { list }
    }

    /// The raw name/value pairs in insertion order (names already lowercase).
    pub(crate) fn raw_list(&self) -> Vec<(StdString, StdString)> {
        self.list.clone()
    }

    /// Constructor init: another Headers (cloned), an array of [name, value]
    /// pairs, or a record. Arbitrary iterables are not supported.
    pub(crate) fn from_init<'js>(ctx: &Ctx<'js>, init: Value<'js>) -> Result<Headers> {
        if let Ok(other) = Class::<Headers>::from_js(ctx, init.clone()) {
            return Ok(Headers {
                list: other.borrow().list.clone(),
            });
        }
        let mut headers = Headers::new();
        if let Some(array) = init.as_array() {
            for index in 0..array.len() {
                let entry: Value = array.get(index)?;
                let Some(pair) = entry.as_array() else {
                    return Err(Exception::throw_type(
                        ctx,
                        "Headers init entry must be a [name, value] pair",
                    ));
                };
                if pair.len() != 2 {
                    return Err(Exception::throw_type(
                        ctx,
                        "Headers init entry must be a [name, value] pair",
                    ));
                }
                let name: Coerced<StdString> = pair.get(0)?;
                let value: Coerced<StdString> = pair.get(1)?;
                headers.append_raw(&name.0, &value.0);
            }
            return Ok(headers);
        }
        if let Some(obj) = init.as_object() {
            for entry in obj.props::<StdString, Coerced<StdString>>() {
                let (name, value) = entry?;
                headers.append_raw(&name, &value.0);
            }
            return Ok(headers);
        }
        Err(Exception::throw_type(
            ctx,
            "Headers init must be a Headers, an array of pairs, or a record",
        ))
    }

    pub(crate) fn append_raw(&mut self, name: &str, value: &str) {
        self.list
            .push((name.to_ascii_lowercase(), value.to_string()));
    }

    fn get(&self, name: &str) -> Option<StdString> {
        let name = name.to_ascii_lowercase();
        let mut values = self
            .list
            .iter()
            .filter(|(n, _)| *n == name)
            .map(|(_, v)| v.as_str());
        let first = values.next()?;
        if name == "set-cookie" {
            // Set-Cookie is never combined; `get` returns the first value.
            return Some(first.to_string());
        }
        let mut combined = StdString::from(first);
        for value in values {
            combined.push_str(", ");
            combined.push_str(value);
        }
        Some(combined)
    }

    fn get_set_cookie(&self) -> Vec<StdString> {
        self.list
            .iter()
            .filter(|(n, _)| n == "set-cookie")
            .map(|(_, v)| v.clone())
            .collect()
    }

    fn set(&mut self, name: &str, value: &str) {
        // Replace the first occurrence in place and drop the rest, per spec.
        let name = name.to_ascii_lowercase();
        let mut replaced = false;
        self.list.retain_mut(|(n, v)| {
            if *n == name {
                if replaced {
                    return false;
                }
                replaced = true;
                *v = value.to_string();
            }
            true
        });
        if !replaced {
            self.list.push((name, value.to_string()));
        }
    }

    fn delete(&mut self, name: &str) {
        let name = name.to_ascii_lowercase();
        self.list.retain(|(n, _)| *n != name);
    }

    pub(crate) fn has(&self, name: &str) -> bool {
        let name = name.to_ascii_lowercase();
        self.list.iter().any(|(n, _)| *n == name)
    }

    /// Iteration view: sorted by name (byte order), stable within one name;
    /// same-name values combine with ", ", except Set-Cookie which yields one
    /// entry per value.
    fn entries_view(&self) -> Vec<(StdString, StdString)> {
        let mut indexed: Vec<(usize, &(StdString, StdString))> =
            self.list.iter().enumerate().collect();
        indexed.sort_by(|(ai, (an, _)), (bi, (bn, _))| an.cmp(bn).then(ai.cmp(bi)));
        let mut out: Vec<(StdString, StdString)> = Vec::new();
        for (_, (name, value)) in indexed {
            if name != "set-cookie"
                && let Some((last_name, last_value)) = out.last_mut()
                && *last_name == *name
            {
                last_value.push_str(", ");
                last_value.push_str(value);
                continue;
            }
            out.push((name.clone(), value.clone()));
        }
        out
    }
}

impl<'js> Trace<'js> for Headers {
    fn trace<'a>(&self, _tracer: Tracer<'a, 'js>) {}
}

// Headers holds no JS references; the lifetime change is trivially identity.
unsafe impl<'js> JsLifetime<'js> for Headers {
    type Changed<'to> = Headers;
}

impl<'js> JsClass<'js> for Headers {
    const NAME: &'static str = "Headers";

    type Mutable = Writable;

    fn prototype(ctx: &Ctx<'js>) -> Result<Option<Object<'js>>> {
        let proto = Object::new(ctx.clone())?;
        proto.set("append", Function::new(ctx.clone(), headers_append)?)?;
        proto.set("delete", Function::new(ctx.clone(), headers_delete)?)?;
        proto.set("get", Function::new(ctx.clone(), headers_get)?)?;
        proto.set(
            "getSetCookie",
            Function::new(ctx.clone(), headers_get_set_cookie)?,
        )?;
        proto.set("has", Function::new(ctx.clone(), headers_has)?)?;
        proto.set("set", Function::new(ctx.clone(), headers_set)?)?;
        let entries = Function::new(ctx.clone(), headers_entries)?;
        proto.set("entries", entries.clone())?;
        // `for..of` over a Headers yields the entries snapshot, per the
        // Headers spec (Symbol.iterator aliases entries).
        proto.set(Symbol::iterator(ctx.clone()).as_atom(), entries)?;
        proto.set("keys", Function::new(ctx.clone(), headers_keys)?)?;
        proto.set("values", Function::new(ctx.clone(), headers_values)?)?;
        proto.set("forEach", Function::new(ctx.clone(), headers_for_each)?)?;
        Ok(Some(proto))
    }

    fn constructor(ctx: &Ctx<'js>) -> Result<Option<Constructor<'js>>> {
        let ctor = Constructor::new_class::<Headers, _, _>(
            ctx.clone(),
            |ctx: Ctx<'js>, init: Opt<Value<'js>>| -> Result<Class<'js, Headers>> {
                let headers = match init.0 {
                    Some(value) if !value.is_undefined() && !value.is_null() => {
                        Headers::from_init(&ctx, value)?
                    }
                    _ => Headers::new(),
                };
                Class::instance(ctx, headers)
            },
        )?;
        Ok(Some(ctor))
    }
}

fn headers_append<'js>(
    this: This<Class<'js, Headers>>,
    name: Coerced<StdString>,
    value: Coerced<StdString>,
) {
    this.0.borrow_mut().append_raw(&name.0, &value.0);
}

fn headers_delete<'js>(this: This<Class<'js, Headers>>, name: Coerced<StdString>) {
    this.0.borrow_mut().delete(&name.0);
}

fn headers_get<'js>(
    ctx: Ctx<'js>,
    this: This<Class<'js, Headers>>,
    name: Coerced<StdString>,
) -> Result<Value<'js>> {
    // A missing header is null, not undefined (`Option` would map to undefined).
    match this.0.borrow().get(&name.0) {
        Some(value) => value.into_js(&ctx),
        None => Ok(Value::new_null(ctx)),
    }
}

fn headers_get_set_cookie<'js>(
    ctx: Ctx<'js>,
    this: This<Class<'js, Headers>>,
) -> Result<Array<'js>> {
    let out = Array::new(ctx)?;
    for value in this.0.borrow().get_set_cookie() {
        out.set(out.len(), value)?;
    }
    Ok(out)
}

fn headers_has<'js>(this: This<Class<'js, Headers>>, name: Coerced<StdString>) -> bool {
    this.0.borrow().has(&name.0)
}

fn headers_set<'js>(
    this: This<Class<'js, Headers>>,
    name: Coerced<StdString>,
    value: Coerced<StdString>,
) {
    this.0.borrow_mut().set(&name.0, &value.0);
}

fn headers_entries<'js>(ctx: Ctx<'js>, this: This<Class<'js, Headers>>) -> Result<Value<'js>> {
    let snapshot = pairs_to_arrays(&ctx, &this.0.borrow().entries_view())?;
    crate::web::array_iterator(&ctx, snapshot)
}

fn headers_keys<'js>(ctx: Ctx<'js>, this: This<Class<'js, Headers>>) -> Result<Value<'js>> {
    let out = Array::new(ctx.clone())?;
    for (name, _) in this.0.borrow().entries_view() {
        out.set(out.len(), name)?;
    }
    crate::web::array_iterator(&ctx, out)
}

fn headers_values<'js>(ctx: Ctx<'js>, this: This<Class<'js, Headers>>) -> Result<Value<'js>> {
    let out = Array::new(ctx.clone())?;
    for (_, value) in this.0.borrow().entries_view() {
        out.set(out.len(), value)?;
    }
    crate::web::array_iterator(&ctx, out)
}

fn headers_for_each<'js>(this: This<Class<'js, Headers>>, callback: Function<'js>) -> Result<()> {
    // `entries_view` returns owned data: no borrow is held across the callback.
    for (name, value) in this.0.borrow().entries_view() {
        callback.call::<_, ()>((value, name, this.0.clone()))?;
    }
    Ok(())
}

fn pairs_to_arrays<'js>(ctx: &Ctx<'js>, pairs: &[(StdString, StdString)]) -> Result<Array<'js>> {
    let out = Array::new(ctx.clone())?;
    for (name, value) in pairs {
        let pair = Array::new(ctx.clone())?;
        pair.set(0, name.clone())?;
        pair.set(1, value.clone())?;
        out.set(out.len(), pair)?;
    }
    Ok(out)
}
