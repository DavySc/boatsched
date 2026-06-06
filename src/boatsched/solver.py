"""Core constraint model and lexicographic staged solver.

Pure library code: no argparse, no sys.exit, no printing to stdout. Failures
are raised as exceptions. Progress is reported through an optional callback.
"""
from __future__ import annotations
import sys
from dataclasses import dataclass, field
from itertools import combinations
from typing import Callable, Dict, List, Optional, Sequence, Tuple

from ortools.sat.python import cp_model

Kid = Tuple[str, str]
ScheduleRow = Tuple[str, int, str, str]   # (day, boat, name, level)


class SolveError(RuntimeError):
    """A lexicographic stage found no solution within its time budget."""


class ValidationError(ValueError):
    """The roster/config combination is structurally infeasible."""


@dataclass
class Config:
    """Problem and solver configuration."""
    boats: int = 13
    levels: Sequence[str] = ("k", "b", "A", "B")     # ordered low -> high
    restricted: Sequence[str] = ("Sat", "Sun", "Tue", "Wed")
    diverse: Sequence[str] = ("Fri", "Mon")
    adj: int = 1                                       # max level spread on restricted days
    stage_seconds: float = 45.0                        # budget for stages A and B each
    seconds: float = 60.0                              # budget for stage C
    workers: int = 8

    @property
    def days(self) -> List[str]:
        return list(self.restricted) + list(self.diverse)


def validate(kids: List[Kid], cfg: Config) -> None:
    if not cfg.days:
        raise ValidationError("need at least one restricted or diverse day")
    if cfg.boats <= 0:
        raise ValidationError("boats must be positive")
    if len(kids) % cfg.boats != 0:
        raise ValidationError(
            f"{len(kids)} kids not divisible by {cfg.boats} boats")
    known = set(cfg.levels)
    bad = sorted({l for _, l in kids if l not in known})
    if bad:
        raise ValidationError(
            f"unknown level(s) {bad}; known levels are {list(cfg.levels)}")


def build_model(kids: List[Kid], cfg: Config) -> dict:
    """Construct the CP-SAT model and the three objective variables."""
    validate(kids, cfg)
    levels = list(cfg.levels)
    level_idx = {l: i for i, l in enumerate(levels)}
    days = cfg.days
    restricted, diverse = set(cfg.restricted), set(cfg.diverse)
    N, n_boats, D = len(kids), cfg.boats, len(cfg.days)
    cap = N // n_boats
    lvl_of = [level_idx[l] for _, l in kids]

    m = cp_model.CpModel()
    x = {(k, d, b): m.NewBoolVar(f"x_{k}_{d}_{b}")
         for k in range(N) for d in range(D) for b in range(n_boats)}

    for k in range(N):
        for d in range(D):
            m.Add(sum(x[k, d, b] for b in range(n_boats)) == 1)
    for d in range(D):
        for b in range(n_boats):
            m.Add(sum(x[k, d, b] for k in range(N)) == cap)

    def presence(d, b, li):
        p = m.NewBoolVar(f"pres_{d}_{b}_{li}")
        mem = [x[k, d, b] for k in range(N) if lvl_of[k] == li]
        if mem:
            m.AddMaxEquality(p, mem)
        else:
            m.Add(p == 0)
        return p

    # A: minimize mixed boats on restricted days (+ adjacency hard constraint)
    mixed_terms = []
    for d, day in enumerate(days):
        if day not in restricted:
            continue
        for b in range(n_boats):
            pres = {li: presence(d, b, li) for li in range(len(levels))}
            for a, bb in combinations(range(len(levels)), 2):
                if bb - a > cfg.adj:
                    m.Add(pres[a] + pres[bb] <= 1)
            mixed = m.NewBoolVar(f"mixed_{d}_{b}")
            nlev = sum(pres.values())
            m.Add(nlev >= 2).OnlyEnforceIf(mixed)
            m.Add(nlev <= 1).OnlyEnforceIf(mixed.Not())
            mixed_terms.append(mixed)
    total_mixed = m.NewIntVar(0, max(1, len(mixed_terms)), "total_mixed")
    m.Add(total_mixed == (sum(mixed_terms) if mixed_terms else 0))

    # B: maximize distinct levels on diverse days
    diverse_terms = []
    for d, day in enumerate(days):
        if day not in diverse:
            continue
        for b in range(n_boats):
            for li in range(len(levels)):
                diverse_terms.append(presence(d, b, li))
    total_distinct = m.NewIntVar(0, max(1, len(diverse_terms)), "total_distinct")
    m.Add(total_distinct == (sum(diverse_terms) if diverse_terms else 0))

    # C: minimize repeat co-boatings
    pair_rep = {}
    for k1, k2 in combinations(range(N), 2):
        meets = []
        for d in range(D):
            sb = []
            for b in range(n_boats):
                s = m.NewBoolVar(f"sb_{k1}_{k2}_{d}_{b}")
                m.AddMultiplicationEquality(s, [x[k1, d, b], x[k2, d, b]])
                sb.append(s)
            md = m.NewBoolVar(f"meet_{k1}_{k2}_{d}")
            m.AddMaxEquality(md, sb)
            meets.append(md)
        rep = m.NewIntVar(0, D, f"rep_{k1}_{k2}")
        m.Add(rep >= sum(meets) - 1)
        pair_rep[k1, k2] = rep
    total_repeats = m.NewIntVar(0, N * D, "total_repeats")
    m.Add(total_repeats == sum(pair_rep.values()))

    # Symmetry-breaking note: boat labels are interchangeable, but the obvious
    # "pin kid k to boat <= k" anchor conflicts with the level constraints, so
    # it is intentionally omitted. A and B still close fast; C is the hard one.

    return dict(model=m, x=x, N=N, D=D, cap=cap, n_boats=n_boats,
                days=days, kids=kids,
                total_mixed=total_mixed, total_distinct=total_distinct,
                total_repeats=total_repeats)


def staged_solve(P: dict, cfg: Config,
                 on_progress: Optional[Callable[[str, str, int], None]] = None):
    """Solve lexicographically A > B > C, locking each stage before the next.

    on_progress(label, status_name, value) is called after each stage.
    Returns (solver, stats_dict).
    """
    m = P["model"]
    A, B, C = P["total_mixed"], P["total_distinct"], P["total_repeats"]

    def run(obj, sense, secs, label):
        m.ClearObjective()
        m.Minimize(obj) if sense == "min" else m.Maximize(obj)
        s = cp_model.CpSolver()
        s.parameters.max_time_in_seconds = secs
        s.parameters.num_search_workers = cfg.workers
        st = s.Solve(m)
        if st not in (cp_model.OPTIMAL, cp_model.FEASIBLE):
            raise SolveError(
                f"stage '{label}' failed with status {s.StatusName(st)}; "
                f"increase the time budget or relax inputs")
        v = s.Value(obj)
        if on_progress:
            on_progress(label, s.StatusName(st), int(v))
        return s, st, v

    _, _, va = run(A, "min", cfg.stage_seconds, "A mixed")
    m.Add(A <= int(va))
    _, _, vb = run(B, "max", cfg.stage_seconds, "B diversity")
    m.Add(B >= int(vb))
    sC, stC, vc = run(C, "min", cfg.seconds, "C repeats")

    return sC, dict(mixed=int(va), distinct=int(vb), repeats=int(vc),
                    status_C=sC.StatusName(stC))


def extract_schedule(solver, P: dict) -> List[ScheduleRow]:
    x, N, D, n_boats = P["x"], P["N"], P["D"], P["n_boats"]
    days, kids = P["days"], P["kids"]
    rows: List[ScheduleRow] = []
    for d in range(D):
        for b in range(n_boats):
            for k in range(N):
                if solver.Value(x[k, d, b]):
                    rows.append((days[d], b + 1, kids[k][0], kids[k][1]))
    return rows


def solve(kids: List[Kid], cfg: Config,
          on_progress: Optional[Callable[[str, str, int], None]] = None
          ) -> Tuple[List[ScheduleRow], dict]:
    """End-to-end: build, staged-solve, extract. Returns (rows, stats)."""
    P = build_model(kids, cfg)
    solver, stats = staged_solve(P, cfg, on_progress=on_progress)
    return extract_schedule(solver, P), stats
