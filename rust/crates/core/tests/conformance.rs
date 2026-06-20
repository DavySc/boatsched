//! Tier-1 conformance bridge: assert the Rust `validate` agrees with the
//! *proven* Lean model in `../../../spec`.
//!
//! The vectors (inputs + the model's verdict for each) are generated from the
//! Lean spec and checked in. Regenerate them after changing the model with:
//!
//!     cd spec && lake exe gen-vectors > conformance_vectors.json
//!
//! The verdict tags mirror `BoatschedSpec.Conformance.verdict`. Rust's
//! `ValidationError` carries only a message string (no taxonomy), so we
//! classify it back into the model's tags by the stable substrings the
//! messages are built from (the same substrings `core`'s own unit tests match).

use boatsched_core::{validate, Config, Kid};
use serde::Deserialize;

#[derive(Deserialize)]
struct KidJson {
    name: String,
    level: String,
}

#[derive(Deserialize)]
struct ConfigJson {
    boats: i64,
    levels: Vec<String>,
    restricted: Vec<String>,
    diverse: Vec<String>,
    adj: i64,
}

#[derive(Deserialize)]
struct CaseJson {
    id: String,
    kids: Vec<KidJson>,
    config: ConfigJson,
    verdict: String,
}

/// Map Rust's `validate` result to the Lean model's verdict tag.
fn verdict_of(kids: &[Kid], cfg: &Config) -> &'static str {
    match validate(kids, cfg) {
        Ok(()) => "ok",
        Err(e) => {
            let m = e.0.as_str();
            // Disjoint substrings; check each model failure case.
            if m.contains("positive") {
                "boatsNotPositive"
            } else if m.contains("divisible") {
                "notDivisible"
            } else if m.contains("unknown") {
                "unknownLevels"
            } else if m.contains("day") {
                "noDays"
            } else {
                panic!("validation error not classifiable into a model verdict: {m:?}")
            }
        }
    }
}

#[test]
fn rust_validate_matches_lean_oracle() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../spec/conformance_vectors.json"
    );
    let data = std::fs::read_to_string(path).unwrap_or_else(|e| {
        panic!(
            "cannot read Lean conformance vectors at {path}: {e}\n\
             regenerate with: cd spec && lake exe gen-vectors > conformance_vectors.json"
        )
    });
    let cases: Vec<CaseJson> = serde_json::from_str(&data).expect("parse conformance vectors");
    assert!(!cases.is_empty(), "no conformance cases found");

    for c in &cases {
        let kids: Vec<Kid> = c
            .kids
            .iter()
            .map(|k| Kid::new(k.name.clone(), k.level.clone()))
            .collect();
        let cfg = Config {
            boats: c.config.boats,
            levels: c.config.levels.clone(),
            restricted: c.config.restricted.clone(),
            diverse: c.config.diverse.clone(),
            adj: c.config.adj,
            ..Config::default()
        };
        let got = verdict_of(&kids, &cfg);
        assert_eq!(
            got, c.verdict,
            "case `{}`: Rust verdict `{got}` disagrees with Lean oracle `{}`",
            c.id, c.verdict
        );
    }
}
