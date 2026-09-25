//! Cross-builds the Linux init the macOS CLI embeds.
//!
//! The init always comes from the CLI's own build. On macOS the CLI embeds
//! the `aarch64-unknown-linux-musl` binary, so this script cross-builds it
//! with `cargo zigbuild` into its own target directory: the outer build
//! holds the lock on the workspace's `target/`. `TARGET` is what is being
//! built, so the nested musl build, whose host is still macOS, skips this
//! script and Linux builds never run `zig` at all.

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR"));
    let embedded = out.join("pinfold-init");
    let target = env::var("TARGET").unwrap_or_default();
    if !target.ends_with("-apple-darwin") {
        // The embedded-init code still has to compile on Linux so the lint
        // gates check it; this placeholder is never mounted.
        fs::write(&embedded, []).expect("write placeholder init");
        println!("cargo:rustc-env=PINFOLD_INIT={}", embedded.display());
        return;
    }
    for path in ["src", "Cargo.toml", "build.rs"] {
        println!("cargo:rerun-if-changed={path}");
    }

    let manifest = env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let target_dir = out.join("init-target");
    let status = Command::new(env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
        .args([
            "zigbuild",
            "--locked",
            "--release",
            "--bin",
            "pinfold",
            "--target",
            "aarch64-unknown-linux-musl",
        ])
        .current_dir(&manifest)
        .env("CARGO_TARGET_DIR", &target_dir)
        .status()
        .expect("run cargo zigbuild");
    assert!(
        status.success(),
        "cargo zigbuild for the embedded init failed; the macOS build needs `zig` and \
         `cargo-zigbuild`: install zig and run `cargo install cargo-zigbuild`"
    );

    let built = target_dir
        .join("aarch64-unknown-linux-musl")
        .join("release")
        .join("pinfold");
    fs::copy(&built, &embedded)
        .unwrap_or_else(|error| panic!("copy embedded init {}: {error}", built.display()));
    println!("cargo:rustc-env=PINFOLD_INIT={}", embedded.display());
}
