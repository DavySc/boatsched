"""Expanded tests for boatsched.solver: validation, Config, and solve invariants."""
import pytest

from boatsched import Config, ValidationError, solve
from boatsched.roster import roster_from_counts
from boatsched.solver import validate


# ---------------------------------------------------------------------------
# validate()
# ---------------------------------------------------------------------------

class TestValidate:
    def test_no_days_raises(self):
        kids = roster_from_counts({"k": 2})
        with pytest.raises(ValidationError, match="day"):
            validate(kids, Config(boats=1, levels=["k"], restricted=[], diverse=[]))

    def test_boats_zero_raises(self):
        kids = roster_from_counts({"k": 2})
        with pytest.raises(ValidationError, match="positive"):
            validate(kids, Config(boats=0, levels=["k"]))

    def test_boats_negative_raises(self):
        kids = roster_from_counts({"k": 2})
        with pytest.raises(ValidationError):
            validate(kids, Config(boats=-1, levels=["k"]))

    def test_indivisible_raises(self):
        kids = roster_from_counts({"k": 5})
        with pytest.raises(ValidationError):
            validate(kids, Config(boats=2, levels=["k"]))

    def test_unknown_level_raises(self):
        kids = [("A", "z")]
        with pytest.raises(ValidationError, match="unknown"):
            validate(kids, Config(boats=1, levels=["k"]))

    def test_valid_passes_no_exception(self):
        kids = roster_from_counts({"k": 4, "b": 4})
        validate(kids, Config(boats=4, levels=["k", "b"]))


# ---------------------------------------------------------------------------
# Config.days
# ---------------------------------------------------------------------------

class TestConfigDays:
    def test_restricted_before_diverse(self):
        cfg = Config(restricted=["Sat", "Sun"], diverse=["Fri"])
        assert cfg.days == ["Sat", "Sun", "Fri"]

    def test_only_restricted(self):
        cfg = Config(restricted=["Sat"], diverse=[])
        assert cfg.days == ["Sat"]

    def test_only_diverse(self):
        cfg = Config(restricted=[], diverse=["Fri"])
        assert cfg.days == ["Fri"]

    def test_empty_both(self):
        cfg = Config(restricted=[], diverse=[])
        assert cfg.days == []


# ---------------------------------------------------------------------------
# solve() — structural invariants
# ---------------------------------------------------------------------------

def _kids():
    return roster_from_counts({"k": 2, "b": 2})

def _cfg(**overrides):
    defaults = dict(
        boats=2, levels=["k", "b"],
        restricted=["Sat"], diverse=["Fri"],
        stage_seconds=10, seconds=10,
    )
    defaults.update(overrides)
    return Config(**defaults)


class TestSolveInvariants:
    def test_row_count(self):
        rows, _ = solve(_kids(), _cfg())
        assert len(rows) == 8  # 4 kids × 2 days

    def test_every_kid_placed_each_day(self):
        kids = _kids()
        cfg = _cfg()
        rows, _ = solve(kids, cfg)
        for day in cfg.days:
            placed = sorted(n for (d, _, n, _) in rows if d == day)
            expected = sorted(n for n, _ in kids)
            assert placed == expected, f"day {day}: {placed} != {expected}"

    def test_uniform_boat_capacity(self):
        kids = _kids()
        cfg = _cfg()
        rows, _ = solve(kids, cfg)
        cap = len(kids) // cfg.boats
        for day in cfg.days:
            for boat in range(1, cfg.boats + 1):
                count = sum(1 for (d, b, _, _) in rows if d == day and b == boat)
                assert count == cap

    def test_boat_numbers_in_range(self):
        cfg = _cfg()
        rows, _ = solve(_kids(), cfg)
        boats = {b for (_, b, _, _) in rows}
        assert boats == set(range(1, cfg.boats + 1))

    def test_days_in_output_match_config(self):
        cfg = _cfg()
        rows, _ = solve(_kids(), cfg)
        days = {d for (d, _, _, _) in rows}
        assert days == set(cfg.days)

    def test_names_in_output_match_input(self):
        kids = _kids()
        rows, _ = solve(kids, _cfg())
        assert {n for (_, _, n, _) in rows} == {n for n, _ in kids}

    def test_levels_in_output_match_input(self):
        kids = _kids()
        rows, _ = solve(kids, _cfg())
        assert {l for (_, _, _, l) in rows} == {l for _, l in kids}

    def test_stats_keys_present(self):
        _, stats = solve(_kids(), _cfg())
        assert {"mixed", "distinct", "repeats", "status_C"} <= set(stats)

    def test_stats_are_nonneg_ints(self):
        _, stats = solve(_kids(), _cfg())
        for key in ("mixed", "distinct", "repeats"):
            assert isinstance(stats[key], int)
            assert stats[key] >= 0

    def test_status_C_is_str(self):
        _, stats = solve(_kids(), _cfg())
        assert isinstance(stats["status_C"], str)
        assert stats["status_C"] in ("OPTIMAL", "FEASIBLE")


# ---------------------------------------------------------------------------
# solve() — on_progress callback
# ---------------------------------------------------------------------------

class TestOnProgress:
    def test_called_three_times(self):
        calls = []
        solve(_kids(), _cfg(), on_progress=lambda l, s, v: calls.append(l))
        assert calls == ["A mixed", "B diversity", "C repeats"]

    def test_values_are_ints(self):
        calls = []
        solve(_kids(), _cfg(), on_progress=lambda l, s, v: calls.append(v))
        assert all(isinstance(v, int) for v in calls)

    def test_values_are_nonneg(self):
        calls = []
        solve(_kids(), _cfg(), on_progress=lambda l, s, v: calls.append(v))
        assert all(v >= 0 for v in calls)

    def test_statuses_are_strings(self):
        calls = []
        solve(_kids(), _cfg(), on_progress=lambda l, s, v: calls.append(s))
        assert all(isinstance(s, str) for s in calls)


# ---------------------------------------------------------------------------
# solve() — level constraint properties
# ---------------------------------------------------------------------------

class TestSolveConstraints:
    def test_single_level_never_mixed(self):
        kids = roster_from_counts({"k": 4})
        cfg = Config(boats=2, levels=["k"],
                     restricted=["Sat"], diverse=["Fri"],
                     stage_seconds=10, seconds=10)
        _, stats = solve(kids, cfg)
        assert stats["mixed"] == 0

    def test_adj0_homogeneous_restricted_boats(self):
        """With adj=0 and balanced counts, restricted boats must be pure level."""
        kids = roster_from_counts({"k": 4, "b": 4})
        cfg = Config(boats=4, levels=["k", "b"], adj=0,
                     restricted=["Sat"], diverse=[],
                     stage_seconds=15, seconds=15)
        rows, stats = solve(kids, cfg)
        assert stats["mixed"] == 0
        for boat in range(1, 5):
            lvls = {l for (d, b, _, l) in rows if d == "Sat" and b == boat}
            assert len(lvls) == 1, f"boat {boat} mixed levels: {lvls}"

    def test_adj1_blocks_nonadjacent_levels(self):
        """With 3 levels and adj=1, level 0 and level 2 must not share a boat."""
        # 6 kids (2k, 2b, 2A), 3 boats, cap=2; k-b adjacent, b-A adjacent, k-A forbidden
        kids = roster_from_counts({"k": 2, "b": 2, "A": 2})
        cfg = Config(boats=3, levels=["k", "b", "A"], adj=1,
                     restricted=["Sat"], diverse=[],
                     stage_seconds=15, seconds=15)
        level_idx = {"k": 0, "b": 1, "A": 2}
        rows, _ = solve(kids, cfg)
        for boat in range(1, 4):
            idxs = {level_idx[l] for (d, b, _, l) in rows if d == "Sat" and b == boat}
            if len(idxs) > 1:
                assert max(idxs) - min(idxs) <= 1, \
                    f"boat {boat} violates adj=1: levels {idxs}"

    def test_single_day_zero_repeats(self):
        """One day means no pair can share a boat twice, so repeats must be 0."""
        kids = roster_from_counts({"k": 2, "b": 2})
        cfg = Config(boats=2, levels=["k", "b"],
                     restricted=["Sat"], diverse=[],
                     stage_seconds=10, seconds=10)
        _, stats = solve(kids, cfg)
        assert stats["repeats"] == 0

    def test_repeats_bounded_by_days(self):
        """Total repeats can't exceed pairs × (days - 1)."""
        kids = roster_from_counts({"k": 2, "b": 2})
        cfg = _cfg()  # 2 days
        n = len(kids)
        max_repeats = (n * (n - 1) // 2) * (len(cfg.days) - 1)
        _, stats = solve(kids, cfg)
        assert stats["repeats"] <= max_repeats
