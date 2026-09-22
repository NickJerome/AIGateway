//! `Event` and `EventTarget`: listener lists live in Rust with `'js`
//! callbacks (traced, never `Persistent`). Dispatch snapshots listener ids:
//! `once` listeners are removed before their callback runs, listeners added
//! during dispatch are not called, and removed listeners are skipped.

use rquickjs::class::{JsClass, Trace, Tracer, Writable};
use rquickjs::function::{Opt, This};
use rquickjs::object::Accessor;
use rquickjs::{Class, Coerced, Constructor, Ctx, Function, JsLifetime, Object, Result, Value};
use std::string::String as StdString;

/// One registered listener. The callback stays a plain `'js` reference owned
/// by the class instance, so the runtime GC sees it through `Trace`.
pub struct Listener<'js> {
    pub id: u64,
    pub type_: StdString,
    pub callback: Function<'js>,
    pub once: bool,
}

impl<'js> Trace<'js> for Listener<'js> {
    fn trace<'a>(&self, tracer: Tracer<'a, 'js>) {
        self.callback.trace(tracer);
    }
}

/// Shared listener-list operations (EventTarget and AbortSignal).
#[derive(Default)]
pub struct Listeners<'js> {
    pub list: Vec<Listener<'js>>,
    pub next_id: u64,
}

impl<'js> Listeners<'js> {
    pub fn add(&mut self, type_: &str, callback: Function<'js>, once: bool) {
        // A duplicate (type, callback) pair is ignored, per the DOM Standard.
        if self
            .list
            .iter()
            .any(|l| l.type_ == type_ && l.callback == callback)
        {
            return;
        }
        self.next_id += 1;
        self.list.push(Listener {
            id: self.next_id,
            type_: type_.to_string(),
            callback,
            once,
        });
    }

    pub fn remove(&mut self, type_: &str, callback: &Function<'js>) {
        self.list
            .retain(|l| !(l.type_ == type_ && &l.callback == callback));
    }

    pub fn remove_id(&mut self, id: u64) {
        self.list.retain(|l| l.id != id);
    }

    pub fn has_id(&self, id: u64) -> bool {
        self.list.iter().any(|l| l.id == id)
    }

    /// Snapshot of the listeners for one dispatch, in registration order.
    pub fn snapshot(&self, type_: &str) -> Vec<Listener<'js>> {
        self.list
            .iter()
            .filter(|l| l.type_ == type_)
            .map(|l| Listener {
                id: l.id,
                type_: l.type_.clone(),
                callback: l.callback.clone(),
                once: l.once,
            })
            .collect()
    }
}

/// `Event`: type plus the init flags; `target`/`currentTarget` are set only
/// while a dispatch is in flight.
pub struct Event<'js> {
    type_: StdString,
    bubbles: bool,
    cancelable: bool,
    composed: bool,
    default_prevented: bool,
    stop_immediate: bool,
    target: Option<Value<'js>>,
}

impl<'js> Event<'js> {
    pub fn new(type_: &str) -> Self {
        Event {
            type_: type_.to_string(),
            bubbles: false,
            cancelable: false,
            composed: false,
            default_prevented: false,
            stop_immediate: false,
            target: None,
        }
    }

    fn read_flag(init: &Option<Object<'js>>, name: &str) -> bool {
        init.as_ref()
            .and_then(|o| o.get::<_, bool>(name).ok())
            .unwrap_or(false)
    }
}

/// `EventTarget`: a flat listener list; no prototype chain sharing with
/// AbortSignal (see the deviation list in the plan).
pub struct EventTarget<'js> {
    listeners: Listeners<'js>,
}

/// Whether `options` requests `once` (object `{once: true}` only; the legacy
/// boolean capture flag does not imply once).
fn read_once(options: &Option<Value<'_>>) -> bool {
    options
        .as_ref()
        .and_then(|v| v.as_object())
        .and_then(|o| o.get::<_, bool>("once").ok())
        .unwrap_or(false)
}

/// Helper so `dispatch_to` can borrow the listener list of either class.
pub trait ListenerOwner<'js>: JsClass<'js> {
    fn with_listeners<R>(class: &Class<'js, Self>, f: impl FnOnce(&Listeners<'js>) -> R) -> R
    where
        Self: Sized;
    fn with_listeners_mut<R>(
        class: &Class<'js, Self>,
        f: impl FnOnce(&mut Listeners<'js>) -> R,
    ) -> R
    where
        Self: Sized;
}

/// The shared dispatch loop.
pub fn dispatch_to<'js, C: ListenerOwner<'js> + JsClass<'js>>(
    owner: &Class<'js, C>,
    owner_value: Value<'js>,
    event: &Class<'js, Event<'js>>,
) -> Result<bool> {
    let type_ = event.borrow().type_.clone();
    let snapshot = C::with_listeners(owner, |l| l.snapshot(&type_));
    event.borrow_mut().target = Some(owner_value);
    for listener in snapshot {
        if !C::with_listeners(owner, |l| l.has_id(listener.id)) {
            continue;
        }
        if listener.once {
            // Remove before the callback runs: a re-entrant dispatch must not
            // call a once listener again.
            C::with_listeners_mut(owner, |l| l.remove_id(listener.id));
        }
        listener.callback.call::<_, ()>((event.clone(),))?;
        if event.borrow().stop_immediate {
            break;
        }
    }
    event.borrow_mut().target = None;
    Ok(!event.borrow().default_prevented)
}

impl<'js> Trace<'js> for Event<'js> {
    fn trace<'a>(&self, tracer: Tracer<'a, 'js>) {
        self.target.trace(tracer);
    }
}

unsafe impl<'js> JsLifetime<'js> for Event<'js> {
    type Changed<'to> = Event<'to>;
}

impl<'js> Trace<'js> for EventTarget<'js> {
    fn trace<'a>(&self, tracer: Tracer<'a, 'js>) {
        self.listeners.list.trace(tracer);
    }
}

unsafe impl<'js> JsLifetime<'js> for EventTarget<'js> {
    type Changed<'to> = EventTarget<'to>;
}

impl<'js> ListenerOwner<'js> for EventTarget<'js> {
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

impl<'js> JsClass<'js> for Event<'js> {
    const NAME: &'static str = "Event";

    type Mutable = Writable;

    fn prototype(ctx: &Ctx<'js>) -> Result<Option<Object<'js>>> {
        let proto = Object::new(ctx.clone())?;
        proto.prop("type", Accessor::from(event_type))?;
        proto.prop("bubbles", Accessor::from(event_bubbles))?;
        proto.prop("cancelable", Accessor::from(event_cancelable))?;
        proto.prop("composed", Accessor::from(event_composed))?;
        proto.prop("defaultPrevented", Accessor::from(event_default_prevented))?;
        proto.prop("target", Accessor::from(event_target))?;
        proto.prop("currentTarget", Accessor::from(event_target))?;
        proto.set(
            "preventDefault",
            Function::new(ctx.clone(), event_prevent_default)?,
        )?;
        proto.set(
            "stopImmediatePropagation",
            Function::new(ctx.clone(), event_stop_immediate)?,
        )?;
        Ok(Some(proto))
    }

    fn constructor(ctx: &Ctx<'js>) -> Result<Option<Constructor<'js>>> {
        let ctor = Constructor::new_class::<Event<'js>, _, _>(
            ctx.clone(),
            |ctx: Ctx<'js>,
             type_: Coerced<StdString>,
             init: Opt<Object<'js>>|
             -> Result<Class<'js, Event<'js>>> {
                let init = init.0;
                Class::instance(
                    ctx,
                    Event {
                        type_: type_.0,
                        bubbles: Event::read_flag(&init, "bubbles"),
                        cancelable: Event::read_flag(&init, "cancelable"),
                        composed: Event::read_flag(&init, "composed"),
                        default_prevented: false,
                        stop_immediate: false,
                        target: None,
                    },
                )
            },
        )?;
        Ok(Some(ctor))
    }
}

impl<'js> JsClass<'js> for EventTarget<'js> {
    const NAME: &'static str = "EventTarget";

    type Mutable = Writable;

    fn prototype(ctx: &Ctx<'js>) -> Result<Option<Object<'js>>> {
        let proto = Object::new(ctx.clone())?;
        proto.set(
            "addEventListener",
            Function::new(ctx.clone(), target_add_event_listener)?,
        )?;
        proto.set(
            "removeEventListener",
            Function::new(ctx.clone(), target_remove_event_listener)?,
        )?;
        proto.set(
            "dispatchEvent",
            Function::new(ctx.clone(), target_dispatch_event)?,
        )?;
        Ok(Some(proto))
    }

    fn constructor(ctx: &Ctx<'js>) -> Result<Option<Constructor<'js>>> {
        let ctor =
            Constructor::new_class::<EventTarget<'js>, _, _>(ctx.clone(), |ctx: Ctx<'js>| {
                Class::instance(
                    ctx,
                    EventTarget {
                        listeners: Listeners::default(),
                    },
                )
            })?;
        Ok(Some(ctor))
    }
}

// --- Event accessors ---

fn event_type<'js>(this: This<Class<'js, Event<'js>>>) -> StdString {
    this.0.borrow().type_.clone()
}

fn event_bubbles<'js>(this: This<Class<'js, Event<'js>>>) -> bool {
    this.0.borrow().bubbles
}

fn event_cancelable<'js>(this: This<Class<'js, Event<'js>>>) -> bool {
    this.0.borrow().cancelable
}

fn event_composed<'js>(this: This<Class<'js, Event<'js>>>) -> bool {
    this.0.borrow().composed
}

fn event_default_prevented<'js>(this: This<Class<'js, Event<'js>>>) -> bool {
    this.0.borrow().default_prevented
}

fn event_target<'js>(ctx: Ctx<'js>, this: This<Class<'js, Event<'js>>>) -> Result<Value<'js>> {
    match &this.0.borrow().target {
        Some(target) => Ok(target.clone()),
        None => Ok(Value::new_null(ctx)),
    }
}

fn event_prevent_default<'js>(this: This<Class<'js, Event<'js>>>) {
    let mut event = this.0.borrow_mut();
    if event.cancelable {
        event.default_prevented = true;
    }
}

fn event_stop_immediate<'js>(this: This<Class<'js, Event<'js>>>) {
    this.0.borrow_mut().stop_immediate = true;
}

// --- EventTarget methods ---

fn target_add_event_listener<'js>(
    this: This<Class<'js, EventTarget<'js>>>,
    type_: Coerced<StdString>,
    callback: Function<'js>,
    options: Opt<Value<'js>>,
) {
    let once = read_once(&options.0);
    this.0.borrow_mut().listeners.add(&type_.0, callback, once);
}

fn target_remove_event_listener<'js>(
    this: This<Class<'js, EventTarget<'js>>>,
    type_: Coerced<StdString>,
    callback: Function<'js>,
) {
    this.0.borrow_mut().listeners.remove(&type_.0, &callback);
}

fn target_dispatch_event<'js>(
    this: This<Class<'js, EventTarget<'js>>>,
    event: Class<'js, Event<'js>>,
) -> Result<bool> {
    let owner_value = this.0.clone().into_value();
    dispatch_to(&this.0, owner_value, &event)
}

/// Used by AbortController/AbortSignal to raise a plain Event instance.
pub fn event_instance<'js>(ctx: Ctx<'js>, type_: &str) -> Result<Class<'js, Event<'js>>> {
    Class::instance(ctx, Event::new(type_))
}
