//! `AbortController` / `AbortSignal`. The signal carries its own listener
//! list (it does not inherit the EventTarget prototype chain — see the plan's
//! deviation list). `abort()` is idempotent: a second call is a no-op.

use rquickjs::class::{JsClass, Trace, Tracer, Writable};
use rquickjs::function::{Opt, This};
use rquickjs::object::Accessor;
use rquickjs::{Class, Coerced, Constructor, Ctx, Function, JsLifetime, Object, Result, Value};
use std::string::String as StdString;

use super::event::{Event, Listeners, dispatch_to, event_instance};

/// The default abort reason: an Error whose `name` is "AbortError". The
/// engine has no DOMException, so a named Error stands in (deviation).
fn default_abort_reason<'js>(ctx: &Ctx<'js>) -> Result<Value<'js>> {
    let err: Object = ctx.eval("new Error('The operation was aborted')")?;
    err.set("name", "AbortError")?;
    Ok(err.into_value())
}

/// `AbortSignal`: aborted flag, reason, and an abort listener list.
/// `new AbortSignal()` is constructible here (browsers throw) — deviation.
pub struct AbortSignal<'js> {
    aborted: bool,
    reason: Option<Value<'js>>,
    listeners: Listeners<'js>,
}

/// `AbortController`: holds its signal as a traced `'js` class reference.
pub struct AbortController<'js> {
    signal: Class<'js, AbortSignal<'js>>,
}

impl<'js> Trace<'js> for AbortSignal<'js> {
    fn trace<'a>(&self, tracer: Tracer<'a, 'js>) {
        self.reason.trace(tracer);
        self.listeners.list.trace(tracer);
    }
}

unsafe impl<'js> JsLifetime<'js> for AbortSignal<'js> {
    type Changed<'to> = AbortSignal<'to>;
}

impl<'js> Trace<'js> for AbortController<'js> {
    fn trace<'a>(&self, tracer: Tracer<'a, 'js>) {
        self.signal.trace(tracer);
    }
}

unsafe impl<'js> JsLifetime<'js> for AbortController<'js> {
    type Changed<'to> = AbortController<'to>;
}

impl<'js> super::event::ListenerOwner<'js> for AbortSignal<'js> {
    fn with_listeners<R>(class: &Class<'js, Self>, f: impl FnOnce(&Listeners<'js>) -> R) -> R {
        f(&class.borrow().listeners)
    }
    fn with_listeners_mut<R>(
        class: &Class<'js, Self>,
        f: impl FnOnce(&mut Listeners<'js>) -> R,
    ) -> R {
        f(&mut class.borrow_mut().listeners)
    }
}

impl<'js> JsClass<'js> for AbortSignal<'js> {
    const NAME: &'static str = "AbortSignal";

    type Mutable = Writable;

    fn prototype(ctx: &Ctx<'js>) -> Result<Option<Object<'js>>> {
        let proto = Object::new(ctx.clone())?;
        proto.prop("aborted", Accessor::from(signal_aborted))?;
        proto.prop("reason", Accessor::from(signal_reason))?;
        proto.set(
            "addEventListener",
            Function::new(ctx.clone(), signal_add_event_listener)?,
        )?;
        proto.set(
            "removeEventListener",
            Function::new(ctx.clone(), signal_remove_event_listener)?,
        )?;
        Ok(Some(proto))
    }

    fn constructor(ctx: &Ctx<'js>) -> Result<Option<Constructor<'js>>> {
        let ctor =
            Constructor::new_class::<AbortSignal<'js>, _, _>(ctx.clone(), |ctx: Ctx<'js>| {
                Class::instance(
                    ctx,
                    AbortSignal {
                        aborted: false,
                        reason: None,
                        listeners: Listeners::default(),
                    },
                )
            })?;
        Ok(Some(ctor))
    }
}

impl<'js> JsClass<'js> for AbortController<'js> {
    const NAME: &'static str = "AbortController";

    type Mutable = Writable;

    fn prototype(ctx: &Ctx<'js>) -> Result<Option<Object<'js>>> {
        let proto = Object::new(ctx.clone())?;
        proto.prop("signal", Accessor::from(controller_signal))?;
        proto.set("abort", Function::new(ctx.clone(), controller_abort)?)?;
        Ok(Some(proto))
    }

    fn constructor(ctx: &Ctx<'js>) -> Result<Option<Constructor<'js>>> {
        let ctor = Constructor::new_class::<AbortController<'js>, _, _>(
            ctx.clone(),
            |ctx: Ctx<'js>| -> Result<Class<'js, AbortController<'js>>> {
                let signal = Class::instance(
                    ctx.clone(),
                    AbortSignal {
                        aborted: false,
                        reason: None,
                        listeners: Listeners::default(),
                    },
                )?;
                Class::instance(ctx, AbortController { signal })
            },
        )?;
        Ok(Some(ctor))
    }
}

// --- AbortSignal accessors and methods ---

fn signal_aborted<'js>(this: This<Class<'js, AbortSignal<'js>>>) -> bool {
    this.0.borrow().aborted
}

fn signal_reason<'js>(
    ctx: Ctx<'js>,
    this: This<Class<'js, AbortSignal<'js>>>,
) -> Result<Value<'js>> {
    match &this.0.borrow().reason {
        Some(reason) => Ok(reason.clone()),
        None => Ok(Value::new_undefined(ctx)),
    }
}

fn signal_add_event_listener<'js>(
    this: This<Class<'js, AbortSignal<'js>>>,
    type_: Coerced<StdString>,
    callback: Function<'js>,
    options: Opt<Value<'js>>,
) {
    let once = options
        .0
        .as_ref()
        .and_then(|v| v.as_object())
        .and_then(|o| o.get::<_, bool>("once").ok())
        .unwrap_or(false);
    this.0.borrow_mut().listeners.add(&type_.0, callback, once);
}

fn signal_remove_event_listener<'js>(
    this: This<Class<'js, AbortSignal<'js>>>,
    type_: Coerced<StdString>,
    callback: Function<'js>,
) {
    this.0.borrow_mut().listeners.remove(&type_.0, &callback);
}

// --- AbortController accessors and methods ---

fn controller_signal<'js>(
    this: This<Class<'js, AbortController<'js>>>,
) -> Class<'js, AbortSignal<'js>> {
    this.0.borrow().signal.clone()
}

fn controller_abort<'js>(
    ctx: Ctx<'js>,
    this: This<Class<'js, AbortController<'js>>>,
    reason: Opt<Value<'js>>,
) -> Result<()> {
    let signal = this.0.borrow().signal.clone();
    {
        let mut sig = signal.borrow_mut();
        if sig.aborted {
            // Repeated abort is a no-op: listeners fire exactly once.
            return Ok(());
        }
        sig.aborted = true;
        sig.reason = Some(match reason.0 {
            Some(value) if !value.is_undefined() => value,
            _ => default_abort_reason(&ctx)?,
        });
    }
    // The state write is done and the borrow released before any JS callback.
    let event: Class<'js, Event<'js>> = event_instance(ctx, "abort")?;
    let signal_value = signal.clone().into_value();
    dispatch_to(&signal, signal_value, &event).map(|_| ())
}
