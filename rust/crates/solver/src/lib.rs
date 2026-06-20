//! Lexicographic staged CP-SAT solver — Rust front end over the OR-Tools C++
//! shim (`cpp/cpsat_shim.cpp`). Mirrors `solver.py`'s public surface:
//! [`solve`], [`Stats`], and re-exports [`Config`]/[`validate`] from
//! `boatsched-core`.
//!
//! The FFI boundary is intentionally tiny and flat (primitive arrays); all
//! modelling lives in the C++ shim so the OR-Tools dependency stays isolated to
//! this crate.

pub use boatsched_core::{validate, Config, Kid, ScheduleRow, SolveError, ValidationError};

use std::os::raw::{c_char, c_int};

extern "C" {
    fn bs_cpsat_solve(
        n_kids: i32,
        lvl_of: *const i32,
        n_levels: i32,
        n_boats: i32,
        n_days: i32,
        day_restricted: *const i32,
        day_diverse: *const i32,
        adj: i32,
        stage_seconds: f64,
        seconds: f64,
        workers: i32,
        prefixing: i32,
        prune_pairs: i32,
        out_assign: *mut i32,
        out_status: *mut i32,
        out_vals: *mut i64,
        out_err: *mut c_char,
        err_cap: i32,
    ) -> c_int;
}

/// Solver result statistics, mirroring the Python `stats` dict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stats {
    pub mixed: i64,
    pub distinct: i64,
    pub repeats: i64,
    pub status_c: String,
}

/// End-to-end failure: a validation error or a solve-stage failure. Mirrors the
/// two Python exception types that `solve` can raise.
#[derive(Debug)]
pub enum SolveFailure {
    Validation(ValidationError),
    Solve(SolveError),
}

impl std::fmt::Display for SolveFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SolveFailure::Validation(e) => write!(f, "{e}"),
            SolveFailure::Solve(e) => write!(f, "{e}"),
        }
    }
}
impl std::error::Error for SolveFailure {}

/// Cp-SAT status code → name, matching `CpSolver.StatusName` in Python.
fn status_name(code: i32) -> &'static str {
    match code {
        0 => "UNKNOWN",
        1 => "MODEL_INVALID",
        2 => "FEASIBLE",
        3 => "INFEASIBLE",
        4 => "OPTIMAL",
        _ => "UNKNOWN",
    }
}

fn is_ok(code: i32) -> bool {
    code == 2 || code == 4 // FEASIBLE | OPTIMAL
}

/// The three stage labels, identical to the Python `on_progress` labels.
const STAGE_LABELS: [&str; 3] = ["A mixed", "B diversity", "C repeats"];

/// Build + staged-solve + extract. Returns `(rows, stats)`. `on_progress`
/// receives `(label, status_name, value)` after each successful stage, exactly
/// like the Python callback.
pub fn solve(
    kids: &[Kid],
    cfg: &Config,
    mut on_progress: Option<&mut dyn FnMut(&str, &str, i64)>,
) -> Result<(Vec<ScheduleRow>, Stats), SolveFailure> {
    // validate() first, mirroring build_model.
    validate(kids, cfg).map_err(SolveFailure::Validation)?;

    let days = cfg.days();
    let d = days.len();
    let n = kids.len();
    let n_boats = cfg.boats; // > 0, validated

    // Level index per kid (position in cfg.levels).
    let level_idx: std::collections::HashMap<&str, i32> = cfg
        .levels
        .iter()
        .enumerate()
        .map(|(i, l)| (l.as_str(), i as i32))
        .collect();
    let lvl_of: Vec<i32> = kids.iter().map(|k| level_idx[k.level.as_str()]).collect();

    let restricted: std::collections::HashSet<&str> =
        cfg.restricted.iter().map(String::as_str).collect();
    let diverse: std::collections::HashSet<&str> = cfg.diverse.iter().map(String::as_str).collect();
    let day_restricted: Vec<i32> = days
        .iter()
        .map(|d| restricted.contains(d.as_str()) as i32)
        .collect();
    let day_diverse: Vec<i32> = days
        .iter()
        .map(|d| diverse.contains(d.as_str()) as i32)
        .collect();

    let mut assign = vec![-1i32; n * d];
    let mut status = [-1i32; 3];
    let mut vals = [0i64; 3];
    let mut err = vec![0 as c_char; 256];

    let rc = unsafe {
        bs_cpsat_solve(
            n as i32,
            lvl_of.as_ptr(),
            cfg.levels.len() as i32,
            n_boats as i32,
            d as i32,
            day_restricted.as_ptr(),
            day_diverse.as_ptr(),
            cfg.adj as i32,
            cfg.stage_seconds,
            cfg.seconds,
            cfg.workers as i32,
            1, // prefixing
            1, // prune_pairs
            assign.as_mut_ptr(),
            status.as_mut_ptr(),
            vals.as_mut_ptr(),
            err.as_mut_ptr(),
            err.len() as i32,
        )
    };

    if rc == 2 {
        let msg = unsafe { std::ffi::CStr::from_ptr(err.as_ptr()) }
            .to_string_lossy()
            .into_owned();
        return Err(SolveFailure::Solve(SolveError(msg)));
    }

    // Emit progress for successful stages in order; raise on the first failure.
    for i in 0..3 {
        if status[i] < 0 {
            // Not run: only happens after an earlier failure, already handled.
            break;
        }
        if !is_ok(status[i]) {
            return Err(SolveFailure::Solve(SolveError(format!(
                "stage '{}' failed with status {}; increase the time budget or relax inputs",
                STAGE_LABELS[i],
                status_name(status[i])
            ))));
        }
        if let Some(cb) = on_progress.as_deref_mut() {
            cb(STAGE_LABELS[i], status_name(status[i]), vals[i]);
        }
    }

    // Extract in the same (day, boat, kid) order as Python extract_schedule.
    let mut rows = Vec::with_capacity(n * d);
    for di in 0..d {
        for b in 0..(n_boats as usize) {
            for k in 0..n {
                if assign[k * d + di] == b as i32 {
                    rows.push(ScheduleRow {
                        day: days[di].clone(),
                        boat: (b as i64) + 1,
                        name: kids[k].name.clone(),
                        level: kids[k].level.clone(),
                    });
                }
            }
        }
    }

    let stats = Stats {
        mixed: vals[0],
        distinct: vals[1],
        repeats: vals[2],
        status_c: status_name(status[2]).to_string(),
    };
    Ok((rows, stats))
}
