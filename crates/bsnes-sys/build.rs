use std::path::PathBuf;
use std::process::Command;

/// Builds bsnes-jg as a static library, then compiles the C ABI shim against it.
///
/// bsnes-jg ships a GNU makefile rather than anything cargo understands, so we
/// shell out. `DISABLE_MODULE=1` builds the core library without the Jolly Good
/// frontend headers, and `USE_VENDORED_SAMPLERATE=1` avoids a system dependency
/// on libsamplerate.
fn main() {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let vendor = manifest_dir
        .join("../../vendor/bsnes-jg")
        .canonicalize()
        .expect("vendor/bsnes-jg missing - run `git submodule update --init`");

    let src = vendor.join("src");
    let lib = vendor.join("objs/libbsnes.a");

    println!("cargo:rerun-if-changed=csrc/shim.cpp");
    println!("cargo:rerun-if-changed=csrc/shim.h");
    println!("cargo:rerun-if-changed={}", src.join("bsnes.hpp").display());

    if !lib.exists() {
        let jobs = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1);

        let status = Command::new("make")
            .current_dir(&vendor)
            .args([
                "ENABLE_STATIC=1",
                "DISABLE_MODULE=1",
                "USE_VENDORED_SAMPLERATE=1",
                &format!("-j{jobs}"),
            ])
            .status()
            .expect("failed to run make - is it installed?");

        assert!(status.success(), "bsnes-jg build failed");
    }

    assert!(lib.exists(), "expected {} after build", lib.display());

    cc::Build::new()
        .cpp(true)
        .std("c++17")
        .file("csrc/shim.cpp")
        .include("csrc")
        .include(&src)
        .warnings(true)
        .compile("beacon_bsnes_shim");

    println!(
        "cargo:rustc-link-search=native={}",
        lib.parent().unwrap().display()
    );
    println!("cargo:rustc-link-lib=static=bsnes");

    // `cc` links the C++ runtime for code it compiles itself, but libbsnes.a is
    // prebuilt by make, so nothing else pulls the standard library in.
    if cfg!(target_os = "macos") {
        println!("cargo:rustc-link-lib=dylib=c++");
    } else if !cfg!(target_env = "msvc") {
        println!("cargo:rustc-link-lib=dylib=stdc++");
    }
}
