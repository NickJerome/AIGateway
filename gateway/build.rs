use sha2::{Digest, Sha256};
use std::{env, fs};
fn main() {
    println!("cargo:rerun-if-changed=Cargo.lock");
    println!("cargo:rerun-if-changed=Cargo.toml");
    let lock = fs::read_to_string("Cargo.lock").expect("locked dependencies");
    let sys = lock
        .split("[[package]]")
        .find(|s| s.contains("name = \"rquickjs-sys\"\n"))
        .expect("rquickjs-sys dependency");
    let target = env::var("TARGET").unwrap();
    let width = env::var("CARGO_CFG_TARGET_POINTER_WIDTH").unwrap();
    let endian = env::var("CARGO_CFG_TARGET_ENDIAN").unwrap();
    let mut inputs = format!("{sys}\n{}", fs::read_to_string("Cargo.toml").unwrap());
    for key in [
        "CARGO_ENCODED_RUSTFLAGS",
        "CARGO_CFG_TARGET_FEATURE",
        "OPT_LEVEL",
        "DEBUG",
        "CC",
        "AR",
        "CFLAGS",
        "TARGET_CFLAGS",
    ] {
        println!("cargo:rerun-if-env-changed={key}");
        inputs.push_str(&format!("\n{key}={}", env::var(key).unwrap_or_default()));
    }
    for key in [
        format!("CFLAGS_{target}"),
        format!("CFLAGS_{}", target.replace('-', "_")),
    ] {
        println!("cargo:rerun-if-env-changed={key}");
        inputs.push_str(&format!("\n{key}={}", env::var(&key).unwrap_or_default()));
    }
    let fingerprint = hex::encode(Sha256::digest(inputs.as_bytes()));
    println!(
        "cargo:rustc-env=GATEWAY_BUILD_ID=qjs-ng-0.16.2/{target}/{width}/{endian}/wopts-default/{fingerprint}"
    );
}
