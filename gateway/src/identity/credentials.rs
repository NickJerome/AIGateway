//! Secret generation and comparison; never expose raw credentials through public records.
use anyhow::Result;
use sha2::{Digest, Sha256};
use std::io::Read;
pub fn secret_equal(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0, |v, (a, b)| v | (a ^ b)) == 0
}
pub(super) fn digest(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}
pub(super) fn random_hex(bytes: usize) -> Result<String> {
    let mut value = vec![0; bytes];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut value)?;
    Ok(hex::encode(value))
}
pub(super) fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
