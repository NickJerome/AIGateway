//! `Request`: url/method/headers plus a host body handle (spec 01 line 28).
//! The `body` getter is an opaque handle placeholder; 02 replaces it with a
//! ReadableStream. Reading and locking the stream are 02's.

use crate::web::body::{self, Body, BodyHandle};
use crate::web::headers::Headers;
use rquickjs::class::{JsClass, Trace, Tracer, Writable};
use rquickjs::function::{Opt, This};
use rquickjs::object::Accessor;
use rquickjs::{
    Class, Coerced, Constructor, Ctx, Exception, FromJs, Function, JsLifetime, Object, Promise,
    Result, Value,
};
use std::string::String as StdString;

pub struct Request<'js> {
    url: StdString,
    method: StdString,
    headers: Class<'js, Headers>,
    body: Body,
}

impl<'js> Request<'js> {
    fn normalize_method(method: StdString) -> StdString {
        let upper = method.to_ascii_uppercase();
        if matches!(
            upper.as_str(),
            "DELETE" | "GET" | "HEAD" | "OPTIONS" | "POST" | "PUT"
        ) {
            upper
        } else {
            method
        }
    }
    /// The request handed to the plugin's fetch() entry: method and url from
    /// the invocation, no headers, no body (02 fills in real request data).
    pub(crate) fn incoming(
        ctx: &Ctx<'js>,
        url: StdString,
        method: StdString,
    ) -> Result<Class<'js, Request<'js>>> {
        let headers = Class::instance(ctx.clone(), Headers::new())?;
        Class::instance(
            ctx.clone(),
            Request {
                url,
                method,
                headers,
                body: Body::none(),
            },
        )
    }

    fn from_parts(
        ctx: &Ctx<'js>,
        url: StdString,
        method: StdString,
        headers: Headers,
        body: Body,
    ) -> Result<Class<'js, Request<'js>>> {
        let method = Self::normalize_method(method);
        if !body.is_none() && matches!(method.as_str(), "GET" | "HEAD") {
            return Err(Exception::throw_type(
                ctx,
                "Request with GET/HEAD method cannot have a body",
            ));
        }
        let headers = Class::instance(ctx.clone(), headers)?;
        Class::instance(
            ctx.clone(),
            Request {
                url,
                method,
                headers,
                body,
            },
        )
    }

    fn clone_request(&self, ctx: &Ctx<'js>) -> Result<Class<'js, Request<'js>>> {
        if self.body.used() {
            return Err(Exception::throw_type(
                ctx,
                "Cannot clone a Request whose body is already used",
            ));
        }
        Request::from_parts(
            ctx,
            self.url.clone(),
            self.method.clone(),
            Headers::from_list(self.headers.borrow().raw_list()),
            self.body.fork(),
        )
    }
}

impl<'js> Trace<'js> for Request<'js> {
    fn trace<'a>(&self, tracer: Tracer<'a, 'js>) {
        self.headers.trace(tracer);
    }
}

// SAFETY: every JS reference (the headers class) is traced above.
unsafe impl<'js> JsLifetime<'js> for Request<'js> {
    type Changed<'to> = Request<'to>;
}

impl<'js> JsClass<'js> for Request<'js> {
    const NAME: &'static str = "Request";

    type Mutable = Writable;

    fn prototype(ctx: &Ctx<'js>) -> Result<Option<Object<'js>>> {
        let proto = Object::new(ctx.clone())?;
        proto.prop("url", Accessor::from(request_url))?;
        proto.prop("method", Accessor::from(request_method))?;
        proto.prop("headers", Accessor::from(request_headers))?;
        proto.prop("body", Accessor::from(request_body))?;
        proto.prop("bodyUsed", Accessor::from(request_body_used))?;
        proto.set("clone", Function::new(ctx.clone(), request_clone)?)?;
        proto.set("text", Function::new(ctx.clone(), request_text)?)?;
        proto.set("json", Function::new(ctx.clone(), request_json)?)?;
        proto.set(
            "arrayBuffer",
            Function::new(ctx.clone(), request_array_buffer)?,
        )?;
        Ok(Some(proto))
    }

    fn constructor(ctx: &Ctx<'js>) -> Result<Option<Constructor<'js>>> {
        let ctor = Constructor::new_class::<Request<'js>, _, _>(
            ctx.clone(),
            |ctx: Ctx<'js>,
             input: Value<'js>,
             init: Opt<Object<'js>>|
             -> Result<Class<'js, Request<'js>>> {
                // First argument: a URL string or another Request (cloned).
                let (url, mut method, mut headers, mut body) =
                    match Class::<Request>::from_js(&ctx, input.clone()) {
                        Ok(other) => {
                            let other = other.borrow();
                            (
                                other.url.clone(),
                                other.method.clone(),
                                Headers::from_list(other.headers.borrow().raw_list()),
                                other.body.clone(),
                            )
                        }
                        Err(_) if input.is_string() => (
                            input
                                .as_string()
                                .expect("is_string was checked")
                                .to_string()?,
                            "GET".into(),
                            Headers::new(),
                            Body::none(),
                        ),
                        Err(_) => {
                            return Err(Exception::throw_type(
                                &ctx,
                                "Request input must be a URL string or a Request",
                            ));
                        }
                    };
                if let Some(init) = init.0 {
                    if let Some(m) = init.get::<_, Option<Coerced<StdString>>>("method")? {
                        method = m.0;
                    }
                    if let Some(h) = init.get::<_, Option<Value<'js>>>("headers")?
                        && !h.is_undefined()
                        && !h.is_null()
                    {
                        headers = Headers::from_init(&ctx, h)?;
                    }
                    if let Some(b) = init.get::<_, Option<Value<'js>>>("body")?
                        && let Some(parsed) = body::parse_body_init(&ctx, b)?
                    {
                        body = parsed.body;
                        if let Some(content_type) = parsed.content_type
                            && !headers.has("content-type")
                        {
                            headers.append_raw("content-type", &content_type);
                        }
                    }
                }
                if body.used() {
                    return Err(Exception::throw_type(
                        &ctx,
                        "Cannot construct from a Request whose body is already used",
                    ));
                }
                Request::from_parts(&ctx, url, method, headers, body.fork())
            },
        )?;
        Ok(Some(ctor))
    }
}

fn request_url<'js>(this: This<Class<'js, Request<'js>>>) -> StdString {
    this.0.borrow().url.clone()
}

fn request_method<'js>(this: This<Class<'js, Request<'js>>>) -> StdString {
    this.0.borrow().method.clone()
}

fn request_headers<'js>(this: This<Class<'js, Request<'js>>>) -> Class<'js, Headers> {
    this.0.borrow().headers.clone()
}

/// `body`: null when absent, otherwise an opaque host body handle (02 turns
/// it into a ReadableStream; reading/locking are 02's).
fn request_body<'js>(ctx: Ctx<'js>, this: This<Class<'js, Request<'js>>>) -> Result<Value<'js>> {
    let borrow = this.0.borrow();
    if borrow.body.is_none() {
        return Ok(Value::new_null(ctx));
    }
    Ok(Class::instance(ctx.clone(), BodyHandle::of(&borrow.body))?.into_value())
}

fn request_body_used<'js>(this: This<Class<'js, Request<'js>>>) -> bool {
    this.0.borrow().body.used()
}

fn request_clone<'js>(
    ctx: Ctx<'js>,
    this: This<Class<'js, Request<'js>>>,
) -> Result<Class<'js, Request<'js>>> {
    this.0.borrow().clone_request(&ctx)
}

fn request_text<'js>(ctx: Ctx<'js>, this: This<Class<'js, Request<'js>>>) -> Result<Promise<'js>> {
    let body = this.0.borrow().body.clone();
    body::read_text(&ctx, &body)
}

fn request_json<'js>(ctx: Ctx<'js>, this: This<Class<'js, Request<'js>>>) -> Result<Promise<'js>> {
    let body = this.0.borrow().body.clone();
    body::read_json(&ctx, &body)
}

fn request_array_buffer<'js>(
    ctx: Ctx<'js>,
    this: This<Class<'js, Request<'js>>>,
) -> Result<Promise<'js>> {
    let body = this.0.borrow().body.clone();
    body::read_array_buffer(&ctx, &body)
}
