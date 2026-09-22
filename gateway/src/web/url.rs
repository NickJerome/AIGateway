//! `URL` and `URLSearchParams` over the `url` crate (WHATWG URL Standard).
//! `url.searchParams` is a live object: mutations write through to the URL
//! query, and assigning `url.search` resyncs the params object.

use rquickjs::class::{JsClass, Trace, Tracer, Writable};
use rquickjs::function::{Opt, This};
use rquickjs::object::Accessor;
use rquickjs::{
    Array, Class, Coerced, Constructor, Ctx, Exception, FromJs, Function, IntoJs, JsLifetime,
    Object, Result, Symbol, Value,
};
use std::string::String as StdString;
use url::Url as StdUrl;

/// Parse `input` with an optional base. The base is parsed first, so an
/// illegal base throws even when `input` is absolute (browser behavior).
fn parse_url(ctx: &Ctx<'_>, input: &str, base: Option<&StdString>) -> Result<StdUrl> {
    let result = match base {
        Some(base) => match StdUrl::parse(base) {
            Ok(base) => base.join(input),
            Err(err) => return Err(Exception::throw_type(ctx, &format!("Invalid URL: {err}"))),
        },
        None => StdUrl::parse(input),
    };
    result.map_err(|err| Exception::throw_type(ctx, &format!("Invalid URL: {err}")))
}

fn parse_query(query: &str) -> Vec<(StdString, StdString)> {
    url::form_urlencoded::parse(query.as_bytes())
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect()
}

fn serialize_pairs(pairs: &[(StdString, StdString)]) -> StdString {
    url::form_urlencoded::Serializer::new(StdString::new())
        .extend_pairs(pairs.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .finish()
}

/// `URL`: wraps `url::Url`; the linked `searchParams` is created lazily.
pub struct Url<'js> {
    inner: StdUrl,
    params: Option<Class<'js, UrlSearchParams<'js>>>,
}

/// `URLSearchParams`: insertion-ordered pairs. `url` links back to the URL
/// this object was taken from, if any.
pub struct UrlSearchParams<'js> {
    pairs: Vec<(StdString, StdString)>,
    url: Option<Class<'js, Url<'js>>>,
}

impl<'js> UrlSearchParams<'js> {
    /// Constructor init: a query string, a record, an array of pairs, or
    /// another URLSearchParams (cloned). Arbitrary iterables are unsupported.
    fn from_init(ctx: &Ctx<'js>, init: Value<'js>) -> Result<Vec<(StdString, StdString)>> {
        if init.is_string() {
            let text = init.into_string().unwrap().to_string()?;
            return Ok(parse_query(text.strip_prefix('?').unwrap_or(&text)));
        }
        if let Ok(other) = Class::<UrlSearchParams>::from_js(ctx, init.clone()) {
            return Ok(other.borrow().pairs.clone());
        }
        let mut pairs = Vec::new();
        if let Some(array) = init.as_array() {
            for index in 0..array.len() {
                let entry: Value = array.get(index)?;
                let Some(pair) = entry.as_array() else {
                    return Err(Exception::throw_type(
                        ctx,
                        "URLSearchParams init entry must be a [name, value] pair",
                    ));
                };
                if pair.len() != 2 {
                    return Err(Exception::throw_type(
                        ctx,
                        "URLSearchParams init entry must be a [name, value] pair",
                    ));
                }
                let name: Coerced<StdString> = pair.get(0)?;
                let value: Coerced<StdString> = pair.get(1)?;
                pairs.push((name.0, value.0));
            }
            return Ok(pairs);
        }
        if let Some(obj) = init.as_object() {
            for entry in obj.props::<StdString, Coerced<StdString>>() {
                let (name, value) = entry?;
                pairs.push((name, value.0));
            }
            return Ok(pairs);
        }
        Err(Exception::throw_type(
            ctx,
            "URLSearchParams init must be a string, record, array of pairs, or URLSearchParams",
        ))
    }

    /// The `application/x-www-form-urlencoded` serialization (BodyInit use).
    pub(crate) fn serialized(&self) -> StdString {
        serialize_pairs(&self.pairs)
    }

    /// Write a mutation through to the linked URL, if this is a live
    /// `searchParams` object.
    fn sync_url(&self) {
        if let Some(url) = &self.url {
            let query = serialize_pairs(&self.pairs);
            let mut url = url.borrow_mut();
            if query.is_empty() {
                url.inner.set_query(None);
            } else {
                url.inner.set_query(Some(&query));
            }
        }
    }

    fn get(&self, name: &str) -> Option<StdString> {
        self.pairs
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.clone())
    }

    fn set(&mut self, name: &str, value: &str) {
        // Replace the first occurrence in place and drop the rest, per spec.
        let mut replaced = false;
        self.pairs.retain_mut(|(n, v)| {
            if n == name {
                if replaced {
                    return false;
                }
                replaced = true;
                *v = value.to_string();
            }
            true
        });
        if !replaced {
            self.pairs.push((name.to_string(), value.to_string()));
        }
    }

    fn delete(&mut self, name: &str) {
        self.pairs.retain(|(n, _)| n != name);
    }

    fn sort(&mut self) {
        // Vec::sort_by is stable: equal names keep insertion order.
        self.pairs.sort_by(|(a, _), (b, _)| a.cmp(b));
    }
}

impl<'js> Trace<'js> for Url<'js> {
    fn trace<'a>(&self, tracer: Tracer<'a, 'js>) {
        self.params.trace(tracer);
    }
}

unsafe impl<'js> JsLifetime<'js> for Url<'js> {
    type Changed<'to> = Url<'to>;
}

impl<'js> Trace<'js> for UrlSearchParams<'js> {
    fn trace<'a>(&self, tracer: Tracer<'a, 'js>) {
        self.url.trace(tracer);
    }
}

unsafe impl<'js> JsLifetime<'js> for UrlSearchParams<'js> {
    type Changed<'to> = UrlSearchParams<'to>;
}

impl<'js> JsClass<'js> for Url<'js> {
    const NAME: &'static str = "URL";

    type Mutable = Writable;

    fn prototype(ctx: &Ctx<'js>) -> Result<Option<Object<'js>>> {
        let proto = Object::new(ctx.clone())?;
        proto.prop("href", Accessor::new(url_href, url_set_href))?;
        proto.prop("protocol", Accessor::new(url_protocol, url_set_protocol))?;
        proto.prop("username", Accessor::new(url_username, url_set_username))?;
        proto.prop("password", Accessor::new(url_password, url_set_password))?;
        proto.prop("host", Accessor::new(url_host, url_set_host))?;
        proto.prop("hostname", Accessor::new(url_hostname, url_set_hostname))?;
        proto.prop("port", Accessor::new(url_port, url_set_port))?;
        proto.prop("pathname", Accessor::new(url_pathname, url_set_pathname))?;
        proto.prop("search", Accessor::new(url_search, url_set_search))?;
        proto.prop("hash", Accessor::new(url_hash, url_set_hash))?;
        proto.prop("origin", Accessor::from(url_origin))?;
        proto.prop("searchParams", Accessor::from(url_search_params))?;
        proto.set("toString", Function::new(ctx.clone(), url_href)?)?;
        proto.set("toJSON", Function::new(ctx.clone(), url_href)?)?;
        Ok(Some(proto))
    }

    fn constructor(ctx: &Ctx<'js>) -> Result<Option<Constructor<'js>>> {
        let ctor = Constructor::new_class::<Url<'js>, _, _>(
            ctx.clone(),
            |ctx: Ctx<'js>,
             input: Coerced<StdString>,
             base: Opt<Coerced<StdString>>|
             -> Result<Class<'js, Url<'js>>> {
                let inner = parse_url(&ctx, &input.0, base.0.as_ref().map(|b| &b.0))?;
                Class::instance(
                    ctx,
                    Url {
                        inner,
                        params: None,
                    },
                )
            },
        )?;
        Ok(Some(ctor))
    }
}

impl<'js> JsClass<'js> for UrlSearchParams<'js> {
    const NAME: &'static str = "URLSearchParams";

    type Mutable = Writable;

    fn prototype(ctx: &Ctx<'js>) -> Result<Option<Object<'js>>> {
        let proto = Object::new(ctx.clone())?;
        proto.set("append", Function::new(ctx.clone(), params_append)?)?;
        proto.set("delete", Function::new(ctx.clone(), params_delete)?)?;
        proto.set("get", Function::new(ctx.clone(), params_get)?)?;
        proto.set("getAll", Function::new(ctx.clone(), params_get_all)?)?;
        proto.set("has", Function::new(ctx.clone(), params_has)?)?;
        proto.set("set", Function::new(ctx.clone(), params_set)?)?;
        proto.set("sort", Function::new(ctx.clone(), params_sort)?)?;
        proto.set("toString", Function::new(ctx.clone(), params_to_string)?)?;
        let entries = Function::new(ctx.clone(), params_entries)?;
        proto.set("entries", entries.clone())?;
        // `for..of` over URLSearchParams yields the entries snapshot, per
        // the URL spec (Symbol.iterator aliases entries).
        proto.set(Symbol::iterator(ctx.clone()).as_atom(), entries)?;
        proto.set("keys", Function::new(ctx.clone(), params_keys)?)?;
        proto.set("values", Function::new(ctx.clone(), params_values)?)?;
        proto.set("forEach", Function::new(ctx.clone(), params_for_each)?)?;
        proto.prop("size", Accessor::from(params_size))?;
        Ok(Some(proto))
    }

    fn constructor(ctx: &Ctx<'js>) -> Result<Option<Constructor<'js>>> {
        let ctor = Constructor::new_class::<UrlSearchParams<'js>, _, _>(
            ctx.clone(),
            |ctx: Ctx<'js>, init: Opt<Value<'js>>| -> Result<Class<'js, UrlSearchParams<'js>>> {
                let pairs = match init.0 {
                    Some(value) if !value.is_undefined() && !value.is_null() => {
                        UrlSearchParams::from_init(&ctx, value)?
                    }
                    _ => Vec::new(),
                };
                Class::instance(ctx, UrlSearchParams { pairs, url: None })
            },
        )?;
        Ok(Some(ctor))
    }
}

// --- URL accessors ---

fn url_href<'js>(this: This<Class<'js, Url<'js>>>) -> StdString {
    this.0.borrow().inner.to_string()
}

fn url_set_href<'js>(
    ctx: Ctx<'js>,
    this: This<Class<'js, Url<'js>>>,
    value: Coerced<StdString>,
) -> Result<()> {
    let inner = parse_url(&ctx, &value.0, None)?;
    let params = this.0.borrow().params.clone();
    this.0.borrow_mut().inner = inner;
    resync_params_after_url_change(&this.0, params);
    Ok(())
}

fn url_protocol<'js>(this: This<Class<'js, Url<'js>>>) -> StdString {
    format!("{}:", this.0.borrow().inner.scheme())
}

fn url_set_protocol<'js>(this: This<Class<'js, Url<'js>>>, value: Coerced<StdString>) {
    let scheme = value.0.trim_end_matches(':');
    let _ = this.0.borrow_mut().inner.set_scheme(scheme);
}

fn url_username<'js>(this: This<Class<'js, Url<'js>>>) -> StdString {
    this.0.borrow().inner.username().to_string()
}

fn url_set_username<'js>(this: This<Class<'js, Url<'js>>>, value: Coerced<StdString>) {
    let _ = this.0.borrow_mut().inner.set_username(&value.0);
}

fn url_password<'js>(this: This<Class<'js, Url<'js>>>) -> StdString {
    this.0
        .borrow()
        .inner
        .password()
        .unwrap_or_default()
        .to_string()
}

fn url_set_password<'js>(this: This<Class<'js, Url<'js>>>, value: Coerced<StdString>) {
    let _ = this
        .0
        .borrow_mut()
        .inner
        .set_password(if value.0.is_empty() {
            None
        } else {
            Some(&value.0)
        });
}

fn url_host<'js>(this: This<Class<'js, Url<'js>>>) -> StdString {
    let url = this.0.borrow();
    let host = url.inner.host_str().unwrap_or_default().to_string();
    match url.inner.port() {
        Some(port) => format!("{host}:{port}"),
        None => host,
    }
}

fn url_set_host<'js>(this: This<Class<'js, Url<'js>>>, value: Coerced<StdString>) {
    // The WHATWG host setter accepts a "host:port" value: set the host, then
    // the port suffix if present and non-empty ("host:" leaves the port
    // unchanged).
    let (host, port) = split_host_port(&value.0);
    let mut url = this.0.borrow_mut();
    if url.inner.set_host(Some(host)).is_err() {
        return;
    }
    if let Some(port) = port
        && let Ok(port) = port.parse::<u16>()
    {
        // rust-url drops the port when it is the scheme default.
        let _ = url.inner.set_port(Some(port));
    }
}

/// Split a host setter value into the host and an optional non-empty port
/// suffix. IPv6 literals (`[::1]:8080`) keep their brackets with the host.
fn split_host_port(input: &str) -> (&str, Option<&str>) {
    let Some(colon) = input.rfind(':') else {
        return (input, None);
    };
    if input.rfind(']').is_some_and(|close| close > colon) {
        return (input, None);
    }
    let host = &input[..colon];
    let port = &input[colon + 1..];
    if port.is_empty() {
        return (host, None);
    }
    (host, Some(port))
}

fn url_hostname<'js>(this: This<Class<'js, Url<'js>>>) -> StdString {
    this.0
        .borrow()
        .inner
        .host_str()
        .unwrap_or_default()
        .to_string()
}

fn url_set_hostname<'js>(this: This<Class<'js, Url<'js>>>, value: Coerced<StdString>) {
    let _ = this.0.borrow_mut().inner.set_host(Some(&value.0));
}

fn url_port<'js>(this: This<Class<'js, Url<'js>>>) -> StdString {
    this.0
        .borrow()
        .inner
        .port()
        .map(|p| p.to_string())
        .unwrap_or_default()
}

fn url_set_port<'js>(this: This<Class<'js, Url<'js>>>, value: Coerced<StdString>) {
    let port = if value.0.is_empty() {
        None
    } else {
        match value.0.parse::<u16>() {
            Ok(port) => Some(port),
            Err(_) => return, // Invalid ports are ignored, as in browsers.
        }
    };
    let _ = this.0.borrow_mut().inner.set_port(port);
}

fn url_pathname<'js>(this: This<Class<'js, Url<'js>>>) -> StdString {
    this.0.borrow().inner.path().to_string()
}

fn url_set_pathname<'js>(this: This<Class<'js, Url<'js>>>, value: Coerced<StdString>) {
    this.0.borrow_mut().inner.set_path(&value.0);
}

fn url_search<'js>(this: This<Class<'js, Url<'js>>>) -> StdString {
    match this.0.borrow().inner.query() {
        Some(query) => format!("?{query}"),
        None => StdString::new(),
    }
}

fn url_set_search<'js>(this: This<Class<'js, Url<'js>>>, value: Coerced<StdString>) {
    let query = value.0.strip_prefix('?').unwrap_or(&value.0).to_string();
    let params = this.0.borrow().params.clone();
    {
        let mut url = this.0.borrow_mut();
        if query.is_empty() {
            url.inner.set_query(None);
        } else {
            url.inner.set_query(Some(&query));
        }
    }
    resync_params_after_url_change(&this.0, params);
}

/// After a URL-side query change, re-parse the query into the linked
/// searchParams object so the two views never disagree.
fn resync_params_after_url_change<'js>(
    url: &Class<'js, Url<'js>>,
    params: Option<Class<'js, UrlSearchParams<'js>>>,
) {
    if let Some(params) = params {
        let query = url.borrow().inner.query().unwrap_or_default().to_string();
        params.borrow_mut().pairs = parse_query(&query);
    }
}

fn url_hash<'js>(this: This<Class<'js, Url<'js>>>) -> StdString {
    match this.0.borrow().inner.fragment() {
        Some(fragment) => format!("#{fragment}"),
        None => StdString::new(),
    }
}

fn url_set_hash<'js>(this: This<Class<'js, Url<'js>>>, value: Coerced<StdString>) {
    let fragment = value.0.strip_prefix('#').unwrap_or(&value.0).to_string();
    let mut url = this.0.borrow_mut();
    if fragment.is_empty() {
        url.inner.set_fragment(None);
    } else {
        url.inner.set_fragment(Some(&fragment));
    }
}

fn url_origin<'js>(this: This<Class<'js, Url<'js>>>) -> StdString {
    this.0.borrow().inner.origin().unicode_serialization()
}

fn url_search_params<'js>(
    ctx: Ctx<'js>,
    this: This<Class<'js, Url<'js>>>,
) -> Result<Class<'js, UrlSearchParams<'js>>> {
    if let Some(params) = this.0.borrow().params.clone() {
        return Ok(params);
    }
    let pairs = parse_query(this.0.borrow().inner.query().unwrap_or_default());
    let params = Class::instance(
        ctx,
        UrlSearchParams {
            pairs,
            url: Some(this.0.clone()),
        },
    )?;
    this.0.borrow_mut().params = Some(params.clone());
    Ok(params)
}

/// `URL.canParse(input, base?)`: parses exactly like the constructor.
pub fn url_can_parse(input: Coerced<StdString>, base: Opt<Coerced<StdString>>) -> bool {
    let result = match base.0 {
        Some(base) => StdUrl::parse(&base.0).and_then(|base| base.join(&input.0)),
        None => StdUrl::parse(&input.0),
    };
    result.is_ok()
}

/// `URL.parse(input, base?)`: the URL or null.
pub fn url_parse<'js>(
    ctx: Ctx<'js>,
    input: Coerced<StdString>,
    base: Opt<Coerced<StdString>>,
) -> Result<Value<'js>> {
    match parse_url(&ctx, &input.0, base.0.as_ref().map(|b| &b.0)) {
        Ok(inner) => Class::instance(
            ctx.clone(),
            Url {
                inner,
                params: None,
            },
        )?
        .into_js(&ctx),
        Err(_) => Ok(Value::new_null(ctx)),
    }
}

// --- URLSearchParams methods ---

fn params_append<'js>(
    this: This<Class<'js, UrlSearchParams<'js>>>,
    name: Coerced<StdString>,
    value: Coerced<StdString>,
) {
    let mut params = this.0.borrow_mut();
    params.pairs.push((name.0, value.0));
    params.sync_url();
}

fn params_delete<'js>(
    this: This<Class<'js, UrlSearchParams<'js>>>,
    name: Coerced<StdString>,
    value: Opt<Coerced<StdString>>,
) {
    let mut params = this.0.borrow_mut();
    match value.0 {
        Some(value) => params
            .pairs
            .retain(|(n, v)| !(n == &name.0 && v == &value.0)),
        None => params.delete(&name.0),
    }
    params.sync_url();
}

fn params_get<'js>(
    ctx: Ctx<'js>,
    this: This<Class<'js, UrlSearchParams<'js>>>,
    name: Coerced<StdString>,
) -> Result<Value<'js>> {
    match this.0.borrow().get(&name.0) {
        Some(value) => value.into_js(&ctx),
        None => Ok(Value::new_null(ctx)),
    }
}

fn params_get_all<'js>(
    ctx: Ctx<'js>,
    this: This<Class<'js, UrlSearchParams<'js>>>,
    name: Coerced<StdString>,
) -> Result<Array<'js>> {
    let out = Array::new(ctx)?;
    for (_, value) in this.0.borrow().pairs.iter().filter(|(n, _)| n == &name.0) {
        out.set(out.len(), value.clone())?;
    }
    Ok(out)
}

fn params_has<'js>(
    this: This<Class<'js, UrlSearchParams<'js>>>,
    name: Coerced<StdString>,
    value: Opt<Coerced<StdString>>,
) -> bool {
    let params = this.0.borrow();
    match value.0 {
        Some(value) => params
            .pairs
            .iter()
            .any(|(n, v)| n == &name.0 && v == &value.0),
        None => params.pairs.iter().any(|(n, _)| n == &name.0),
    }
}

fn params_set<'js>(
    this: This<Class<'js, UrlSearchParams<'js>>>,
    name: Coerced<StdString>,
    value: Coerced<StdString>,
) {
    let mut params = this.0.borrow_mut();
    params.set(&name.0, &value.0);
    params.sync_url();
}

fn params_sort<'js>(this: This<Class<'js, UrlSearchParams<'js>>>) {
    let mut params = this.0.borrow_mut();
    params.sort();
    params.sync_url();
}

fn params_to_string<'js>(this: This<Class<'js, UrlSearchParams<'js>>>) -> StdString {
    serialize_pairs(&this.0.borrow().pairs)
}

fn params_entries<'js>(
    ctx: Ctx<'js>,
    this: This<Class<'js, UrlSearchParams<'js>>>,
) -> Result<Value<'js>> {
    let out = Array::new(ctx.clone())?;
    for (name, value) in &this.0.borrow().pairs {
        let pair = Array::new(ctx.clone())?;
        pair.set(0, name.clone())?;
        pair.set(1, value.clone())?;
        out.set(out.len(), pair)?;
    }
    crate::web::array_iterator(&ctx, out)
}

fn params_keys<'js>(
    ctx: Ctx<'js>,
    this: This<Class<'js, UrlSearchParams<'js>>>,
) -> Result<Value<'js>> {
    let out = Array::new(ctx.clone())?;
    for (name, _) in &this.0.borrow().pairs {
        out.set(out.len(), name.clone())?;
    }
    crate::web::array_iterator(&ctx, out)
}

fn params_values<'js>(
    ctx: Ctx<'js>,
    this: This<Class<'js, UrlSearchParams<'js>>>,
) -> Result<Value<'js>> {
    let out = Array::new(ctx.clone())?;
    for (_, value) in &this.0.borrow().pairs {
        out.set(out.len(), value.clone())?;
    }
    crate::web::array_iterator(&ctx, out)
}

fn params_for_each<'js>(
    ctx: Ctx<'js>,
    this: This<Class<'js, UrlSearchParams<'js>>>,
    callback: Function<'js>,
    this_arg: Opt<Value<'js>>,
) -> Result<()> {
    let this_arg = this_arg
        .0
        .filter(|v| !v.is_undefined())
        .unwrap_or_else(|| Value::new_undefined(ctx));
    // The list is live: pairs appended by the callback are visited in the
    // same loop, and no borrow is held across the callback.
    let mut index = 0;
    loop {
        let (name, value) = {
            let params = this.0.borrow();
            if index >= params.pairs.len() {
                break;
            }
            params.pairs[index].clone()
        };
        callback.call::<_, ()>((This(this_arg.clone()), value, name, this.0.clone()))?;
        index += 1;
    }
    Ok(())
}

fn params_size<'js>(this: This<Class<'js, UrlSearchParams<'js>>>) -> usize {
    this.0.borrow().pairs.len()
}
