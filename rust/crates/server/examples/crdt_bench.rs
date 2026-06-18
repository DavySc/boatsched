//! CRDT benchmark for the *Rust* (yrs) collab layer — the counterpart of
//! `rust/bench/py_crdt_bench.py`. Same realistic schedule (13 boats x 6 days x
//! 4 kids), same three hot ops (build / apply / scan), JSON-per-op output.
//!
//! Run: `cargo run -p boatsched-server --release --example crdt_bench`

use std::time::Instant;

use boatsched_core::ScheduleRow;
use boatsched_server::collab;
use yrs::updates::decoder::Decode;
use yrs::{Doc, Transact, Update};

const BOATS: i64 = 13;
const CAP: usize = 4;
const ITERS: u32 = 2000;
const DAYS: [&str; 6] = ["Sat", "Sun", "Tue", "Wed", "Fri", "Mon"];

fn rows() -> Vec<ScheduleRow> {
    let mut out = Vec::new();
    let mut kid = 0usize;
    for day in DAYS {
        for boat in 1..=BOATS {
            for _ in 0..CAP {
                out.push(ScheduleRow {
                    day: day.into(),
                    boat,
                    name: format!("kid{}", kid % 52),
                    level: "k".into(),
                });
                kid += 1;
            }
        }
    }
    out
}

fn time_op(label: &str, iters: u32, mut f: impl FnMut()) {
    f(); // warmup
    let t0 = Instant::now();
    for _ in 0..iters {
        f();
    }
    let elapsed = t0.elapsed().as_secs_f64();
    println!(
        "{{\"op\": \"{label}\", \"impl\": \"rust\", \"iters\": {iters}, \"total_ms\": {:.2}, \"per_op_us\": {:.2}}}",
        elapsed * 1000.0,
        elapsed / iters as f64 * 1e6,
    );
}

fn main() {
    let rs = rows();
    let update = collab::init_doc_update_from_rows(&rs);

    time_op("build", ITERS, || {
        let _ = collab::init_doc_update_from_rows(&rs);
    });

    time_op("apply", ITERS, || {
        let doc = Doc::new();
        doc.get_or_insert_map("schedule");
        let mut txn = doc.transact_mut();
        txn.apply_update(Update::decode_v1(&update).unwrap()).unwrap();
    });

    time_op("scan", ITERS, || {
        let doc = Doc::new();
        doc.get_or_insert_map("schedule");
        collab::apply_update(&doc, &update);
        let _ = collab::read_assignments(&doc);
    });
}
