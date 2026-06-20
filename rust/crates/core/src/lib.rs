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
    /// Ordered day list: `restricted` followed by `diverse`. The order fixes
    /// the day index used throughout the solver, so it must not change.
    pub fn days(&self) -> Vec<String> {
        let mut d = self.restricted.clone();
        d.extend(self.diverse.iter().cloned());
        d
    }
}

/// The roster/config combination is structurally infeasible.
#[derive(Debug, Error, PartialEq, Eq)]
#[error("{0}")]
pub struct ValidationError(pub String);

/// A lexicographic stage found no solution within its time budget.
#[derive(Debug, Error, PartialEq, Eq)]
#[error("{0}")]
pub struct SolveError(pub String);

/// Validate the roster/config. Error messages mirror the Python originals
/// (`tests/test_solver.py` matches the substrings "day", "positive",
/// "unknown").
pub fn validate(kids: &[Kid], cfg: &Config) -> Result<(), ValidationError> {
    if cfg.days().is_empty() {
        return Err(ValidationError(
            "need at least one restricted or diverse day".into(),
        ));
    }
    if cfg.boats <= 0 {
        return Err(ValidationError("boats must be positive".into()));
    }
    // len(kids) % boats != 0  (boats > 0 here)
    let n = kids.len() as i64;
    if n % cfg.boats != 0 {
        return Err(ValidationError(format!(
            "{} kids not divisible by {} boats",
            kids.len(),
            cfg.boats
        )));
    }
    let known: std::collections::HashSet<&str> = cfg.levels.iter().map(String::as_str).collect();
    let mut bad: Vec<&str> = kids
        .iter()
        .map(|k| k.level.as_str())
        .filter(|l| !known.contains(l))
        .collect();
    bad.sort_unstable();
    bad.dedup();
    if !bad.is_empty() {
        let bad_fmt = bad
            .iter()
            .map(|l| format!("'{l}'"))
            .collect::<Vec<_>>()
            .join(", ");
        let known_fmt = cfg
            .levels
            .iter()
            .map(|l| format!("'{l}'"))
            .collect::<Vec<_>>()
            .join(", ");
        return Err(ValidationError(format!(
            "unknown level(s) [{bad_fmt}]; known levels are [{known_fmt}]"
        )));
    }
    Ok(())
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
        let err = validate(&kids, &cfg).unwrap_err();
        assert!(err.0.contains("day"), "msg: {}", err.0);
    }

    #[test]
    fn validate_boats_zero_raises() {
        let kids = kids_from_counts(&[("k", 2)]);
        let cfg = Config {
            boats: 0,
            levels: svec(&["k"]),
            ..Default::default()
        };
        let err = validate(&kids, &cfg).unwrap_err();
        assert!(err.0.contains("positive"), "msg: {}", err.0);
    }

    #[test]
    fn validate_boats_negative_raises() {
        let kids = kids_from_counts(&[("k", 2)]);
        let cfg = Config {
            boats: -1,
            levels: svec(&["k"]),
            ..Default::default()
        };
        assert!(validate(&kids, &cfg).is_err());
    }

    #[test]
    fn validate_indivisible_raises() {
        let kids = kids_from_counts(&[("k", 5)]);
        let cfg = Config {
            boats: 2,
            levels: svec(&["k"]),
            ..Default::default()
        };
        assert!(validate(&kids, &cfg).is_err());
    }

    #[test]
    fn validate_unknown_level_raises() {
        let kids = vec![Kid::new("A", "z")];
        let cfg = Config {
            boats: 1,
            levels: svec(&["k"]),
            ..Default::default()
        };
        let err = validate(&kids, &cfg).unwrap_err();
        assert!(err.0.contains("unknown"), "msg: {}", err.0);
    }

    #[test]
    fn validate_valid_passes() {
        let kids = kids_from_counts(&[("k", 4), ("b", 4)]);
        let cfg = Config {
            boats: 4,
            levels: svec(&["k", "b"]),
            ..Default::default()
        };
        assert!(validate(&kids, &cfg).is_ok());
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
