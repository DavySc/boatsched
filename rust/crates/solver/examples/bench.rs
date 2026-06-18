//! Benchmark harness for the *Rust* boatsched solver — the byte-identical
//! counterpart of `rust/bench/py_bench.py`. Runs the same fixed cases through
//! [`boatsched_solver::solve`] and prints one JSON object per case (stage
//! statuses, the three objective values, wall-clock solve time) so the two
//! harnesses can be diffed for objective parity and timing.
//!
//! Run: `cargo run -p boatsched-solver --release --example bench -- [roster.csv]`

use std::time::Instant;

use boatsched_core::{svec, Config, Kid};
use boatsched_roster::{load_roster, roster_from_counts};
use boatsched_solver::solve;

fn case_defs(roster_path: Option<&str>) -> Vec<(&'static str, Vec<Kid>, Config)> {
    let mut v: Vec<(&'static str, Vec<Kid>, Config)> = vec![
        (
            "tiny_2k2b",
            roster_from_counts(&[("k", 2), ("b", 2)]),
            Config {
                boats: 2,
                levels: svec(&["k", "b"]),
                restricted: svec(&["Sat"]),
                diverse: svec(&["Fri"]),
                stage_seconds: 10.0,
                seconds: 10.0,
                ..Default::default()
            },
        ),
        (
            "adj0_4k4b",
            roster_from_counts(&[("k", 4), ("b", 4)]),
            Config {
                boats: 4,
                levels: svec(&["k", "b"]),
                adj: 0,
                restricted: svec(&["Sat"]),
                diverse: vec![],
                stage_seconds: 15.0,
                seconds: 15.0,
                ..Default::default()
            },
        ),
        (
            "mid_16_4lvl",
            roster_from_counts(&[("k", 4), ("b", 4), ("A", 4), ("B", 4)]),
            Config {
                boats: 4,
                levels: svec(&["k", "b", "A", "B"]),
                restricted: svec(&["Sat", "Sun", "Tue", "Wed"]),
                diverse: svec(&["Fri", "Mon"]),
                stage_seconds: 20.0,
                seconds: 30.0,
                ..Default::default()
            },
        ),
    ];
    if let Some(path) = roster_path {
        let roster = load_roster(path).expect("load roster");
        v.push((
            "full_roster52",
            roster,
            Config {
                boats: 13,
                restricted: svec(&["Sat", "Sun", "Tue", "Wed"]),
                diverse: svec(&["Fri", "Mon"]),
                stage_seconds: 20.0,
                seconds: 60.0,
                ..Default::default()
            },
        ));
    }
    v
}

fn main() {
    let roster_path = std::env::args().nth(1);
    for (name, kids, cfg) in case_defs(roster_path.as_deref()) {
        let mut statuses: Vec<String> = Vec::new();
        let t0 = Instant::now();
        let (_, stats) = {
            let mut cb = |_l: &str, s: &str, _v: i64| statuses.push(s.to_string());
            solve(&kids, &cfg, Some(&mut cb)).expect("solve")
        };
        let elapsed = t0.elapsed().as_secs_f64();
        let statuses_json = statuses
            .iter()
            .map(|s| format!("\"{s}\""))
            .collect::<Vec<_>>()
            .join(", ");
        println!(
            "{{\"case\": \"{}\", \"impl\": \"rust\", \"mixed\": {}, \"distinct\": {}, \"repeats\": {}, \"status_C\": \"{}\", \"stage_statuses\": [{}], \"elapsed_s\": {:.4}}}",
            name, stats.mixed, stats.distinct, stats.repeats, stats.status_c, statuses_json, elapsed
        );
    }
}
