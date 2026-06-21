#!/usr/bin/env python3
"""Read-only CRDT benchmark for the *Python* (pycrdt) collab layer.

Mirrors `crates/server/examples/crdt_bench.rs`: builds a realistic schedule doc
(13 boats x 6 days x 4 kids = 312 name entries / 78 keys), then times the three
hot operations the collab/commit paths exercise:

  build   — group rows + build the Y.Doc + encode the full v1 update
  apply   — decode + apply that update into a fresh doc (what every client does)
  scan    — read the schedule map + conflict-scan (the commit endpoint)

Prints one JSON object per op. Does not modify the implementation.
"""
import json
import time

import pycrdt

BOATS, DAYS, CAP = 13, ["Sat", "Sun", "Tue", "Wed", "Fri", "Mon"], 4
ITERS = 2000


def rows():
    out = []
    kid = 0
    for day in DAYS:
        for boat in range(1, BOATS + 1):
            for _ in range(CAP):
                out.append((day, boat, f"kid{kid % 52}"))
                kid += 1
    return out


def build_update(rs):
    groups = {}
    for day, boat, name in rs:
        groups.setdefault(f"{day}:{boat}", []).append(name)
    doc = pycrdt.Doc()
    sch = doc.get("schedule", type=pycrdt.Map)
    with doc.transaction():
        for key, names in sorted(groups.items()):
            sch[key] = pycrdt.Array(names)
    return doc.get_update()


def time_op(label, fn, iters):
    # warmup
    fn()
    t0 = time.perf_counter()
    for _ in range(iters):
        fn()
    elapsed = time.perf_counter() - t0
    print(json.dumps({
        "op": label, "impl": "python", "iters": iters,
        "total_ms": round(elapsed * 1000, 2),
        "per_op_us": round(elapsed / iters * 1e6, 2),
    }))


def main():
    rs = rows()
    update = build_update(rs)

    time_op("build", lambda: build_update(rs), ITERS)

    def apply():
        d = pycrdt.Doc()
        d.apply_update(update)

    time_op("apply", apply, ITERS)

    def scan():
        d = pycrdt.Doc()
        d.apply_update(update)
        sch = d.get("schedule", type=pycrdt.Map)
        seen = {}
        conflicts = 0
        for key, members in sch.items():
            colon = key.find(":")
            day, boat = key[:colon], int(key[colon + 1:])
            for name in members:
                k = (name, day)
                if k in seen:
                    conflicts += 1
                else:
                    seen[k] = boat
        return conflicts

    time_op("scan", scan, ITERS)


if __name__ == "__main__":
    main()
