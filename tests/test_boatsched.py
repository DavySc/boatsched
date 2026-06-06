"""Fast tests: roster parsing, validation, and a tiny end-to-end solve."""
import pytest

from boatsched import Config, ValidationError, solve
from boatsched.roster import (
    parse_roster_bytes, roster_from_counts, RosterError,
)


def test_roster_from_counts():
    kids = roster_from_counts({"k": 2, "b": 2})
    assert len(kids) == 4
    assert {l for _, l in kids} == {"k", "b"}


def test_parse_bytes_requires_columns():
    with pytest.raises(RosterError):
        parse_roster_bytes(b"foo,bar\n1,2\n")


def test_parse_bytes_ok():
    kids = parse_roster_bytes(b"name,level\nAlice,k\nBob,b\n")
    assert kids == [("Alice", "k"), ("Bob", "b")]


def test_validation_divisibility():
    kids = roster_from_counts({"k": 5})
    with pytest.raises(ValidationError):
        solve(kids, Config(boats=2, levels=["k"]))


def test_validation_unknown_level():
    kids = [("A", "z"), ("B", "z")]
    with pytest.raises(ValidationError):
        solve(kids, Config(boats=1, levels=["k", "b"]))


def test_tiny_solve():
    # 4 kids, 2 boats, 2 levels, 1 restricted + 1 diverse day. Small enough
    # to solve in well under a second.
    kids = roster_from_counts({"k": 2, "b": 2})
    cfg = Config(boats=2, levels=["k", "b"],
                 restricted=["Sat"], diverse=["Fri"],
                 stage_seconds=10, seconds=10)
    rows, stats = solve(kids, cfg)
    # 4 kids x 2 days = 8 assignment rows
    assert len(rows) == 8
    # every kid placed exactly once per day
    for day in ("Sat", "Fri"):
        names = [n for (d, b, n, l) in rows if d == day]
        assert sorted(names) == sorted(n for n, _ in kids)
    assert set(stats) >= {"mixed", "distinct", "repeats"}
