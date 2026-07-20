//! Compiles and embeds the `bee-ebpf` BPF object — but only when building with `--features enforce`.
//! Without the feature (the default host build) this is a no-op, so a stock stable toolchain works.
//!
//! We invoke the bpf build directly (rather than via aya-build) because `bee-ebpf` is intentionally
//! excluded from the workspace, so aya-build's `--package` lookup wouldn't find it. The command here
//! matches the aya development guide: nightly + `build-std=core` + `--btf` link arg.

use std::path::PathBuf;
use std::process::Command;

fn main() {
    if std::env::var_os("CARGO_FEATURE_ENFORCE").is_none() {
        return; // default host build: no eBPF compilation
    }

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let ebpf_dir = manifest_dir.join("..").join("bee-ebpf");
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());

    println!("cargo:rerun-if-changed=../bee-ebpf/src");
    println!("cargo:rerun-if-changed=../bee-common/src");
    println!("cargo:rerun-if-changed=../bee-ebpf/Cargo.toml");

    let mut cmd = Command::new("cargo");
    cmd.current_dir(&ebpf_dir)
        // Clear inherited toolchain/rustflags so the nightly bpf build isn't polluted by the outer
        // stable build's environment.
        .env_remove("RUSTUP_TOOLCHAIN")
        .env_remove("RUSTC")
        .env_remove("RUSTC_WRAPPER")
        .env_remove("RUSTC_WORKSPACE_WRAPPER")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("CARGO_BUILD_TARGET")
        .env("RUSTFLAGS", "-C link-arg=--btf")
        .args([
            "+nightly",
            "build",
            "--target",
            "bpfel-unknown-none",
            "-Z",
            "build-std=core",
            "--release",
        ]);

    let status = cmd.status().expect("failed to invoke cargo for bee-ebpf");
    assert!(status.success(), "bee-ebpf BPF build failed");

    let obj = ebpf_dir.join("target/bpfel-unknown-none/release/bee");
    std::fs::copy(&obj, out_dir.join("bee"))
        .unwrap_or_else(|e| panic!("copy {} -> OUT_DIR/bee: {e}", obj.display()));
}
