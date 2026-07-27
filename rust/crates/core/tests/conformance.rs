//! Tier-1 conformance bridge: assert the Rust `validate` agrees with the
//! *proven* Lean model in `../../../spec`.
//!
//! The vectors (inputs + the model's verdict for each) are generated from the
//! Lean spec and checked in *next to this test* — inside the `rust/` Docker build
//! context — so the image build's `cargo test` gate can run this conformance
//! check too. Regenerate them after changing the model with:
//!
//!     cd spec && lake exe gen-vectors > ../rust/crates/core/tests/conformance_vectors.json
//!
//! The verdict tags mirror `BoatschedSpec.Conformance.verdict`. Rust's
//! `ValidationError` is a typed enum whose variants mirror the Lean taxonomy
//! one-to-one, so we classify by a direct `match` (no substring guessing).

use boatsched_core::{validate, Kid, ProblemConfig, ValidationError};
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
fn verdict_of(kids: &[Kid], cfg: &ProblemConfig) -> &'static str {
    match validate(kids, cfg) {
        Ok(()) => "ok",
        Err(ValidationError::NoDays) => "noDays",
        Err(ValidationError::BoatsNotPositive) => "boatsNotPositive",
        Err(ValidationError::NotDivisible { .. }) => "notDivisible",
        Err(ValidationError::UnknownLevels) => "unknownLevels",
    }
}

#[test]
fn rust_validate_matches_lean_oracle() {
    // Embedded at compile time so the test is self-contained and works inside the
    // Docker build context (which is `rust/` only — `spec/` is not shipped there).
    // Regenerate with:
    //   cd spec && lake exe gen-vectors > ../rust/crates/core/tests/conformance_vectors.json
    let data = include_str!("conformance_vectors.json");
    let cases: Vec<CaseJson> = serde_json::from_str(data).expect("parse conformance vectors");
    assert!(!cases.is_empty(), "no conformance cases found");

    for c in &cases {
        let kids: Vec<Kid> = c
            .kids
            .iter()
            .map(|k| Kid::new(k.name.clone(), k.level.clone()))
            .collect();
        let cfg = ProblemConfig {
            boats: c.config.boats,
            levels: c.config.levels.clone(),
            restricted: c.config.restricted.clone(),
            diverse: c.config.diverse.clone(),
            adj: c.config.adj,
        };
        let got = verdict_of(&kids, &cfg);
        assert_eq!(
            got, c.verdict,
            "case `{}`: Rust verdict `{got}` disagrees with Lean oracle `{}`",
            c.id, c.verdict
        );
    }
}
