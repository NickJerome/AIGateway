//! `TextEncoder` / `TextDecoder`: UTF-8 only, streaming decode with a
//! pending tail and cross-chunk BOM detection.

use rquickjs::class::{JsClass, Trace, Tracer, Writable};
use rquickjs::function::{Opt, This};
use rquickjs::object::Accessor;
use rquickjs::{
    ArrayBuffer, Class, Coerced, Constructor, Ctx, Exception, FromJs, Function, JsLifetime, Object,
    Result, TypedArray, Value,
};
use std::string::String as StdString;

/// `TextEncoder`: UTF-8 only. `encodeInto` is not provided.
pub struct TextEncoder;

/// `TextDecoder`: UTF-8 only, with streaming state across `decode()` calls.
pub struct TextDecoder {
    fatal: bool,
    ignore_bom: bool,
    // Bytes carried over: an incomplete UTF-8 tail, or a BOM prefix held back
    // until the next chunk disambiguates it.
    pending: Vec<u8>,
    // BOM sniffing is armed at the start of each flush cycle.
    bom_done: bool,
    // Set by a `stream: true` call; the next non-streaming call flushes
    // without resetting, and the call after that starts a fresh cycle.
    do_not_flush: bool,
}

/// WTF-8 → UTF-8: QuickJS emits lone surrogates as three-byte WTF-8
/// sequences (0xED 0xA0..=0xBF xx); each becomes U+FFFD. Surrogate pairs are
/// already merged by the engine. Reading `String::to_string()` instead would
/// fail outright on a lone surrogate.
fn wtf8_to_utf8(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0xED
            && i + 2 < bytes.len()
            && (0xA0..=0xBF).contains(&bytes[i + 1])
            && (0x80..=0xBF).contains(&bytes[i + 2])
        {
            out.extend_from_slice(&[0xEF, 0xBF, 0xBD]); // U+FFFD
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    out
}

/// Bytes of an `ArrayBuffer` or `Uint8Array` argument. Other views
/// (DataView, non-u8 typed arrays) are not accepted.
fn input_bytes<'js>(ctx: &Ctx<'js>, input: Value<'js>) -> Result<Vec<u8>> {
    if input.is_undefined() || input.is_null() {
        return Ok(Vec::new());
    }
    let raw = if let Some(buffer) = ArrayBuffer::from_value(input.clone()) {
        buffer.as_raw().map(|p| unsafe { p.as_ref() }.to_vec())
    } else if let Ok(view) = TypedArray::<u8>::from_value(input) {
        view.as_raw().map(|p| unsafe { p.as_ref() }.to_vec())
    } else {
        None
    };
    raw.ok_or_else(|| {
        Exception::throw_type(
            ctx,
            "TextDecoder input must be an ArrayBuffer or Uint8Array",
        )
    })
}

/// Decode `data` as UTF-8 per the Encoding Standard: an invalid sequence
/// emits one U+FFFD (or fails in fatal mode) and consumes exactly the bytes
/// the standard prescribes; an incomplete tail is kept in `pending` when
/// streaming, otherwise it is an error like any other.
fn decode_utf8(
    ctx: &Ctx<'_>,
    decoder: &mut TextDecoder,
    bytes: &[u8],
    stream: bool,
) -> Result<StdString> {
    const BOM: &[u8] = &[0xEF, 0xBB, 0xBF];
    let mut data = std::mem::take(&mut decoder.pending);
    data.extend_from_slice(bytes);
    if !decoder.ignore_bom && !decoder.bom_done {
        if data.starts_with(BOM) {
            data.drain(..3);
            decoder.bom_done = true;
        } else if stream && data.len() < 3 && BOM.starts_with(&data) {
            // A cross-chunk BOM: hold the prefix until more bytes arrive.
            decoder.pending = data;
            return Ok(StdString::new());
        } else {
            decoder.bom_done = true;
        }
    }
    let mut out = StdString::new();
    let mut rest = data.as_slice();
    loop {
        match std::str::from_utf8(rest) {
            Ok(valid) => {
                out.push_str(valid);
                return Ok(out);
            }
            Err(err) => {
                out.push_str(
                    std::str::from_utf8(&rest[..err.valid_up_to()]).expect("valid prefix"),
                );
                rest = &rest[err.valid_up_to()..];
                match err.error_len() {
                    Some(consumed) => {
                        if decoder.fatal {
                            return Err(Exception::throw_type(
                                ctx,
                                "The encoded data was not valid for encoding utf-8",
                            ));
                        }
                        out.push('\u{FFFD}');
                        rest = &rest[consumed..];
                    }
                    None => {
                        // Incomplete sequence at the end of the input.
                        if stream {
                            decoder.pending = rest.to_vec();
                            return Ok(out);
                        }
                        if decoder.fatal {
                            return Err(Exception::throw_type(
                                ctx,
                                "The encoded data was not valid for encoding utf-8",
                            ));
                        }
                        out.push('\u{FFFD}');
                        return Ok(out);
                    }
                }
            }
        }
    }
}

fn read_flag(options: &Option<Value<'_>>, name: &str) -> Result<bool> {
    match options {
        Some(value) => match value.as_object() {
            Some(obj) => Ok(obj.get::<_, bool>(name).unwrap_or(false)),
            None => Ok(false),
        },
        None => Ok(false),
    }
}

impl<'js> Trace<'js> for TextEncoder {
    fn trace<'a>(&self, _tracer: Tracer<'a, 'js>) {}
}

unsafe impl<'js> JsLifetime<'js> for TextEncoder {
    type Changed<'to> = TextEncoder;
}

impl<'js> Trace<'js> for TextDecoder {
    fn trace<'a>(&self, _tracer: Tracer<'a, 'js>) {}
}

unsafe impl<'js> JsLifetime<'js> for TextDecoder {
    type Changed<'to> = TextDecoder;
}

impl<'js> JsClass<'js> for TextEncoder {
    const NAME: &'static str = "TextEncoder";

    type Mutable = Writable;

    fn prototype(ctx: &Ctx<'js>) -> Result<Option<Object<'js>>> {
        let proto = Object::new(ctx.clone())?;
        proto.prop("encoding", Accessor::from(encoder_encoding))?;
        proto.set("encode", Function::new(ctx.clone(), encoder_encode)?)?;
        Ok(Some(proto))
    }

    fn constructor(ctx: &Ctx<'js>) -> Result<Option<Constructor<'js>>> {
        let ctor = Constructor::new_class::<TextEncoder, _, _>(ctx.clone(), |ctx: Ctx<'js>| {
            Class::instance(ctx, TextEncoder)
        })?;
        Ok(Some(ctor))
    }
}

impl<'js> JsClass<'js> for TextDecoder {
    const NAME: &'static str = "TextDecoder";

    type Mutable = Writable;

    fn prototype(ctx: &Ctx<'js>) -> Result<Option<Object<'js>>> {
        let proto = Object::new(ctx.clone())?;
        proto.prop("encoding", Accessor::from(decoder_encoding))?;
        proto.prop("fatal", Accessor::from(decoder_fatal))?;
        proto.prop("ignoreBOM", Accessor::from(decoder_ignore_bom))?;
        proto.set("decode", Function::new(ctx.clone(), decoder_decode)?)?;
        Ok(Some(proto))
    }

    fn constructor(ctx: &Ctx<'js>) -> Result<Option<Constructor<'js>>> {
        let ctor = Constructor::new_class::<TextDecoder, _, _>(
            ctx.clone(),
            |ctx: Ctx<'js>,
             label: Opt<Coerced<StdString>>,
             options: Opt<Value<'js>>|
             -> Result<Class<'js, TextDecoder>> {
                let label = label.0.map(|l| l.0).unwrap_or_else(|| "utf-8".to_string());
                // Encoding Standard labels for UTF-8.
                match label.trim().to_ascii_lowercase().as_str() {
                    "unicode-1-1-utf-8" | "unicode11utf8" | "unicode20utf8" | "utf-8" | "utf8"
                    | "x-unicode20utf8" => {}
                    _ => {
                        return Err(Exception::throw_range(
                            &ctx,
                            &format!("The encoding label provided ('{label}') is invalid"),
                        ));
                    }
                }
                let options = options.0.filter(|v| !v.is_undefined() && !v.is_null());
                Class::instance(
                    ctx,
                    TextDecoder {
                        fatal: read_flag(&options, "fatal")?,
                        ignore_bom: read_flag(&options, "ignoreBOM")?,
                        pending: Vec::new(),
                        bom_done: false,
                        do_not_flush: false,
                    },
                )
            },
        )?;
        Ok(Some(ctor))
    }
}

fn encoder_encoding<'js>(_this: This<Class<'js, TextEncoder>>) -> &'static str {
    "utf-8"
}

fn encoder_encode<'js>(
    ctx: Ctx<'js>,
    _this: This<Class<'js, TextEncoder>>,
    input: Opt<Value<'js>>,
) -> Result<TypedArray<'js, u8>> {
    let bytes = match input.0 {
        Some(value) if !value.is_undefined() => {
            if value.is_string() {
                // Take the raw CString bytes (WTF-8): `String::to_string()`
                // would error on a lone surrogate instead of replacing it
                // with U+FFFD.
                let text = value.into_string().unwrap();
                let raw = text.to_cstring()?;
                let raw: &[u8] = raw.as_ref();
                wtf8_to_utf8(raw)
            } else {
                // Web IDL string coercion for non-strings (numbers, objects).
                // Coerced<> goes through JS ToString; such results are always
                // valid UTF-8 for the inputs we accept here.
                let Coerced(text) = Coerced::<StdString>::from_js(&ctx, value)?;
                text.into_bytes()
            }
        }
        _ => Vec::new(),
    };
    TypedArray::<u8>::new_copy(ctx, bytes)
}

fn decoder_encoding<'js>(_this: This<Class<'js, TextDecoder>>) -> &'static str {
    "utf-8"
}

fn decoder_fatal<'js>(this: This<Class<'js, TextDecoder>>) -> bool {
    this.0.borrow().fatal
}

fn decoder_ignore_bom<'js>(this: This<Class<'js, TextDecoder>>) -> bool {
    this.0.borrow().ignore_bom
}

fn decoder_decode<'js>(
    ctx: Ctx<'js>,
    this: This<Class<'js, TextDecoder>>,
    input: Opt<Value<'js>>,
    options: Opt<Value<'js>>,
) -> Result<StdString> {
    let bytes = match input.0 {
        Some(value) => input_bytes(&ctx, value)?,
        None => Vec::new(),
    };
    let options = options.0.filter(|v| !v.is_undefined() && !v.is_null());
    let stream = read_flag(&options, "stream")?;
    let decoder = &mut this.0.borrow_mut();
    // Encoding Standard, decode() step 1: once the previous call flushed (or
    // before the first call), the next call starts a fresh cycle — decoder
    // state and the BOM-seen flag reset, so BOM sniffing re-arms per flush
    // cycle.
    if !decoder.do_not_flush {
        decoder.pending.clear();
        decoder.bom_done = false;
    }
    decoder.do_not_flush = stream;
    decode_utf8(&ctx, decoder, &bytes, stream)
}
