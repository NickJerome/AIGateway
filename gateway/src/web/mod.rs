//! Host-side foundation Web API objects (spec 01 "基础 API": Headers, URL,
//! URLSearchParams, TextEncoder, TextDecoder, Event, EventTarget,
//! AbortController, AbortSignal).
//!
//! All classes are hand-written `JsClass` implementations; the `macro`
//! feature stays off. Class instances hold only `'js` references and trace
//! them — a `Persistent` inside a class would form a GC root cycle that
//! trips an assertion in `JS_FreeRuntime`.

pub mod abort;
pub mod body;
pub mod clone;
pub mod console;
pub mod crypto;
pub mod encoding;
pub mod event;
pub mod headers;
pub mod request;
pub mod response;
pub mod text;
pub mod url;

use rquickjs::function::This;
use rquickjs::{Array, Class, Ctx, Function, Object, Result, Symbol, Value};

/// Turn a snapshot Array into a genuine Array Iterator so that
/// entries()/keys()/values() satisfy the iterable protocol (`for..of` calls
/// `next()`, which a plain array lacks). Iteration stays a snapshot: later
/// mutations do not leak into an in-flight iteration.
pub(crate) fn array_iterator<'js>(ctx: &Ctx<'js>, array: Array<'js>) -> Result<Value<'js>> {
    let iter_fn: Function = array
        .as_object()
        .get(Symbol::iterator(ctx.clone()).as_atom())?;
    iter_fn.call((This(array),))
}

/// A real Error instance with a custom name. DOMException is outside the
/// supported subset, but promise errors must still satisfy `instanceof Error`.
pub(crate) fn named_error<'js>(ctx: &Ctx<'js>, name: &str, message: &str) -> Result<Object<'js>> {
    let error = rquickjs::Exception::from_message(ctx.clone(), message)?.into_object();
    error.set("name", name)?;
    Ok(error)
}

/// Install every foundation class onto the context globals. Called once per
/// request Context from `Engine::new`, so both entry paths (the sync `invoke`
/// adapter and the async request task) share this single install point.
pub fn install(ctx: &Ctx<'_>) -> Result<()> {
    body::install(ctx)?;
    ctx.eval::<(), _>(include_str!("vendor/streams.js"))?;
    ctx.eval::<(), _>(include_str!("streams.js"))?;
    let globals = ctx.globals();
    Class::<headers::Headers>::define(&globals)?;
    Class::<url::Url>::define(&globals)?;
    Class::<url::UrlSearchParams>::define(&globals)?;
    Class::<text::TextEncoder>::define(&globals)?;
    Class::<text::TextDecoder>::define(&globals)?;
    Class::<event::Event>::define(&globals)?;
    Class::<event::EventTarget>::define(&globals)?;
    Class::<abort::AbortController>::define(&globals)?;
    Class::<abort::AbortSignal>::define(&globals)?;
    Class::<request::Request>::define(&globals)?;
    Class::<response::Response>::define(&globals)?;
    // URL statics live on the constructor, not the prototype.
    let url_ctor: Object = globals.get("URL")?;
    url_ctor.set("canParse", Function::new(ctx.clone(), url::url_can_parse)?)?;
    url_ctor.set("parse", Function::new(ctx.clone(), url::url_parse)?)?;
    // Response statics live on the constructor, not the prototype.
    let response_ctor: Object = globals.get("Response")?;
    response_ctor.set("json", Function::new(ctx.clone(), response::response_json)?)?;
    response_ctor.set(
        "redirect",
        Function::new(ctx.clone(), response::response_redirect)?,
    )?;
    response_ctor.set(
        "error",
        Function::new(ctx.clone(), response::response_error)?,
    )?;
    clone::install(ctx)?;
    encoding::install(ctx)?;
    crypto::install(ctx)?;
    console::install(ctx)?;
    Ok(())
}
