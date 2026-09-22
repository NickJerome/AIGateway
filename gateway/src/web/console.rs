//! `console` (spec 01 line 36): log/info/warn/error/debug format their
//! arguments and copy owned text to the host log sink — the sink never holds
//! a request JS reference. The default sink is stderr; tests can capture
//! records. Persistence lands in 07.

use rquickjs::function::Rest;
use rquickjs::{Ctx, Function, Object, Result, Value};
use std::string::String as StdString;
use std::sync::Mutex;

/// One formatted console line: owned data only, safe to cross threads.
pub struct ConsoleRecord {
    pub level: StdString,
    pub message: StdString,
}

static CAPTURE: Mutex<Option<Vec<ConsoleRecord>>> = Mutex::new(None);

/// Test hook: start capturing formatted records (stderr stays active).
pub fn start_capture() {
    *CAPTURE.lock().unwrap() = Some(Vec::new());
}

/// Test hook: stop capturing.
pub fn stop_capture() {
    *CAPTURE.lock().unwrap() = None;
}

/// Test hook: take everything captured since `start_capture`.
pub fn drain_capture() -> Vec<ConsoleRecord> {
    CAPTURE
        .lock()
        .unwrap()
        .as_mut()
        .map(std::mem::take)
        .unwrap_or_default()
}

macro_rules! console_method {
    ($name:ident, $level:literal) => {
        fn $name<'js>(ctx: Ctx<'js>, args: Rest<Value<'js>>) {
            emit($level, format_args(&ctx, args));
        }
    };
}

console_method!(console_log, "log");
console_method!(console_info, "info");
console_method!(console_warn, "warn");
console_method!(console_error, "error");
console_method!(console_debug, "debug");

/// Install the `console` global.
pub fn install(ctx: &Ctx<'_>) -> Result<()> {
    let console = Object::new(ctx.clone())?;
    console.set("log", Function::new(ctx.clone(), console_log)?)?;
    console.set("info", Function::new(ctx.clone(), console_info)?)?;
    console.set("warn", Function::new(ctx.clone(), console_warn)?)?;
    console.set("error", Function::new(ctx.clone(), console_error)?)?;
    console.set("debug", Function::new(ctx.clone(), console_debug)?)?;
    ctx.globals().set("console", console)?;
    Ok(())
}

fn emit(level: &str, message: StdString) {
    // The record is fully owned: no request JS reference escapes the Engine.
    eprintln!("[{level}] {message}");
    if let Some(buf) = CAPTURE.lock().unwrap().as_mut() {
        buf.push(ConsoleRecord {
            level: level.into(),
            message,
        });
    }
}

fn format_args<'js>(ctx: &Ctx<'js>, args: Rest<Value<'js>>) -> StdString {
    args.0
        .iter()
        .map(|value| format_one(ctx, value))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Format one argument: strings verbatim, everything else JSON-stringified
/// (undefined for the non-serializable); cycles fall back to a marker.
fn format_one<'js>(ctx: &Ctx<'js>, value: &Value<'js>) -> StdString {
    if value.is_undefined() {
        return "undefined".into();
    }
    if value.is_null() {
        return "null".into();
    }
    if value.is_string() {
        return value
            .as_string()
            .and_then(|s| s.to_string().ok())
            .unwrap_or_default();
    }
    let stringify: Result<Function<'_>> = ctx
        .globals()
        .get::<_, Object<'_>>("JSON")
        .and_then(|json| json.get("stringify"));
    if let Ok(stringify) = stringify {
        match stringify.call::<_, Value>((value.clone(),)) {
            Ok(v) if v.is_string() => {
                return v
                    .as_string()
                    .and_then(|s| s.to_string().ok())
                    .unwrap_or_default();
            }
            Ok(v) if v.is_undefined() => return "undefined".into(),
            Ok(_) => {}
            Err(_) => {
                // Clear the pending exception (e.g. a cyclic structure).
                let _ = ctx.catch();
            }
        }
    }
    "[unserializable]".into()
}
