#!/usr/bin/env python3
"""Read-only benchmark harness for the *Python* boatsched solver.

Runs a fixed set of representative cases through `boatsched.solve()` and prints
one JSON object per case: stage statuses, the three objective values, and the
wall-clock solve time. The Rust harness (`crates/solver/examples/bench.rs`)
runs the byte-identical cases so the two outputs can be diffed for objective
parity and timing comparison.

This script imports the production solver unchanged — it does NOT modify it.
"""
import json
import sys
import time

from boatsched import Config, solve
from boatsched.roster import load_roster, roster_from_counts

# Identical case definitions in both harnesses. Keep in sync with bench.rs.
def cases():
    yield (
        "tiny_2k2b",
        roster_from_counts({"k": 2, "b": 2}),
        Config(boats=2, levels=["k", "b"], restricted=["Sat"], diverse=["Fri"],
               stage_seconds=10, seconds=10),
    )
    yield (
        "adj0_4k4b",
        roster_from_counts({"k": 4, "b": 4}),
        Config(boats=4, levels=["k", "b"], adj=0, restricted=["Sat"], diverse=[],
               stage_seconds=15, seconds=15),
    )
    yield (
        "mid_16_4lvl",
        roster_from_counts({"k": 4, "b": 4, "A": 4, "B": 4}),
        Config(boats=4, levels=["k", "b", "A", "B"],
               restricted=["Sat", "Sun", "Tue", "Wed"], diverse=["Fri", "Mon"],
               stage_seconds=20, seconds=30),
    )
    # Realistic production case: the example roster, default-shaped config.
    roster = load_roster(sys.argv[1]) if len(sys.argv) > 1 else None
    if roster is not None:
        yield (
            "full_roster52",
            roster,
            Config(boats=13, restricted=["Sat", "Sun", "Tue", "Wed"],
                   diverse=["Fri", "Mon"], stage_seconds=20, seconds=60),
        )


def main():
    for name, kids, cfg in cases():
        statuses = []
        on_progress = lambda l, s, v: statuses.append(s)  # noqa: E731
        t0 = time.perf_counter()
        _, stats = solve(kids, cfg, on_progress=on_progress)
        elapsed = time.perf_counter() - t0
        print(json.dumps({
            "case": name,
            "impl": "python",
            "mixed": stats["mixed"],
            "distinct": stats["distinct"],
            "repeats": stats["repeats"],
            "status_C": stats["status_C"],
            "stage_statuses": statuses,
            "elapsed_s": round(elapsed, 4),
        }))


if __name__ == "__main__":
    main()
