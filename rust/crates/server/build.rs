//! Two jobs: embed the built SPA, and emit the OR-Tools rpath for this crate's
//! binaries/tests.
//!
//! **SPA.** `web/dist/index.html` is a single self-contained file. Without a
//! frontend build (plain `cargo test`) a placeholder page is embedded instead, so
//! the crate still compiles; the Docker build always runs the web build first.
//!
//! **rpath.**
//! `boatsched-server` links the OR-Tools C++ library transitively through
//! `boatsched-solver`. A build script's `cargo:rustc-link-arg` only applies to
//! the crate it belongs to, so the rpath the solver's build script sets does
//! **not** propagate to the binaries/tests produced here — without this they'd
//! fail at startup with `libortools.so.9: cannot open shared object file`.
//!
//! We re-discover the SDK (same logic as `crates/solver/build.rs`: `ORTOOLS_ROOT`
//! or auto-discovery under `rust/vendor/`) and set the rpath so the loader finds
//! `libortools.so` and its bundled deps at runtime.

use std::path::{Path, PathBuf};

const PLACEHOLDER: &str = "<!doctype html><meta charset=\"utf-8\"><title>PZK</title>\
<p>Frontend not built. Run <code>npm ci &amp;&amp; npm run build</code> in <code>web/</code> and rebuild the server.</p>";

fn embed_spa() {
    let dist = Path::new("../../../web/dist/index.html");
    println!("cargo:rerun-if-changed={}", dist.display());
    let out = Path::new(&std::env::var("OUT_DIR").expect("OUT_DIR")).join("index.html");
    let bytes = std::fs::read(dist).unwrap_or_else(|_| PLACEHOLDER.as_bytes().to_vec());
    std::fs::write(out, &bytes).expect("write embedded index.html");
    // ETag for the embedded page, so a returning phone revalidates with a 304
    // instead of downloading the whole inlined bundle again.
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut h);
    println!("cargo:rustc-env=SPA_HASH={:016x}", h.finish());
}

fn find_sdk_lib() -> Option<PathBuf> {
    if let Ok(root) = std::env::var("ORTOOLS_ROOT") {
        let lib = PathBuf::from(root).join("lib");
        if lib.exists() {
            return Some(lib);
        }
    }
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").ok()?);
    let vendor = manifest.join("../../vendor").canonicalize().ok()?;
    for entry in std::fs::read_dir(&vendor).ok()?.flatten() {
        let p = entry.path();
        if p.is_dir()
            && p.file_name()
                .and_then(|n| n.to_str())
                .map_or(false, |n| n.starts_with("or-tools"))
            && p.join("lib").exists()
        {
            return Some(p.join("lib"));
        }
    }
    None
}

fn main() {
    embed_spa();
    println!("cargo:rerun-if-env-changed=ORTOOLS_ROOT");
    if let Some(lib) = find_sdk_lib() {
        println!("cargo:rustc-link-search=native={}", lib.display());
        println!("cargo:rustc-link-arg=-Wl,-rpath,{}", lib.display());
    }
    // If not found, the solver crate's own build will already have failed; we
    // stay silent so a no-solver build (should that ever exist) isn't blocked.
}
