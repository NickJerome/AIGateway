//! `crypto` (spec 01 line 35): getRandomValues, randomUUID, subtle.digest
//! (SHA-256/384/512), and HMAC sign/verify gated behind a raw-only
//! `subtle.importKey` (spec 01 line 39). CryptoKey is not constructible from
//! JS and never survives its request (every request owns a fresh Engine).
//! Randomness comes from `libc::getrandom`; no other new dependency beyond
//! `hmac`.

use crate::web::body::{bytes_argument, rejected, resolved};
use hmac::{EagerHash, Hmac, KeyInit, Mac};
use rquickjs::class::{JsClass, Trace, Tracer, Writable};
use rquickjs::function::Opt;
use rquickjs::{
    Array, Class, Coerced, Constructor, Ctx, Exception, Function, JsLifetime, Object, Promise,
    Result, TypedArray, U8Clamped, Value,
};
use sha2::{Digest, Sha256, Sha384, Sha512};
use std::string::String as StdString;

/// Maximum byte length for getRandomValues (spec: 64 KiB quota).
const MAX_RANDOM_BYTES: usize = 65536;

/// The digest algorithms open to plugins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HashAlg {
    Sha256,
    Sha384,
    Sha512,
}

impl HashAlg {
    fn from_name(name: &str) -> Option<HashAlg> {
        match name.to_ascii_uppercase().as_str() {
            "SHA-256" => Some(HashAlg::Sha256),
            "SHA-384" => Some(HashAlg::Sha384),
            "SHA-512" => Some(HashAlg::Sha512),
            _ => None,
        }
    }

    fn digest(&self, data: &[u8]) -> Vec<u8> {
        match self {
            HashAlg::Sha256 => Sha256::digest(data).to_vec(),
            HashAlg::Sha384 => Sha384::digest(data).to_vec(),
            HashAlg::Sha512 => Sha512::digest(data).to_vec(),
        }
    }

    fn hmac(&self, key: &[u8], data: &[u8]) -> Vec<u8> {
        match self {
            HashAlg::Sha256 => hmac_compute::<Sha256>(key, data),
            HashAlg::Sha384 => hmac_compute::<Sha384>(key, data),
            HashAlg::Sha512 => hmac_compute::<Sha512>(key, data),
        }
    }
}

fn hmac_compute<D: EagerHash>(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = <Hmac<D> as KeyInit>::new_from_slice(key).expect("HMAC accepts any key length");
    Mac::update(&mut mac, data);
    Mac::finalize(mac).into_bytes().to_vec()
}

/// Fill `buf` from the kernel CSPRNG.
fn fill_random(ctx: &Ctx<'_>, buf: &mut [u8]) -> Result<()> {
    let mut filled = 0;
    while filled < buf.len() {
        // SAFETY: `buf` is a live, writable slice; getrandom writes at most
        // the given length into it.
        let n = unsafe {
            libc::getrandom(
                buf[filled..].as_mut_ptr().cast::<libc::c_void>(),
                buf.len() - filled,
                0,
            )
        };
        if n < 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(Exception::throw_message(
                ctx,
                &format!("getrandom failed: {err}"),
            ));
        }
        filled += n as usize;
    }
    Ok(())
}

/// A raw-imported HMAC secret key. Never constructible from JS directly.
pub struct CryptoKey {
    hash: HashAlg,
    key: Vec<u8>,
    usages: Vec<StdString>,
}

impl<'js> Trace<'js> for CryptoKey {
    fn trace<'a>(&self, _tracer: Tracer<'a, 'js>) {}
}

// CryptoKey holds no JS references; the lifetime change is trivially identity.
unsafe impl<'js> JsLifetime<'js> for CryptoKey {
    type Changed<'to> = CryptoKey;
}

impl<'js> JsClass<'js> for CryptoKey {
    const NAME: &'static str = "CryptoKey";

    type Mutable = Writable;

    fn prototype(_ctx: &Ctx<'js>) -> Result<Option<Object<'js>>> {
        // No methods: sign/verify live on crypto.subtle and take the key.
        Ok(Some(Object::new(_ctx.clone())?))
    }

    fn constructor(ctx: &Ctx<'js>) -> Result<Option<Constructor<'js>>> {
        let ctor = Constructor::new_class::<CryptoKey, _, _>(
            ctx.clone(),
            |ctx: Ctx<'js>| -> Result<Class<'js, CryptoKey>> {
                Err(Exception::throw_type(
                    &ctx,
                    "CryptoKey is not constructible; use crypto.subtle.importKey",
                ))
            },
        )?;
        Ok(Some(ctor))
    }
}

/// `crypto.getRandomValues(typedArray)`: integer typed arrays only, at most
/// 64 KiB; fills in place and returns the same array.
fn get_random_values<'js>(ctx: Ctx<'js>, array: Value<'js>) -> Result<Value<'js>> {
    let Some(obj) = array.as_object() else {
        return Err(Exception::throw_type(
            &ctx,
            "getRandomValues expects an integer typed array",
        ));
    };
    let globals = ctx.globals();
    let mut integer = false;
    for ctor in [
        "Uint8Array",
        "Uint8ClampedArray",
        "Int8Array",
        "Uint16Array",
        "Int16Array",
        "Uint32Array",
        "Int32Array",
    ] {
        let ctor: Object = globals.get(ctor)?;
        if obj.is_instance_of(&ctor) {
            integer = true;
            break;
        }
    }
    if !integer {
        return Err(Exception::throw_type(
            &ctx,
            "getRandomValues expects an integer typed array",
        ));
    }
    let mut raw = None;
    macro_rules! try_integer_view {
        ($ty:ty) => {
            if raw.is_none() {
                if let Ok(view) = TypedArray::<$ty>::from_value(array.clone()) {
                    raw = view.as_raw();
                }
            }
        };
    }
    try_integer_view!(u8);
    try_integer_view!(U8Clamped);
    try_integer_view!(i8);
    try_integer_view!(u16);
    try_integer_view!(i16);
    try_integer_view!(u32);
    try_integer_view!(i32);
    let Some(mut raw) = raw else {
        return Err(Exception::throw_type(
            &ctx,
            "getRandomValues expects an integer typed array",
        ));
    };
    let byte_length = raw.len();
    if byte_length > MAX_RANDOM_BYTES {
        let error = crate::web::named_error(
            &ctx,
            "QuotaExceededError",
            "getRandomValues array exceeds 65536 bytes",
        )?;
        return Err(ctx.throw(error.into_value()));
    }
    // SAFETY: no JavaScript runs while this engine-owned view is borrowed.
    fill_random(&ctx, unsafe { raw.as_mut() })?;
    Ok(array)
}

/// `crypto.randomUUID()`: RFC 4122 version 4.
fn random_uuid(ctx: Ctx<'_>) -> Result<StdString> {
    let mut bytes = [0u8; 16];
    fill_random(&ctx, &mut bytes)?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: StdString = bytes.iter().map(|b| format!("{b:02x}")).collect();
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    ))
}

/// Read an algorithm argument that is either a string or `{ name: ... }`.
fn algorithm_name<'js>(value: &Value<'js>) -> Option<StdString> {
    if value.is_string() {
        return value.as_string()?.to_string().ok();
    }
    let obj = value.as_object()?;
    let name: Coerced<StdString> = obj.get("name").ok()?;
    Some(name.0)
}

/// `crypto.subtle.digest(algorithm, data)` -> Promise<ArrayBuffer>.
fn subtle_digest<'js>(
    ctx: Ctx<'js>,
    algorithm: Value<'js>,
    data: Value<'js>,
) -> Result<Promise<'js>> {
    let data = bytes_argument(&ctx, data)?;
    let Some(name) = algorithm_name(&algorithm) else {
        return Err(Exception::throw_type(
            &ctx,
            "algorithm must be a string or { name }",
        ));
    };
    let Some(hash) = HashAlg::from_name(&name) else {
        return rejected(
            &ctx,
            "NotSupportedError",
            "only SHA-256, SHA-384 and SHA-512 digests are supported",
        );
    };
    let buffer = rquickjs::ArrayBuffer::new(ctx.clone(), hash.digest(&data))?;
    resolved(&ctx, buffer)
}

/// `crypto.subtle.importKey('raw', keyData, { name: 'HMAC', hash }, extractable, usages)`
/// -> Promise<CryptoKey>. Only raw HMAC keys; RSA/ECDSA/JWK/encryption stay
/// out (main spec line 517).
fn subtle_import_key<'js>(
    ctx: Ctx<'js>,
    format: Coerced<StdString>,
    key_data: Value<'js>,
    algorithm: Value<'js>,
    _extractable: Opt<bool>,
    usages: Opt<Array<'js>>,
) -> Result<Promise<'js>> {
    if format.0 != "raw" {
        return rejected(
            &ctx,
            "NotSupportedError",
            "only 'raw' HMAC key import is supported",
        );
    }
    let mut key_data = bytes_argument(&ctx, key_data)?;
    let Some(algorithm_obj) = algorithm.as_object() else {
        return Err(Exception::throw_type(
            &ctx,
            "HMAC import algorithm must be { name: 'HMAC', hash }",
        ));
    };
    let name: Option<Coerced<StdString>> = algorithm_obj.get("name")?;
    if !name.is_some_and(|n| n.0.eq_ignore_ascii_case("HMAC")) {
        return rejected(&ctx, "NotSupportedError", "only HMAC keys can be imported");
    }
    let hash_value: Value = algorithm_obj.get("hash")?;
    let Some(hash_name) = algorithm_name(&hash_value) else {
        return Err(Exception::throw_type(
            &ctx,
            "HMAC hash must be a string or { name }",
        ));
    };
    let Some(hash) = HashAlg::from_name(&hash_name) else {
        return rejected(
            &ctx,
            "NotSupportedError",
            "HMAC hash must be SHA-256, SHA-384 or SHA-512",
        );
    };
    if key_data.is_empty() {
        return rejected(&ctx, "DataError", "HMAC key must not be empty");
    }
    if let Some(length) = algorithm_obj.get::<_, Option<u64>>("length")? {
        let available = (key_data.len() as u64) * 8;
        if length == 0 || length > available || length <= available.saturating_sub(8) {
            return rejected(&ctx, "DataError", "invalid HMAC key length");
        }
        let last_bits = (length % 8) as u8;
        if last_bits != 0 {
            let last = key_data.last_mut().expect("non-empty key was checked");
            *last &= 0xff << (8 - last_bits);
        }
    }
    let mut usages_out: Vec<StdString> = Vec::new();
    if let Some(usages) = usages.0 {
        for index in 0..usages.len() {
            let usage: Coerced<StdString> = usages.get(index)?;
            if !matches!(usage.0.as_str(), "sign" | "verify") {
                return rejected(
                    &ctx,
                    "SyntaxError",
                    "HMAC key usages must be 'sign' or 'verify'",
                );
            }
            usages_out.push(usage.0);
        }
    }
    if usages_out.is_empty() {
        return rejected(&ctx, "SyntaxError", "key usages must not be empty");
    }
    let key = Class::instance(
        ctx.clone(),
        CryptoKey {
            hash,
            key: key_data,
            usages: usages_out,
        },
    )?;
    resolved(&ctx, key)
}

/// The outcome of the shared sign/verify prelude.
enum HmacGate<'js> {
    Ready(HashAlg, Vec<u8>),
    Rejected(Promise<'js>),
}

/// Shared sign/verify prelude: HMAC algorithm, CryptoKey with the usage.
fn hmac_gate<'js>(
    ctx: &Ctx<'js>,
    algorithm: &Value<'js>,
    key: &Class<'js, CryptoKey>,
    usage: &str,
) -> Result<HmacGate<'js>> {
    let Some(name) = algorithm_name(algorithm) else {
        return Err(Exception::throw_type(
            ctx,
            "algorithm must be a string or { name }",
        ));
    };
    if !name.eq_ignore_ascii_case("HMAC") {
        return Ok(HmacGate::Rejected(rejected(
            ctx,
            "NotSupportedError",
            "only HMAC sign/verify is supported",
        )?));
    }
    let key = key.borrow();
    if !key.usages.iter().any(|u| u == usage) {
        return Ok(HmacGate::Rejected(rejected(
            ctx,
            "InvalidAccessError",
            "the key does not allow this operation",
        )?));
    }
    Ok(HmacGate::Ready(key.hash, key.key.clone()))
}

/// `crypto.subtle.sign('HMAC', key, data)` -> Promise<ArrayBuffer>.
fn subtle_sign<'js>(
    ctx: Ctx<'js>,
    algorithm: Value<'js>,
    key: Class<'js, CryptoKey>,
    data: Value<'js>,
) -> Result<Promise<'js>> {
    let data = bytes_argument(&ctx, data)?;
    match hmac_gate(&ctx, &algorithm, &key, "sign")? {
        HmacGate::Rejected(promise) => Ok(promise),
        HmacGate::Ready(hash, key_bytes) => {
            let buffer = rquickjs::ArrayBuffer::new(ctx.clone(), hash.hmac(&key_bytes, &data))?;
            resolved(&ctx, buffer)
        }
    }
}

/// `crypto.subtle.verify('HMAC', key, signature, data)` -> Promise<boolean>.
/// A signature of the wrong length is false, not an error.
fn subtle_verify<'js>(
    ctx: Ctx<'js>,
    algorithm: Value<'js>,
    key: Class<'js, CryptoKey>,
    signature: Value<'js>,
    data: Value<'js>,
) -> Result<Promise<'js>> {
    let signature = bytes_argument(&ctx, signature)?;
    let data = bytes_argument(&ctx, data)?;
    match hmac_gate(&ctx, &algorithm, &key, "verify")? {
        HmacGate::Rejected(promise) => Ok(promise),
        HmacGate::Ready(hash, key_bytes) => {
            let expected = hash.hmac(&key_bytes, &data);
            let ok = expected.len() == signature.len()
                && expected
                    .iter()
                    .zip(signature.iter())
                    .fold(0u8, |acc, (a, b)| acc | (a ^ b))
                    == 0;
            resolved(&ctx, ok)
        }
    }
}

/// Install the `crypto` global and the `CryptoKey` class.
pub fn install(ctx: &Ctx<'_>) -> Result<()> {
    let globals = ctx.globals();
    Class::<CryptoKey>::define(&globals)?;
    let subtle = Object::new(ctx.clone())?;
    subtle.set("digest", Function::new(ctx.clone(), subtle_digest)?)?;
    subtle.set("importKey", Function::new(ctx.clone(), subtle_import_key)?)?;
    subtle.set("sign", Function::new(ctx.clone(), subtle_sign)?)?;
    subtle.set("verify", Function::new(ctx.clone(), subtle_verify)?)?;
    let crypto = Object::new(ctx.clone())?;
    crypto.set(
        "getRandomValues",
        Function::new(ctx.clone(), get_random_values)?,
    )?;
    crypto.set("randomUUID", Function::new(ctx.clone(), random_uuid)?)?;
    crypto.set("subtle", subtle)?;
    globals.set("crypto", crypto)?;
    Ok(())
}
