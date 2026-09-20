//! Carries the device program inside the bridge, so that starting a pixbar needs this one binary and no adb.
//!
//! The device program is an ARM build of `pixbar-device`, and this script makes it: a bridge that was built
//! without it, or with yesterday's, looks fine until it has to start a panel. `PIXBAR_DEVICE_BIN=<file>` takes a
//! ready-made one instead (a packager's, or a machine without the ARM target).

use std::path::{Path, PathBuf};
use std::process::Command;

const TARGET: &str = "armv7-unknown-linux-musleabihf";

fn main() {
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("pixbar-device");
    println!("cargo:rerun-if-env-changed=PIXBAR_DEVICE_BIN");
    if let Some(ready) = std::env::var_os("PIXBAR_DEVICE_BIN").filter(|p| !p.is_empty()) {
        println!("cargo:rerun-if-changed={}", Path::new(&ready).display());
        let bytes = std::fs::read(&ready).unwrap_or_else(|e| panic!("PIXBAR_DEVICE_BIN={}: {e}", Path::new(&ready).display()));
        assert!(bytes.starts_with(b"\x7fELF"), "PIXBAR_DEVICE_BIN={} is not an ELF program", Path::new(&ready).display());
        std::fs::write(&out, bytes).unwrap();
        return;
    }

    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    // Everything the device program is made of. (A directory counts with all that is in it.)
    for part in ["crates/pixbar-device", "crates/pixbar-render", "crates/pixbar-proto", "Cargo.toml", "Cargo.lock"] {
        println!("cargo:rerun-if-changed={}", workspace.join(part).display());
    }

    // A target directory of its own: the one this build runs in is locked by the cargo that runs this script.
    // OUT_DIR is <target>[/<triple>]/<profile>/build/<package>-<hash>/out; next to <profile>, so that debug and
    // release builds of the bridge share one ARM build.
    let arm_dir = out.ancestors().nth(5).expect("OUT_DIR has cargo's layout").join("pixbar-device-arm");
    let mut cargo = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    cargo
        .current_dir(&workspace)
        .args(["build", "--release", "--locked", "-p", "pixbar-device", "--target", TARGET, "--target-dir"])
        .arg(&arm_dir)
        // Not left to .cargo/config.toml, which cargo only finds from inside the checkout: linked with the lld
        // that ships with Rust, so no cross gcc is needed.
        .args(["--config", &format!("target.{TARGET}.linker=\"rust-lld\"")])
        .args(["--config", &format!("target.{TARGET}.rustflags=[\"-C\",\"link-self-contained=yes\",\"-C\",\"target-feature=+crt-static\"]")]);
    // What cargo set up for the host build of the bridge must not leak into the ARM build of the device program.
    for var in ["CARGO_ENCODED_RUSTFLAGS", "RUSTFLAGS", "CARGO_BUILD_TARGET", "CARGO_TARGET_DIR", "CARGO_BUILD_TARGET_DIR", "RUSTC_WORKSPACE_WRAPPER", "TARGET", "HOST", "PROFILE", "OPT_LEVEL", "DEBUG"] {
        cargo.env_remove(var);
    }
    let run = cargo.output().unwrap_or_else(|e| panic!("could not run cargo for the device program: {e}"));
    if !run.status.success() {
        let said = String::from_utf8_lossy(&run.stderr);
        if said.contains("target may not be installed") || said.contains("can't find crate for `core`") || said.contains("can't find crate for `std`") {
            panic!("\n\nThe pixbar device program is an ARM build, and this Rust has no standard library for it yet:\n\n    rustup target add {TARGET}\n\n(or PIXBAR_DEVICE_BIN=<a pixbar-device built elsewhere>)\n\n");
        }
        panic!("\n\nbuilding the device program failed:\n{said}\n");
    }
    std::fs::copy(arm_dir.join(TARGET).join("release/pixbar-device"), &out).expect("the ARM build's pixbar-device");
}
