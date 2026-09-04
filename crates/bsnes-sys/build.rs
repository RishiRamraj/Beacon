use std::path::{Path, PathBuf};
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

    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let target = std::env::var("TARGET").unwrap();

    // Each target builds the core in its own copy of the vendored tree.
    //
    // The makefile pins its object directory with `override OBJDIR := objs`, inside its own
    // tree, so it cannot be redirected from the command line and two targets would overwrite
    // each other's objects. That is not theoretical: the first Windows cross-build linked the
    // Linux library and failed on every symbol in it. Copying the tree — 25MB, once per
    // target — is the cheap way to isolate them without patching a vendored makefile.
    let tree = out.join(format!("bsnes-jg-{target}"));
    let src = tree.join("src");
    let lib = tree.join("objs/libbsnes.a");

    // The core is where the CPU goes, and it is worth knowing by how much: holding 60fps
    // costs about three quarters of a core, against about two per cent for the whole ALttP
    // plugin (measured by stubbing its on_frame out under a running game).
    //
    // The flags are explicit here because they are a knob someone will reach for, and the
    // vendored makefile's own default is easy to miss (mk/common.mk sets `CXXFLAGS ?= -O2`,
    // which is what was in force all along — a grep of the top-level Makefile suggests -O0
    // and is wrong). Measured from one savestate, three samples each: -O2 about 81%, -O3
    // about 76%, -O2 -march=native about 75%. All within a few points of each other, so the
    // core is simply expensive rather than badly built, and the portable default stands.
    //
    // BEACON_NATIVE=1 tunes for the building machine. Off by default because the Windows
    // package is cross-built here and shipped elsewhere.
    let opt = match std::env::var("BEACON_NATIVE").ok().as_deref() {
        Some("1") => "-O2 -march=native",
        _ => "-O2",
    };
    let stamp = tree.join(".beacon-cxxflags");
    let stale = std::fs::read_to_string(&stamp).map(|s| s != opt).unwrap_or(true);

    if !tree.exists() {
        copy_tree(&vendor, &tree);
    }
    // A flag change has to rebuild, and the guard below is "does the library exist" — so
    // without this, editing the flags above would silently keep the old library forever.
    if stale && lib.exists() {
        let _ = std::fs::remove_dir_all(tree.join("objs"));
    }

    println!("cargo:rerun-if-changed=csrc/shim.cpp");
    println!("cargo:rerun-if-changed=csrc/shim.h");
    println!(
        "cargo:rerun-if-changed={}",
        vendor.join("src/bsnes.hpp").display()
    );

    if !lib.exists() {
        let jobs = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1);

        // The target's own toolchain, not the host's. `cc` already knows how to find it —
        // including a cross prefix like x86_64-w64-mingw32- — so asking it beats guessing,
        // and the makefile takes all three as ordinary `?=` variables.
        let probe = cc::Build::new();
        let cc_tool = probe.get_compiler();
        let cxx_tool = cc::Build::new().cpp(true).get_compiler();
        let ar_tool = probe.get_archiver();

        let status = Command::new("make")
            .current_dir(&tree)
            .args([
                "ENABLE_STATIC=1",
                "DISABLE_MODULE=1",
                "USE_VENDORED_SAMPLERATE=1",
                &format!("CC={}", cc_tool.path().display()),
                &format!("CXX={}", cxx_tool.path().display()),
                &format!("AR={}", ar_tool.get_program().to_string_lossy()),
                &format!("CFLAGS={opt}"),
                &format!("CXXFLAGS={opt}"),
                &format!("-j{jobs}"),
            ])
            .status()
            .expect("failed to run make - is it installed?");

        assert!(status.success(), "bsnes-jg build failed");
    }

    assert!(lib.exists(), "expected {} after build", lib.display());
    let _ = std::fs::write(&stamp, opt);

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
    //
    // From the TARGET, not from `cfg!`. In a build script `cfg!` describes the machine doing
    // the building, so cross-compiling to Windows from Linux asked for libstdc++ by accident
    // and would have asked for the wrong one entirely from a Mac.
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    match target_os.as_str() {
        "macos" | "ios" => println!("cargo:rustc-link-lib=dylib=c++"),
        _ if target_env == "msvc" => {}
        _ => println!("cargo:rustc-link-lib=dylib=stdc++"),
    }
}

/// Copies the vendored source tree, skipping git metadata and any objects already built for
/// another target.
fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("could not create the build tree");
    for entry in std::fs::read_dir(from).expect("could not read the vendored tree") {
        let entry = entry.expect("could not read a vendored entry");
        let name = entry.file_name();
        // `.git` is a submodule pointer and `objs` belongs to whoever built it.
        if name == ".git" || name == "objs" {
            continue;
        }
        let dest = to.join(&name);
        if entry.file_type().expect("could not stat").is_dir() {
            copy_tree(&entry.path(), &dest);
        } else {
            std::fs::copy(entry.path(), &dest).expect("could not copy a vendored file");
        }
    }
}
