//! `Response`: status/headers plus a host body handle (spec 01 line 28).
//! The `body` getter is an opaque handle placeholder; 02 replaces it with a
//! ReadableStream. `Response.json/redirect/error` live on the constructor.

use crate::web::body::{self, Body, BodyHandle};
use crate::web::headers::Headers;
use rquickjs::class::{JsClass, Trace, Tracer, Writable};
use rquickjs::function::{Opt, This};
use rquickjs::object::Accessor;
use rquickjs::{
    Class, Coerced, Constructor, Ctx, Exception, Function, JsLifetime, Object, Promise, Result,
    Value,
};
use std::string::String as StdString;

/// Statuses that must not carry a body (fetch spec "null body status").
const NULL_BODY_STATUSES: [u16; 4] = [101, 204, 205, 304];

pub struct Response<'js> {
    status: u16,
    headers: Class<'js, Headers>,
    body: Body,
    url: StdString,
}

impl<'js> Response<'js> {
    fn from_parts(
        ctx: &Ctx<'js>,
        status: u16,
        headers: Headers,
        body: Body,
        url: StdString,
    ) -> Result<Class<'js, Response<'js>>> {
        let headers = Class::instance(ctx.clone(), headers)?;
        Class::instance(
            ctx.clone(),
            Response {
                status,
                headers,
                body,
                url,
            },
        )
    }

    /// Extract the response into owned data for the host response slot (the
    /// body is consumed; spec 01: the request task writes status, headers and
    /// body into the slot, never a JS value). Stub semantics until 02 streams:
    /// a body already consumed by text()/json()/arrayBuffer() extracts as
    /// empty bytes while status and headers still reach the slot.
    pub(crate) fn extract(&self) -> (u16, Vec<(StdString, StdString)>, Vec<u8>) {
        (
            self.status,
            self.headers.borrow().raw_list(),
            self.body.take().unwrap_or_default(),
        )
    }

    /// Consume the body and decode it as text (sync `invoke` adapter path).
    pub(crate) fn take_body_text(&self) -> StdString {
        let bytes = self.body.take().unwrap_or_default();
        StdString::from_utf8_lossy(&bytes).into_owned()
    }

    fn clone_response(&self, ctx: &Ctx<'js>) -> Result<Class<'js, Response<'js>>> {
        if self.body.used() {
            return Err(Exception::throw_type(
                ctx,
                "Cannot clone a Response whose body is already used",
            ));
        }
        Response::from_parts(
            ctx,
            self.status,
            Headers::from_list(self.headers.borrow().raw_list()),
            self.body.fork(),
            self.url.clone(),
        )
    }
}

impl<'js> Trace<'js> for Response<'js> {
    fn trace<'a>(&self, tracer: Tracer<'a, 'js>) {
        self.headers.trace(tracer);
    }
}

// SAFETY: every JS reference (the headers class) is traced above.
unsafe impl<'js> JsLifetime<'js> for Response<'js> {
    type Changed<'to> = Response<'to>;
}

impl<'js> JsClass<'js> for Response<'js> {
    const NAME: &'static str = "Response";

    type Mutable = Writable;

    fn prototype(ctx: &Ctx<'js>) -> Result<Option<Object<'js>>> {
        let proto = Object::new(ctx.clone())?;
        proto.prop("status", Accessor::from(response_status))?;
        proto.prop("ok", Accessor::from(response_ok))?;
        proto.prop("headers", Accessor::from(response_headers))?;
        proto.prop("url", Accessor::from(response_url))?;
        proto.prop("body", Accessor::from(response_body))?;
        proto.prop("bodyUsed", Accessor::from(response_body_used))?;
        proto.set("clone", Function::new(ctx.clone(), response_clone)?)?;
        proto.set("text", Function::new(ctx.clone(), response_text)?)?;
        proto.set("json", Function::new(ctx.clone(), response_json_method)?)?;
        proto.set(
            "arrayBuffer",
            Function::new(ctx.clone(), response_array_buffer)?,
        )?;
        Ok(Some(proto))
    }

    fn constructor(ctx: &Ctx<'js>) -> Result<Option<Constructor<'js>>> {
        let ctor = Constructor::new_class::<Response<'js>, _, _>(
            ctx.clone(),
            |ctx: Ctx<'js>,
             body_init: Opt<Value<'js>>,
             init: Opt<Object<'js>>|
             -> Result<Class<'js, Response<'js>>> {
                let mut status = 200u16;
                let mut headers = Headers::new();
                let mut url = StdString::new();
                if let Some(init) = init.0 {
                    if let Some(s) = init.get::<_, Option<u16>>("status")? {
                        status = s;
                    }
                    if let Some(u) = init.get::<_, Option<Coerced<StdString>>>("url")? {
                        url = u.0;
                    }
                    if let Some(h) = init.get::<_, Option<Value<'js>>>("headers")?
                        && !h.is_undefined()
                        && !h.is_null()
                    {
                        headers = Headers::from_init(&ctx, h)?;
                    }
                }
                if !(200..=599).contains(&status) {
                    return Err(Exception::throw_range(
                        &ctx,
                        "Response status must be between 200 and 599",
                    ));
                }
                let parsed = match body_init.0 {
                    Some(value) if !value.is_undefined() => body::parse_body_init(&ctx, value)?,
                    _ => None,
                };
                if parsed.is_some() && NULL_BODY_STATUSES.contains(&status) {
                    return Err(Exception::throw_type(
                        &ctx,
                        "Response with a null body status (101/204/205/304) cannot have a body",
                    ));
                }
                let body = match parsed {
                    Some(parsed) => {
                        if let Some(content_type) = parsed.content_type
                            && !headers.has("content-type")
                        {
                            headers.append_raw("content-type", &content_type);
                        }
                        parsed.body
                    }
                    None => Body::none(),
                };
                Response::from_parts(&ctx, status, headers, body, url)
            },
        )?;
        Ok(Some(ctor))
    }
}

/// `Response.json(data, init)`: JSON-serialize the data; a non-serializable
/// value (JSON.stringify returns undefined) throws a TypeError.
pub fn response_json<'js>(
    ctx: Ctx<'js>,
    data: Value<'js>,
    init: Opt<Object<'js>>,
) -> Result<Class<'js, Response<'js>>> {
    let json: Object = ctx.globals().get("JSON")?;
    let stringify: Function = json.get("stringify")?;
    let serialized = stringify.call::<_, Value>((data,))?;
    if !serialized.is_string() {
        return Err(Exception::throw_type(
            &ctx,
            "Response.json data is not JSON serializable",
        ));
    }
    let text = serialized
        .as_string()
        .expect("is_string was checked")
        .to_string()?;
    let mut status = 200u16;
    let mut headers = Headers::new();
    if let Some(init) = init.0 {
        if let Some(s) = init.get::<_, Option<u16>>("status")? {
            status = s;
        }
        if let Some(h) = init.get::<_, Option<Value<'js>>>("headers")?
            && !h.is_undefined()
            && !h.is_null()
        {
            headers = Headers::from_init(&ctx, h)?;
        }
    }
    if !(200..=599).contains(&status) {
        return Err(Exception::throw_range(
            &ctx,
            "Response status must be between 200 and 599",
        ));
    }
    if NULL_BODY_STATUSES.contains(&status) {
        return Err(Exception::throw_type(
            &ctx,
            "Response with a null body status (101/204/205/304) cannot have a body",
        ));
    }
    if !headers.has("content-type") {
        headers.append_raw("content-type", "application/json");
    }
    Response::from_parts(
        &ctx,
        status,
        headers,
        Body::bytes(text.into_bytes()),
        StdString::new(),
    )
}

/// `Response.redirect(url, status)`: 301/302/303/307/308 with a location
/// header and no body.
pub fn response_redirect<'js>(
    ctx: Ctx<'js>,
    url: Coerced<StdString>,
    status: Opt<u16>,
) -> Result<Class<'js, Response<'js>>> {
    let status = status.0.unwrap_or(302);
    if ![301, 302, 303, 307, 308].contains(&status) {
        return Err(Exception::throw_range(
            &ctx,
            "redirect status must be 301, 302, 303, 307 or 308",
        ));
    }
    let mut headers = Headers::new();
    headers.append_raw("location", &url.0);
    Response::from_parts(&ctx, status, headers, Body::none(), StdString::new())
}

/// `Response.error()`: status 0, no headers, no body; `ok` is false.
pub fn response_error<'js>(ctx: Ctx<'js>) -> Result<Class<'js, Response<'js>>> {
    Response::from_parts(&ctx, 0, Headers::new(), Body::none(), StdString::new())
}

fn response_status<'js>(this: This<Class<'js, Response<'js>>>) -> u16 {
    this.0.borrow().status
}

fn response_ok<'js>(this: This<Class<'js, Response<'js>>>) -> bool {
    (200..=299).contains(&this.0.borrow().status)
}

fn response_headers<'js>(this: This<Class<'js, Response<'js>>>) -> Class<'js, Headers> {
    this.0.borrow().headers.clone()
}

fn response_url<'js>(this: This<Class<'js, Response<'js>>>) -> StdString {
    this.0.borrow().url.clone()
}

/// `body`: null when absent, otherwise an opaque host body handle (02 turns
/// it into a ReadableStream; reading/locking are 02's).
fn response_body<'js>(ctx: Ctx<'js>, this: This<Class<'js, Response<'js>>>) -> Result<Value<'js>> {
    let borrow = this.0.borrow();
    if borrow.body.is_none() {
        return Ok(Value::new_null(ctx));
    }
    Ok(Class::instance(ctx.clone(), BodyHandle::of(&borrow.body))?.into_value())
}

fn response_body_used<'js>(this: This<Class<'js, Response<'js>>>) -> bool {
    this.0.borrow().body.used()
}

fn response_clone<'js>(
    ctx: Ctx<'js>,
    this: This<Class<'js, Response<'js>>>,
) -> Result<Class<'js, Response<'js>>> {
    this.0.borrow().clone_response(&ctx)
}

fn response_text<'js>(
    ctx: Ctx<'js>,
    this: This<Class<'js, Response<'js>>>,
) -> Result<Promise<'js>> {
    let body = this.0.borrow().body.clone();
    body::read_text(&ctx, &body)
}

fn response_json_method<'js>(
    ctx: Ctx<'js>,
    this: This<Class<'js, Response<'js>>>,
) -> Result<Promise<'js>> {
    let body = this.0.borrow().body.clone();
    body::read_json(&ctx, &body)
}

fn response_array_buffer<'js>(
    ctx: Ctx<'js>,
    this: This<Class<'js, Response<'js>>>,
) -> Result<Promise<'js>> {
    let body = this.0.borrow().body.clone();
    body::read_array_buffer(&ctx, &body)
}
