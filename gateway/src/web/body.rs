//! One-shot host body buffers behind Request and Response (spec 01 line 41:
//! bodies stay host-side in this milestone; 02 turns them into streams and
//! never pre-materializes a large body as a JS array).

use crate::web::url::UrlSearchParams;
use rquickjs::class::{JsClass, Trace, Tracer, Writable};
use rquickjs::{
    Array, ArrayBuffer, Class, Constructor, Ctx, Exception, FromJs, Function, IntoJs, JsLifetime,
    Object, Promise, Result, TypedArray, U8Clamped, Value,
};
use std::cell::RefCell;
use std::rc::Rc;
use std::string::String as StdString;

/// Host-side body storage: a one-shot byte buffer. `used` flips when a read
/// method consumes a present body; cloning a Request/Response forks the
/// bytes before either side can consume them.
#[derive(Clone, Default)]
pub struct Body {
    inner: Rc<RefCell<BodyInner>>,
}

#[derive(Default)]
struct BodyInner {
    bytes: Option<Vec<u8>>,
    used: bool,
}

impl Body {
    pub fn none() -> Body {
        Body::default()
    }

    pub fn bytes(bytes: Vec<u8>) -> Body {
        Body {
            inner: Rc::new(RefCell::new(BodyInner {
                bytes: Some(bytes),
                used: false,
            })),
        }
    }

    /// Whether a body is present at all (an absent body never counts as used).
    pub fn is_none(&self) -> bool {
        self.inner.borrow().bytes.is_none()
    }

    pub fn used(&self) -> bool {
        self.inner.borrow().used
    }

    /// Deep copy for `clone()`: both halves start unconsumed.
    pub fn fork(&self) -> Body {
        let bytes = self.inner.borrow().bytes.clone();
        Body {
            inner: Rc::new(RefCell::new(BodyInner { bytes, used: false })),
        }
    }

    /// Consume the bytes. An absent body stays unused; a present body flips
    /// `used` exactly once.
    pub fn take(&self) -> Option<Vec<u8>> {
        let mut inner = self.inner.borrow_mut();
        if inner.bytes.is_some() {
            inner.used = true;
        }
        inner.bytes.take()
    }
}

/// The opaque JS handle returned by the `body` getter, standing in for a
/// ReadableStream until 02. It carries the host `Body` so a handle passed
/// back as BodyInit (frameworks do this, e.g. Hono's
/// `c.newResponse(res.body, res)`) keeps the body host-side instead of
/// materializing it as a JS array. The class has no prototype methods and is
/// never exposed as a global constructor, so plugin code cannot forge one.
pub struct BodyHandle {
    body: Body,
}

impl BodyHandle {
    pub fn of(body: &Body) -> BodyHandle {
        BodyHandle { body: body.clone() }
    }
}

impl<'js> Trace<'js> for BodyHandle {
    fn trace<'a>(&self, _tracer: Tracer<'a, 'js>) {}
}

// SAFETY: `BodyHandle` holds no JS references; nothing to trace.
unsafe impl<'js> JsLifetime<'js> for BodyHandle {
    type Changed<'to> = BodyHandle;
}

impl<'js> JsClass<'js> for BodyHandle {
    const NAME: &'static str = "BodyHandle";

    type Mutable = Writable;

    fn constructor(_ctx: &Ctx<'js>) -> Result<Option<Constructor<'js>>> {
        // Not constructible from JS; instances only come from `body` getters.
        Ok(None)
    }
}

/// The result of parsing a BodyInit value.
pub struct ParsedBody {
    pub body: Body,
    pub content_type: Option<StdString>,
}

/// BodyInit subset (spec 01 line 41): null, string, ArrayBuffer,
/// ArrayBufferView, URLSearchParams — plus the opaque host body handle
/// returned by `request.body`/`response.body`, which stands in for the
/// ReadableStream branch until 02. Blob/FormData are unsupported by the
/// project and are rejected.
pub fn parse_body_init<'js>(ctx: &Ctx<'js>, value: Value<'js>) -> Result<Option<ParsedBody>> {
    if value.is_null() || value.is_undefined() {
        return Ok(None);
    }
    if let Ok(handle) = Class::<BodyHandle>::from_js(ctx, value.clone()) {
        // Reusing a handle shares the one underlying body: it is still
        // consumed exactly once, by whichever side reads it first.
        let body = &handle.borrow().body;
        if body.used() {
            return Err(Exception::throw_type(
                ctx,
                "Cannot use a body that has already been consumed",
            ));
        }
        return Ok(Some(ParsedBody {
            body: body.clone(),
            content_type: None,
        }));
    }
    if value.is_string() {
        let text = value.as_string().expect("is_string").to_string()?;
        return Ok(Some(ParsedBody {
            body: Body::bytes(text.into_bytes()),
            content_type: Some("text/plain;charset=UTF-8".into()),
        }));
    }
    if let Ok(params) = Class::<UrlSearchParams>::from_js(ctx, value.clone()) {
        let query = params.borrow().serialized();
        return Ok(Some(ParsedBody {
            body: Body::bytes(query.into_bytes()),
            content_type: Some("application/x-www-form-urlencoded;charset=UTF-8".into()),
        }));
    }
    if value.as_object().is_some_and(|o| o.is_array_buffer()) {
        let Some(buffer) = ArrayBuffer::from_value(value) else {
            return Err(Exception::throw_type(ctx, "invalid ArrayBuffer"));
        };
        return Ok(Some(ParsedBody {
            body: Body::bytes(buffer_bytes(&buffer)),
            content_type: None,
        }));
    }
    if is_array_buffer_view(ctx, &value)? {
        return Ok(Some(ParsedBody {
            body: Body::bytes(view_bytes(&value)?),
            content_type: None,
        }));
    }
    Err(Exception::throw_type(
        ctx,
        "unsupported BodyInit type (Blob/FormData are not supported; ReadableStream bodies belong to milestone 02)",
    ))
}

/// Copy the full contents of an ArrayBuffer.
pub fn buffer_bytes(buffer: &ArrayBuffer<'_>) -> Vec<u8> {
    match buffer.as_raw() {
        // SAFETY: the buffer is alive while `buffer` is borrowed; we only read.
        Some(raw) => unsafe { raw.as_ref() }.to_vec(),
        None => Vec::new(),
    }
}

/// Capture native DataView accessors before plugin code can replace globals or
/// prototypes. The closure is rooted by JS, rather than a host Persistent.
pub(crate) fn install(ctx: &Ctx<'_>) -> Result<()> {
    ctx.eval::<(), _>(r#"
        (() => {
            const proto = DataView.prototype;
            const buffer = Object.getOwnPropertyDescriptor(proto, 'buffer').get;
            const offset = Object.getOwnPropertyDescriptor(proto, 'byteOffset').get;
            const length = Object.getOwnPropertyDescriptor(proto, 'byteLength').get;
            const apply = Reflect.apply;
            Object.defineProperty(globalThis, '__gatewayDataViewParts', {
                value: view => [apply(buffer, view, []), apply(offset, view, []), apply(length, view, [])]
            });
        })();
    "#)
}

/// Copy the viewed byte range of an ArrayBufferView (typed array or DataView).
pub fn view_bytes(view: &Value<'_>) -> Result<Vec<u8>> {
    macro_rules! copy_typed_array {
        ($ty:ty) => {
            if let Ok(array) = TypedArray::<$ty>::from_value(view.clone()) {
                let Some(raw) = array.as_raw() else {
                    return Err(Exception::throw_type(view.ctx(), "detached ArrayBuffer"));
                };
                // SAFETY: no JavaScript runs while the engine-owned bytes are copied.
                return Ok(unsafe { raw.as_ref() }.to_vec());
            }
        };
    }
    copy_typed_array!(u8);
    copy_typed_array!(U8Clamped);
    copy_typed_array!(i8);
    copy_typed_array!(u16);
    copy_typed_array!(i16);
    copy_typed_array!(u32);
    copy_typed_array!(i32);
    copy_typed_array!(f32);
    copy_typed_array!(f64);
    copy_typed_array!(i64);
    copy_typed_array!(u64);

    // Native getters read internal slots and perform the DataView brand check.
    let parts: Function = view.ctx().globals().get("__gatewayDataViewParts")?;
    let parts: Array = parts.call((view.clone(),))?;
    let buffer: ArrayBuffer = parts.get(0)?;
    let offset: usize = parts.get(1)?;
    let len: usize = parts.get(2)?;
    let raw = buffer_bytes(&buffer);
    let Some(end) = offset.checked_add(len) else {
        return Err(Exception::throw_range(
            view.ctx(),
            "ArrayBufferView range overflow",
        ));
    };
    let Some(bytes) = raw.get(offset..end) else {
        return Err(Exception::throw_range(
            view.ctx(),
            "ArrayBufferView is out of bounds",
        ));
    };
    Ok(bytes.to_vec())
}

/// `ArrayBuffer.isView(value)` via the engine builtin.
pub fn is_array_buffer_view<'js>(ctx: &Ctx<'js>, value: &Value<'js>) -> Result<bool> {
    let globals = ctx.globals();
    let array_buffer: Object = globals.get("ArrayBuffer")?;
    let is_view: Function = array_buffer.get("isView")?;
    is_view.call((value.clone(),))
}

/// Bytes from an ArrayBuffer or ArrayBufferView argument (crypto data input).
pub fn bytes_argument<'js>(ctx: &Ctx<'js>, value: Value<'js>) -> Result<Vec<u8>> {
    if value.as_object().is_some_and(|o| o.is_array_buffer()) {
        let Some(buffer) = ArrayBuffer::from_value(value) else {
            return Err(Exception::throw_type(ctx, "invalid ArrayBuffer"));
        };
        return Ok(buffer_bytes(&buffer));
    }
    if value.as_object().is_some() && is_array_buffer_view(ctx, &value)? {
        return view_bytes(&value);
    }
    Err(Exception::throw_type(
        ctx,
        "data must be an ArrayBuffer or an ArrayBufferView",
    ))
}

/// A resolved promise wrapping `value`.
pub fn resolved<'js, T: IntoJs<'js>>(ctx: &Ctx<'js>, value: T) -> Result<Promise<'js>> {
    let (promise, resolve, _) = Promise::new(ctx)?;
    resolve.call::<_, ()>((value,))?;
    Ok(promise)
}

/// A rejected promise carrying an Error-shaped object with the given name.
/// The engine has no DOMException, so named plain errors stand in for it.
pub fn rejected<'js>(ctx: &Ctx<'js>, name: &str, message: &str) -> Result<Promise<'js>> {
    let (promise, _, reject) = Promise::new(ctx)?;
    let error = crate::web::named_error(ctx, name, message)?;
    reject.call::<_, ()>((error,))?;
    Ok(promise)
}

/// `text()`: consume the body and decode it as UTF-8 (replacement on error).
pub fn read_text<'js>(ctx: &Ctx<'js>, body: &Body) -> Result<Promise<'js>> {
    if body.used() {
        return rejected(ctx, "TypeError", "Body has already been consumed");
    }
    let bytes = body.take().unwrap_or_default();
    resolved(ctx, StdString::from_utf8_lossy(&bytes).into_owned())
}

/// `arrayBuffer()`: consume the body and hand the bytes to JS.
pub fn read_array_buffer<'js>(ctx: &Ctx<'js>, body: &Body) -> Result<Promise<'js>> {
    if body.used() {
        return rejected(ctx, "TypeError", "Body has already been consumed");
    }
    let bytes = body.take().unwrap_or_default();
    let buffer = ArrayBuffer::new(ctx.clone(), bytes)?;
    resolved(ctx, buffer)
}

/// `json()`: consume the body and parse it; a parse failure rejects with the
/// original SyntaxError value.
pub fn read_json<'js>(ctx: &Ctx<'js>, body: &Body) -> Result<Promise<'js>> {
    if body.used() {
        return rejected(ctx, "TypeError", "Body has already been consumed");
    }
    let bytes = body.take().unwrap_or_default();
    let text = StdString::from_utf8_lossy(&bytes).into_owned();
    let (promise, resolve, reject) = Promise::new(ctx)?;
    let json: Object = ctx.globals().get("JSON")?;
    let parse: Function = json.get("parse")?;
    match parse.call::<_, Value>((text,)) {
        Ok(value) => resolve.call::<_, ()>((value,))?,
        Err(_) => {
            let caught = ctx.catch();
            reject.call::<_, ()>((caught,))?;
        }
    }
    Ok(promise)
}
