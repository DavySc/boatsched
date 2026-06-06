"""Command-line interface for boatsched."""
from __future__ import annotations
import argparse
import csv
import sys

from .solver import Config, SolveError, ValidationError, solve
from .roster import load_roster, roster_from_counts, RosterError


def parse_args(argv=None):
    p = argparse.ArgumentParser(
        prog="boatsched",
        description="Boat assignment optimizer (lexicographic A>B>C).")
    src = p.add_mutually_exclusive_group(required=True)
    src.add_argument("--roster", help="CSV with columns name,level")
    src.add_argument("--counts", nargs="+", metavar="LEVEL=N",
                     help="synthetic roster, e.g. k=12 b=16 A=12 B=12")
    p.add_argument("--boats", type=int, required=True)
    p.add_argument("--levels", nargs="+", default=["k", "b", "A", "B"],
                   help="levels low->high (default: k b A B)")
    p.add_argument("--restricted", nargs="*", default=["Sat", "Sun", "Tue", "Wed"])
    p.add_argument("--diverse", nargs="*", default=["Fri", "Mon"])
    p.add_argument("--adj", type=int, default=1,
                   help="max level-index spread on restricted days (default 1)")
    p.add_argument("--seconds", type=float, default=60,
                   help="time budget for the repeat-minimization stage")
    p.add_argument("--stage-seconds", type=float, default=45,
                   help="time budget for stages A and B each")
    p.add_argument("--workers", type=int, default=8)
    p.add_argument("--out", help="write schedule CSV here (else print)")
    return p.parse_args(argv)


def main(argv=None) -> int:
    args = parse_args(argv)
    try:
        if args.roster:
            kids = load_roster(args.roster)
        else:
            counts = {}
            for tok in args.counts:
                lvl, n = tok.split("=")
                counts[lvl] = int(n)
            kids = roster_from_counts(counts)
    except (RosterError, ValueError) as e:
        print(f"error: {e}", file=sys.stderr)
        return 2

    cfg = Config(boats=args.boats, levels=args.levels,
                 restricted=args.restricted, diverse=args.diverse,
                 adj=args.adj, stage_seconds=args.stage_seconds,
                 seconds=args.seconds, workers=args.workers)

    def progress(label, status, value):
        print(f"[{label}] {status} value={value}", file=sys.stderr)

    try:
        rows, res = solve(kids, cfg, on_progress=progress)
    except (SolveError, ValidationError) as e:
        print(f"error: {e}", file=sys.stderr)
        return 1

    print("\n=== RESULT (lexicographic A>B>C) ===")
    print(f"mixed boats (restricted) : {res['mixed']}")
    print(f"distinct-level total     : {res['distinct']}")
    print(f"repeat co-boatings       : {res['repeats']} ({res['status_C']})")

    if args.out:
        with open(args.out, "w", newline="") as f:
            w = csv.writer(f)
            w.writerow(["day", "boat", "name", "level"])
            w.writerows(rows)
        print(f"schedule written to {args.out}")
    else:
        cur = None
        for day, boat, name, lvl in rows:
            if day != cur:
                print(f"\n-- {day} --"); cur = day
            print(f"  boat {boat:>2}: {name} ({lvl})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
