//! `atob` / `btoa` (spec 01 line 34): Latin-1/base64 only, not a general
//! UTF-8 codec. Invalid input throws an `InvalidCharacterError`-named error
//! (the engine has no DOMException; deviation list).

use rquickjs::{Ctx, Result};
use std::string::String as StdString;

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn invalid_character(ctx: &Ctx<'_>, message: &str) -> rquickjs::Error {
    let error = crate::web::named_error(ctx, "InvalidCharacterError", message);
    match error {
        Ok(error) => ctx.throw(error.into_value()),
        Err(err) => err,
    }
}

/// `btoa`: every input char must be a Latin-1 code point (<= 0xFF).
pub fn btoa(ctx: Ctx<'_>, data: rquickjs::Coerced<StdString>) -> Result<StdString> {
    let mut bytes = Vec::with_capacity(data.0.len());
    for ch in data.0.chars() {
        if (ch as u32) > 0xFF {
            return Err(invalid_character(
                &ctx,
                "btoa: the string contains characters outside the Latin-1 range",
            ));
        }
        bytes.push(ch as u8);
    }
    Ok(base64_encode(&bytes))
}

fn base64_encode(bytes: &[u8]) -> StdString {
    let mut out = StdString::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// `atob`: forgiving decode — ASCII whitespace is removed; anything outside
/// the alphabet or misplaced padding throws.
pub fn atob(ctx: Ctx<'_>, data: rquickjs::Coerced<StdString>) -> Result<StdString> {
    match base64_decode(&data.0) {
        Some(bytes) => Ok(bytes.iter().map(|b| *b as char).collect()),
        None => Err(invalid_character(
            &ctx,
            "atob: the string is not correctly encoded",
        )),
    }
}

fn decode_char(b: u8) -> Option<u8> {
    ALPHABET
        .iter()
        .position(|&a| a == b)
        .map(|position| position as u8)
}

fn base64_decode(input: &str) -> Option<Vec<u8>> {
    let clean: Vec<u8> = input
        .bytes()
        .filter(|b| !matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0C))
        .collect();
    // Padding is only legal as a one- or two-character suffix on a multiple
    // of four.
    let padding = clean.iter().rev().take_while(|&&b| b == b'=').count();
    if padding > 2 || (padding > 0 && !clean.len().is_multiple_of(4)) {
        return None;
    }
    let data = &clean[..clean.len() - padding];
    if data.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(data.len() / 4 * 3 + 3);
    let mut acc: u32 = 0;
    let mut bits = 0u32;
    for &b in data {
        acc = (acc << 6) | decode_char(b)? as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

/// Install `atob` and `btoa` onto the context globals.
pub fn install(ctx: &Ctx<'_>) -> Result<()> {
    let globals = ctx.globals();
    globals.set("atob", rquickjs::Function::new(ctx.clone(), atob)?)?;
    globals.set("btoa", rquickjs::Function::new(ctx.clone(), btoa)?)?;
    Ok(())
}
