//! Core domain types shared across the boatsched crates.
//!
//! Mirrors the pure-data portions of the Python `solver.py` (`Config`,
//! `Kid`, `ScheduleRow`, `validate`, error types) plus the closed
//! enrollment-level enum enforced by the DB `CHECK` constraint.
//!
//! Note on levels: the *solver* treats levels as arbitrary ordered string
//! labels (`Config.levels`, e.g. `["x","y","z"]` in tests), so `Kid.level`
//! and `Config.levels` are `String`s — NOT the [`EnrollmentLevel`] enum.
//! [`EnrollmentLevel`] models the separate, fixed DB/API enum (`k|b|A|B`).

use thiserror::Error;

pub mod authz;

/// A participant: `(name, level)` where `level` is a label from `Config.levels`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Kid {
    pub name: String,
    pub level: String,
}

impl Kid {
    pub fn new(name: impl Into<String>, level: impl Into<String>) -> Self {
        Kid {
            name: name.into(),
            level: level.into(),
        }
    }
}

/// One output assignment row: `(day, boat, name, level)` with 1-based boat.
/// Serializes to `{"day","boat","name","level"}`, matching the JSON the Python
/// `/schedule` and `/status` endpoints emit.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ScheduleRow {
    pub day: String,
    pub boat: i64,
    pub name: String,
    pub level: String,
}

/// Problem and solver configuration. Field defaults match the Python
/// `Config` dataclass (stage_seconds=20.0, seconds=115.0); the CLI and server
/// override stage_seconds/seconds at their own layer (45/60).
/// Serializes to the same JSON shape as Python's `asdict(Config)` (keys
/// `boats, levels, restricted, diverse, adj, stage_seconds, seconds, workers`),
/// so the `schedule.config` TEXT column written by `save_schedule` matches.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Config {
    /// May be <= 0 so that [`validate`] can reject it (parity with Python).
    pub boats: i64,
    pub levels: Vec<String>,
    pub restricted: Vec<String>,
    pub diverse: Vec<String>,
    pub adj: i64,
    pub stage_seconds: f64,
    pub seconds: f64,
    pub workers: i64,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            boats: 13,
            levels: svec(&["k", "b", "A", "B"]),
            restricted: svec(&["Sat", "Sun", "Tue", "Wed"]),
            diverse: svec(&["Fri", "Mon"]),
            adj: 1,
            stage_seconds: 20.0,
            seconds: 115.0,
            workers: 8,
        }
    }
}

impl Config {
    /// The float-free projection of this config that the formally-verified
    /// domain core ([`validate`], [`ProblemConfig::days`]) operates on. The
    /// solver-timing fields (`stage_seconds`, `seconds`, `workers`) are dropped
    /// because Aeneas's `F64` cannot appear in an extracted `struct`; only
    /// [`ProblemConfig`] crosses the verification boundary.
    pub fn problem(&self) -> ProblemConfig {
        ProblemConfig {
            boats: self.boats,
            levels: self.levels.clone(),
            restricted: self.restricted.clone(),
            diverse: self.diverse.clone(),
            adj: self.adj,
        }
    }

    /// Ordered day list: `restricted` followed by `diverse`. Delegates to the
    /// verified [`ProblemConfig::days`] so the solver and the proof share one
    /// definition. The order fixes the day index used throughout the solver, so
    /// it must not change.
    pub fn days(&self) -> Vec<String> {
        self.problem().days()
    }
}

/// The float-free subset of [`Config`] that the formally-verified domain core
/// operates on. Aeneas cannot represent `f64` inside an extracted `struct`, so
/// the solver-timing fields live only on [`Config`]; everything the
/// roster/config *validation* and day-ordering logic needs lives here and is
/// provable against `spec/BoatschedSpec` (see EXT-VAL).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ProblemConfig {
    /// May be <= 0 so that [`validate`] can reject it (parity with Python).
    pub boats: i64,
    pub levels: Vec<String>,
    pub restricted: Vec<String>,
    pub diverse: Vec<String>,
    pub adj: i64,
}

impl Default for ProblemConfig {
    fn default() -> Self {
        // Mirrors the problem fields of `Config::default`.
        ProblemConfig {
            boats: 13,
            levels: svec(&["k", "b", "A", "B"]),
            restricted: svec(&["Sat", "Sun", "Tue", "Wed"]),
            diverse: svec(&["Fri", "Mon"]),
            adj: 1,
        }
    }
}

impl ProblemConfig {
    /// Ordered day list: `restricted` followed by `diverse`. Written as explicit
    /// index loops (no iterator combinators) so Aeneas can extract it.
    pub fn days(&self) -> Vec<String> {
        let mut d: Vec<String> = Vec::new();
        let mut i = 0;
        while i < self.restricted.len() {
            d.push(self.restricted[i].clone());
            i += 1;
        }
        let mut j = 0;
        while j < self.diverse.len() {
            d.push(self.diverse[j].clone());
            j += 1;
        }
        d
    }

    /// Is `level` one of the configured solver levels? Explicit loop (no
    /// `HashSet`/iterator) so it extracts cleanly; the single `String`
    /// comparison is the one opaque external Aeneas emits (modelled `a == b`).
    fn knows_level(&self, level: &String) -> bool {
        let mut i = 0;
        while i < self.levels.len() {
            if self.levels[i] == *level {
                return true;
            }
            i += 1;
        }
        false
    }

    /// Are all kids' levels configured? Explicit loop with an early exit (no
    /// iterator/`HashSet`). A boolean rather than the offending-level list
    /// because Aeneas cannot join a loop that *conditionally* pushes to an
    /// owned `Vec`; the decision (not the diagnostic list) is what `validate`
    /// and the spec need, and this shape extracts and proves cleanly (EXT-VAL).
    fn all_known(&self, kids: &[Kid]) -> bool {
        let mut i = 0;
        let n = kids.len();
        while i < n {
            if !self.knows_level(&kids[i].level) {
                return false;
            }
            i += 1;
        }
        true
    }
}

/// The roster/config combination is structurally infeasible. The variants
/// mirror the Lean taxonomy in `spec/BoatschedSpec/Validate.lean`
/// (`ValidationError`) exactly — the refinement proof (EXT-VAL) maps one onto
/// the other. Human-readable text lives in the `Display` impl **outside** the
/// Aeneas verification boundary; callers/tests match on the substrings "day",
/// "positive", "divisible", "unknown".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationError {
    /// No restricted or diverse days configured (the first guard).
    NoDays,
    /// `boats <= 0`.
    BoatsNotPositive,
    /// Roster size (`kids`) is not divisible by `boats`.
    NotDivisible { kids: i64, boats: i64 },
    /// One or more kids carry a level not in `Config.levels`. Carries no list:
    /// see [`ProblemConfig::all_known`] for why the verified core decides this
    /// as a boolean (mirrors Lean `ValidationError.unknownLevels`).
    UnknownLevels,
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ValidationError::NoDays => {
                write!(f, "need at least one restricted or diverse day")
            }
            ValidationError::BoatsNotPositive => write!(f, "boats must be positive"),
            ValidationError::NotDivisible { kids, boats } => {
                write!(f, "{kids} kids not divisible by {boats} boats")
            }
            ValidationError::UnknownLevels => {
                write!(f, "unknown level(s) present in roster")
            }
        }
    }
}

impl std::error::Error for ValidationError {}

/// A lexicographic stage found no solution within its time budget.
#[derive(Debug, Error, PartialEq, Eq)]
#[error("{0}")]
pub struct SolveError(pub String);

/// Validate the roster/config. Returns the first failure reason (typed), or
/// `Ok(())`. Written in an Aeneas-extractable style (index loops, no
/// `HashSet`/`format!`/`sort`/iterators) and taking the float-free
/// [`ProblemConfig`]; this is the function bound to the Lean model
/// `Boatsched.validate` by the EXT-VAL refinement proof. The check order —
/// days, boats, divisibility, unknown levels — is load-bearing (callers and the
/// spec depend on it).
pub fn validate(kids: &[Kid], cfg: &ProblemConfig) -> Result<(), ValidationError> {
    if cfg.days().is_empty() {
        return Err(ValidationError::NoDays);
    }
    if cfg.boats <= 0 {
        return Err(ValidationError::BoatsNotPositive);
    }
    // len(kids) % boats != 0  (boats > 0 here)
    let n = kids.len() as i64;
    if n % cfg.boats != 0 {
        return Err(ValidationError::NotDivisible {
            kids: n,
            boats: cfg.boats,
        });
    }
    if cfg.all_known(kids) {
        Ok(())
    } else {
        Err(ValidationError::UnknownLevels)
    }
}

/// The fixed enrollment-level enum, ordered low→high. Mirrors the DB
/// `CHECK (level IN ('k','b','A','B'))` and the API `VALID_LEVELS` tuple.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EnrollmentLevel {
    /// Ketelbinkie (lowest)
    K,
    /// Bemanning
    B,
    /// A
    A,
    /// B (highest)
    BCap,
}

impl EnrollmentLevel {
    pub const ALL: [EnrollmentLevel; 4] = [
        EnrollmentLevel::K,
        EnrollmentLevel::B,
        EnrollmentLevel::A,
        EnrollmentLevel::BCap,
    ];

    /// The stored string form used by the DB and JSON API.
    pub fn as_str(self) -> &'static str {
        match self {
            EnrollmentLevel::K => "k",
            EnrollmentLevel::B => "b",
            EnrollmentLevel::A => "A",
            EnrollmentLevel::BCap => "B",
        }
    }

    /// Parse a stored/level string; case-sensitive like the DB CHECK.
    pub fn parse(s: &str) -> Option<EnrollmentLevel> {
        match s {
            "k" => Some(EnrollmentLevel::K),
            "b" => Some(EnrollmentLevel::B),
            "A" => Some(EnrollmentLevel::A),
            "B" => Some(EnrollmentLevel::BCap),
            _ => None,
        }
    }
}

/// Helper: build a `Vec<String>` from string slices.
pub fn svec(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kids_from_counts(counts: &[(&str, usize)]) -> Vec<Kid> {
        let mut v = Vec::new();
        for (level, n) in counts {
            for j in 0..*n {
                v.push(Kid::new(format!("{level}{j}"), *level));
            }
        }
        v
    }

    // ---- Config.days (mirrors TestConfigDays) ----
    #[test]
    fn days_restricted_before_diverse() {
        let cfg = Config {
            restricted: svec(&["Sat", "Sun"]),
            diverse: svec(&["Fri"]),
            ..Default::default()
        };
        assert_eq!(cfg.days(), svec(&["Sat", "Sun", "Fri"]));
    }

    #[test]
    fn days_only_restricted() {
        let cfg = Config {
            restricted: svec(&["Sat"]),
            diverse: vec![],
            ..Default::default()
        };
        assert_eq!(cfg.days(), svec(&["Sat"]));
    }

    #[test]
    fn days_only_diverse() {
        let cfg = Config {
            restricted: vec![],
            diverse: svec(&["Fri"]),
            ..Default::default()
        };
        assert_eq!(cfg.days(), svec(&["Fri"]));
    }

    #[test]
    fn days_empty_both() {
        let cfg = Config {
            restricted: vec![],
            diverse: vec![],
            ..Default::default()
        };
        assert!(cfg.days().is_empty());
    }

    // ---- validate (mirrors TestValidate) ----
    #[test]
    fn validate_no_days_raises() {
        let kids = kids_from_counts(&[("k", 2)]);
        let cfg = Config {
            boats: 1,
            levels: svec(&["k"]),
            restricted: vec![],
            diverse: vec![],
            ..Default::default()
        };
        let err = validate(&kids, &cfg.problem()).unwrap_err();
        assert!(matches!(err, ValidationError::NoDays), "err: {err:?}");
    }

    #[test]
    fn validate_boats_zero_raises() {
        let kids = kids_from_counts(&[("k", 2)]);
        let cfg = Config {
            boats: 0,
            levels: svec(&["k"]),
            ..Default::default()
        };
        let err = validate(&kids, &cfg.problem()).unwrap_err();
        assert!(
            matches!(err, ValidationError::BoatsNotPositive),
            "err: {err:?}"
        );
    }

    #[test]
    fn validate_boats_negative_raises() {
        let kids = kids_from_counts(&[("k", 2)]);
        let cfg = Config {
            boats: -1,
            levels: svec(&["k"]),
            ..Default::default()
        };
        assert!(validate(&kids, &cfg.problem()).is_err());
    }

    #[test]
    fn validate_indivisible_raises() {
        let kids = kids_from_counts(&[("k", 5)]);
        let cfg = Config {
            boats: 2,
            levels: svec(&["k"]),
            ..Default::default()
        };
        assert!(validate(&kids, &cfg.problem()).is_err());
    }

    #[test]
    fn validate_unknown_level_raises() {
        let kids = vec![Kid::new("A", "z")];
        let cfg = Config {
            boats: 1,
            levels: svec(&["k"]),
            ..Default::default()
        };
        let err = validate(&kids, &cfg.problem()).unwrap_err();
        assert!(
            matches!(err, ValidationError::UnknownLevels),
            "err: {err:?}"
        );
    }

    #[test]
    fn validate_valid_passes() {
        let kids = kids_from_counts(&[("k", 4), ("b", 4)]);
        let cfg = Config {
            boats: 4,
            levels: svec(&["k", "b"]),
            ..Default::default()
        };
        assert!(validate(&kids, &cfg.problem()).is_ok());
    }

    // ---- EnrollmentLevel ----
    #[test]
    fn enrollment_level_roundtrip() {
        for lv in EnrollmentLevel::ALL {
            assert_eq!(EnrollmentLevel::parse(lv.as_str()), Some(lv));
        }
        assert_eq!(EnrollmentLevel::parse("beginner"), None);
    }

    #[test]
    fn enrollment_level_ordering_low_to_high() {
        assert!(EnrollmentLevel::K < EnrollmentLevel::B);
        assert!(EnrollmentLevel::B < EnrollmentLevel::A);
        assert!(EnrollmentLevel::A < EnrollmentLevel::BCap);
    }
}
