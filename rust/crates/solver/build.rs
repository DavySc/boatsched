//! Build the C++ CP-SAT shim and link it against the OR-Tools C++ SDK.
//!
//! The SDK (headers + `libortools.so` + bundled abseil/protobuf) is located
//! via the `ORTOOLS_ROOT` env var, or auto-discovered under `rust/vendor/` as
//! `or-tools_*_cpp_*`. See `rust/README.md` for how the SDK is obtained — it is
//! a large, gitignored, version-pinned (9.15.6755) artifact, not vendored into
//! git.

use std::path::{Path, PathBuf};

fn find_sdk() -> PathBuf {
    if let Ok(root) = std::env::var("ORTOOLS_ROOT") {
        let p = PathBuf::from(root);
        assert!(p.join("include/ortools/sat/cp_model.h").exists(),
            "ORTOOLS_ROOT={} does not contain include/ortools/sat/cp_model.h", p.display());
        return p;
    }
    // Auto-discover under ../../vendor relative to this crate.
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let vendor = manifest.join("../../vendor").canonicalize().unwrap_or_else(|_| {
        panic!("OR-Tools SDK not found: set ORTOOLS_ROOT or place it under rust/vendor/ \
                (see rust/README.md). manifest={}", manifest.display())
    });
    for entry in std::fs::read_dir(&vendor).expect("read rust/vendor").flatten() {
        let p = entry.path();
        if p.is_dir()
            && p.file_name().and_then(|n| n.to_str()).map_or(false, |n| n.starts_with("or-tools"))
            && p.join("include/ortools/sat/cp_model.h").exists()
        {
            return p;
        }
    }
    panic!("OR-Tools SDK not found under {}; set ORTOOLS_ROOT (see rust/README.md)", vendor.display());
}

fn main() {
    let sdk = find_sdk();
    let include = sdk.join("include");
    let lib = sdk.join("lib");

    println!("cargo:rerun-if-changed=cpp/cpsat_shim.cpp");
    println!("cargo:rerun-if-env-changed=ORTOOLS_ROOT");

    // Compile definitions mirror the ortools::ortools CMake INTERFACE target
    // (`OR_PROTO_DLL=` empties the proto export macro for static linking; the
    // USE_* flags match the prebuilt library's feature set).
    cc::Build::new()
        .cpp(true)
        .std("c++17")
        .file("cpp/cpsat_shim.cpp")
        .include(&include)
        .define("OR_PROTO_DLL", "")
        .define("USE_MATH_OPT", None)
        .define("USE_BOP", None)
        .define("USE_CBC", None)
        .define("USE_CLP", None)
        .define("USE_GLOP", None)
        .define("USE_HIGHS", None)
        .define("USE_PDLP", None)
        .define("USE_SCIP", None)
        .flag_if_supported("-Wno-unused-parameter")
        .flag_if_supported("-Wno-deprecated-declarations")
        .flag_if_supported("-Wno-sign-compare")
        .opt_level(2)
        .compile("cpsat_shim");

    // Link the OR-Tools shared library and make the loader find it (and its
    // bundled abseil/protobuf deps) at runtime via rpath.
    println!("cargo:rustc-link-search=native={}", lib.display());
    println!("cargo:rustc-link-lib=dylib=ortools");
    println!("cargo:rustc-link-arg=-Wl,-rpath,{}", lib.display());
    let _ = Path::new(&lib);
}
