//! End-to-end solver parity tests — mirror the Python `tests/test_solver.py`
//! (`TestSolveInvariants`, `TestOnProgress`, `TestSolveConstraints`) and
//! `tests/test_boatsched.py::test_tiny_solve`.
//!
//! These exercise the real OR-Tools C++ shim through the FFI: they are the
//! first runtime validation that the `libortools.so` rpath resolves and the
//! shim produces correct results, not merely that it compiles. Parity bar is
//! objective values + structural invariants (CP-SAT with multiple workers is
//! nondeterministic about *which* optimum it returns; see `cpsat_shim.cpp`).

use boatsched_core::{svec, Config, Kid, ScheduleRow};
use boatsched_roster::roster_from_counts;
use boatsched_solver::{solve, SolveFailure};

// ---- shared fixtures, mirroring _kids() / _cfg() in test_solver.py ----

fn kids() -> Vec<Kid> {
    roster_from_counts(&[("k", 2), ("b", 2)])
}

/// Mirror of `_cfg(**overrides)`: boats=2, two levels, one restricted + one
/// diverse day, tiny time budgets (problems solve in milliseconds).
fn cfg() -> Config {
    Config {
        boats: 2,
        levels: svec(&["k", "b"]),
        restricted: svec(&["Sat"]),
        diverse: svec(&["Fri"]),
        stage_seconds: 10.0,
        seconds: 10.0,
        ..Default::default()
    }
}

fn run(kids: &[Kid], cfg: &Config) -> (Vec<ScheduleRow>, boatsched_solver::Stats) {
    solve(kids, cfg, None).expect("solve should succeed")
}

// ---------------------------------------------------------------------------
// test_boatsched.py::test_tiny_solve
// ---------------------------------------------------------------------------

#[test]
fn tiny_solve() {
    let kids = roster_from_counts(&[("k", 2), ("b", 2)]);
    let cfg = Config {
        boats: 2,
        levels: svec(&["k", "b"]),
        restricted: svec(&["Sat"]),
        diverse: svec(&["Fri"]),
        stage_seconds: 10.0,
        seconds: 10.0,
        ..Default::default()
    };
    let (rows, stats) = run(&kids, &cfg);
    // 4 kids x 2 days = 8 assignment rows
    assert_eq!(rows.len(), 8);
    // every kid placed exactly once per day
    for day in ["Sat", "Fri"] {
        let mut placed: Vec<&str> = rows
            .iter()
            .filter(|r| r.day == day)
            .map(|r| r.name.as_str())
            .collect();
        placed.sort();
        let mut expected: Vec<&str> = kids.iter().map(|k| k.name.as_str()).collect();
        expected.sort();
        assert_eq!(placed, expected);
    }
    // stats fields are all present by construction (struct); spot-check status.
    assert!(matches!(stats.status_c.as_str(), "OPTIMAL" | "FEASIBLE"));
}

// ---------------------------------------------------------------------------
// TestSolveInvariants
// ---------------------------------------------------------------------------

#[test]
fn row_count() {
    let (rows, _) = run(&kids(), &cfg());
    assert_eq!(rows.len(), 8); // 4 kids x 2 days
}

#[test]
fn every_kid_placed_each_day() {
    let kids = kids();
    let cfg = cfg();
    let (rows, _) = run(&kids, &cfg);
    for day in cfg.days() {
        let mut placed: Vec<&str> = rows
            .iter()
            .filter(|r| r.day == day)
            .map(|r| r.name.as_str())
            .collect();
        placed.sort();
        let mut expected: Vec<&str> = kids.iter().map(|k| k.name.as_str()).collect();
        expected.sort();
        assert_eq!(placed, expected, "day {day}");
    }
}

#[test]
fn uniform_boat_capacity() {
    let kids = kids();
    let cfg = cfg();
    let (rows, _) = run(&kids, &cfg);
    let cap = kids.len() as i64 / cfg.boats;
    for day in cfg.days() {
        for boat in 1..=cfg.boats {
            let count = rows
                .iter()
                .filter(|r| r.day == day && r.boat == boat)
                .count() as i64;
            assert_eq!(count, cap);
        }
    }
}

#[test]
fn boat_numbers_in_range() {
    let cfg = cfg();
    let (rows, _) = run(&kids(), &cfg);
    let boats: std::collections::HashSet<i64> = rows.iter().map(|r| r.boat).collect();
    let expected: std::collections::HashSet<i64> = (1..=cfg.boats).collect();
    assert_eq!(boats, expected);
}

#[test]
fn days_in_output_match_config() {
    let cfg = cfg();
    let (rows, _) = run(&kids(), &cfg);
    let days: std::collections::HashSet<String> = rows.iter().map(|r| r.day.clone()).collect();
    let expected: std::collections::HashSet<String> = cfg.days().into_iter().collect();
    assert_eq!(days, expected);
}

#[test]
fn names_in_output_match_input() {
    let kids = kids();
    let (rows, _) = run(&kids, &cfg());
    let got: std::collections::HashSet<&str> = rows.iter().map(|r| r.name.as_str()).collect();
    let expected: std::collections::HashSet<&str> = kids.iter().map(|k| k.name.as_str()).collect();
    assert_eq!(got, expected);
}

#[test]
fn levels_in_output_match_input() {
    let kids = kids();
    let (rows, _) = run(&kids, &cfg());
    let got: std::collections::HashSet<&str> = rows.iter().map(|r| r.level.as_str()).collect();
    let expected: std::collections::HashSet<&str> = kids.iter().map(|k| k.level.as_str()).collect();
    assert_eq!(got, expected);
}

#[test]
fn stats_are_nonneg() {
    let (_, stats) = run(&kids(), &cfg());
    assert!(stats.mixed >= 0);
    assert!(stats.distinct >= 0);
    assert!(stats.repeats >= 0);
}

#[test]
fn status_c_is_optimal_or_feasible() {
    let (_, stats) = run(&kids(), &cfg());
    assert!(matches!(stats.status_c.as_str(), "OPTIMAL" | "FEASIBLE"));
}

// ---------------------------------------------------------------------------
// TestOnProgress
// ---------------------------------------------------------------------------

#[test]
fn on_progress_called_three_times_with_labels() {
    let mut labels: Vec<String> = Vec::new();
    {
        let mut cb = |l: &str, _s: &str, _v: i64| labels.push(l.to_string());
        solve(&kids(), &cfg(), Some(&mut cb)).unwrap();
    }
    assert_eq!(labels, svec(&["A mixed", "B diversity", "C repeats"]));
}

#[test]
fn on_progress_values_nonneg() {
    let mut vals: Vec<i64> = Vec::new();
    {
        let mut cb = |_l: &str, _s: &str, v: i64| vals.push(v);
        solve(&kids(), &cfg(), Some(&mut cb)).unwrap();
    }
    assert_eq!(vals.len(), 3);
    assert!(vals.iter().all(|&v| v >= 0));
}

#[test]
fn on_progress_statuses_are_known() {
    let mut statuses: Vec<String> = Vec::new();
    {
        let mut cb = |_l: &str, s: &str, _v: i64| statuses.push(s.to_string());
        solve(&kids(), &cfg(), Some(&mut cb)).unwrap();
    }
    assert_eq!(statuses.len(), 3);
    assert!(statuses
        .iter()
        .all(|s| matches!(s.as_str(), "OPTIMAL" | "FEASIBLE")));
}

// ---------------------------------------------------------------------------
// TestSolveConstraints
// ---------------------------------------------------------------------------

#[test]
fn single_level_never_mixed() {
    let kids = roster_from_counts(&[("k", 4)]);
    let cfg = Config {
        boats: 2,
        levels: svec(&["k"]),
        restricted: svec(&["Sat"]),
        diverse: svec(&["Fri"]),
        stage_seconds: 10.0,
        seconds: 10.0,
        ..Default::default()
    };
    let (_, stats) = run(&kids, &cfg);
    assert_eq!(stats.mixed, 0);
}

#[test]
fn adj0_homogeneous_restricted_boats() {
    // With adj=0 and balanced counts, restricted boats must be pure level.
    let kids = roster_from_counts(&[("k", 4), ("b", 4)]);
    let cfg = Config {
        boats: 4,
        levels: svec(&["k", "b"]),
        adj: 0,
        restricted: svec(&["Sat"]),
        diverse: vec![],
        stage_seconds: 15.0,
        seconds: 15.0,
        ..Default::default()
    };
    let (rows, stats) = run(&kids, &cfg);
    assert_eq!(stats.mixed, 0);
    for boat in 1..=4 {
        let lvls: std::collections::HashSet<&str> = rows
            .iter()
            .filter(|r| r.day == "Sat" && r.boat == boat)
            .map(|r| r.level.as_str())
            .collect();
        assert_eq!(lvls.len(), 1, "boat {boat} mixed levels: {lvls:?}");
    }
}

#[test]
fn adj1_blocks_nonadjacent_levels() {
    // 6 kids (2k, 2b, 2A), 3 boats, cap=2; k-b adjacent, b-A adjacent, k-A forbidden.
    let kids = roster_from_counts(&[("k", 2), ("b", 2), ("A", 2)]);
    let cfg = Config {
        boats: 3,
        levels: svec(&["k", "b", "A"]),
        adj: 1,
        restricted: svec(&["Sat"]),
        diverse: vec![],
        stage_seconds: 15.0,
        seconds: 15.0,
        ..Default::default()
    };
    let level_idx = |l: &str| match l {
        "k" => 0,
        "b" => 1,
        "A" => 2,
        _ => unreachable!(),
    };
    let (rows, _) = run(&kids, &cfg);
    for boat in 1..=3 {
        let idxs: std::collections::BTreeSet<i32> = rows
            .iter()
            .filter(|r| r.day == "Sat" && r.boat == boat)
            .map(|r| level_idx(r.level.as_str()))
            .collect();
        if idxs.len() > 1 {
            let span = idxs.iter().max().unwrap() - idxs.iter().min().unwrap();
            assert!(span <= 1, "boat {boat} violates adj=1: levels {idxs:?}");
        }
    }
}

#[test]
fn single_day_zero_repeats() {
    // One day means no pair can share a boat twice, so repeats must be 0.
    let kids = roster_from_counts(&[("k", 2), ("b", 2)]);
    let cfg = Config {
        boats: 2,
        levels: svec(&["k", "b"]),
        restricted: svec(&["Sat"]),
        diverse: vec![],
        stage_seconds: 10.0,
        seconds: 10.0,
        ..Default::default()
    };
    let (_, stats) = run(&kids, &cfg);
    assert_eq!(stats.repeats, 0);
}

#[test]
fn repeats_bounded_by_days() {
    // Total repeats can't exceed pairs x (days - 1).
    let kids = kids();
    let cfg = cfg(); // 2 days
    let n = kids.len() as i64;
    let max_repeats = (n * (n - 1) / 2) * (cfg.days().len() as i64 - 1);
    let (_, stats) = run(&kids, &cfg);
    assert!(stats.repeats <= max_repeats);
}

// ---------------------------------------------------------------------------
// Golden objective-value parity vs the Python solver.
//
// These assert the *exact* objective values captured from the Python
// implementation (`rust/bench/py_bench.py`). They are safe to pin because the
// optimum *value* of an objective is mathematically unique even though CP-SAT
// is nondeterministic about *which* optimal assignment it returns — so any
// case that solves to OPTIMAL has a reproducible (mixed, distinct, repeats)
// triple. Cases that only reach FEASIBLE within the time budget are NOT pinned
// (their incumbent depends on the search and differs run-to-run); see the
// REWRITE report for the full-roster divergence.
// ---------------------------------------------------------------------------

#[test]
fn golden_tiny_objectives_match_python() {
    let (_, stats) = run(&kids(), &cfg());
    assert_eq!(stats.mixed, 0);
    assert_eq!(stats.distinct, 4);
    assert_eq!(stats.repeats, 0);
    assert_eq!(stats.status_c, "OPTIMAL");
}

#[test]
fn golden_mid_16_objectives_match_python() {
    let kids = roster_from_counts(&[("k", 4), ("b", 4), ("A", 4), ("B", 4)]);
    let cfg = Config {
        boats: 4,
        levels: svec(&["k", "b", "A", "B"]),
        restricted: svec(&["Sat", "Sun", "Tue", "Wed"]),
        diverse: svec(&["Fri", "Mon"]),
        stage_seconds: 20.0,
        seconds: 30.0,
        ..Default::default()
    };
    let (_, stats) = run(&kids, &cfg);
    // Captured from Python: OPTIMAL across all three stages.
    assert_eq!(stats.mixed, 0);
    assert_eq!(stats.distinct, 32);
    assert_eq!(stats.repeats, 72);
    assert_eq!(stats.status_c, "OPTIMAL");
}

// ---------------------------------------------------------------------------
// Validation passthrough — solve() surfaces ValidationError as
// SolveFailure::Validation before touching the solver (mirrors build_model's
// validate() call).
// ---------------------------------------------------------------------------

#[test]
fn indivisible_roster_is_validation_error() {
    let kids = roster_from_counts(&[("k", 5)]);
    let cfg = Config {
        boats: 2,
        levels: svec(&["k"]),
        restricted: svec(&["Sat"]),
        diverse: vec![],
        ..Default::default()
    };
    match solve(&kids, &cfg, None) {
        Err(SolveFailure::Validation(_)) => {}
        other => panic!("expected Validation error, got {other:?}"),
    }
}

#[test]
fn unknown_level_is_validation_error() {
    let kids = vec![Kid::new("A", "z")];
    let cfg = Config {
        boats: 1,
        levels: svec(&["k"]),
        restricted: svec(&["Sat"]),
        diverse: vec![],
        ..Default::default()
    };
    match solve(&kids, &cfg, None) {
        Err(SolveFailure::Validation(_)) => {}
        other => panic!("expected Validation error, got {other:?}"),
    }
}
