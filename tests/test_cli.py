"""Tests for boatsched.cli."""
import csv
import pytest

from boatsched.cli import parse_args, main


class TestParseArgs:
    def test_roster_and_boats(self):
        args = parse_args(["--roster", "r.csv", "--boats", "4"])
        assert args.roster == "r.csv"
        assert args.boats == 4

    def test_counts_and_boats(self):
        args = parse_args(["--counts", "k=4", "b=4", "--boats", "2"])
        assert args.counts == ["k=4", "b=4"]
        assert args.boats == 2

    def test_defaults(self):
        args = parse_args(["--roster", "r.csv", "--boats", "2"])
        assert args.levels == ["k", "b", "A", "B"]
        assert args.restricted == ["Sat", "Sun", "Tue", "Wed"]
        assert args.diverse == ["Fri", "Mon"]
        assert args.adj == 1
        assert args.seconds == 60
        assert args.stage_seconds == 45
        assert args.workers == 8
        assert args.out is None

    def test_custom_levels(self):
        args = parse_args(["--roster", "r.csv", "--boats", "2",
                           "--levels", "x", "y", "z"])
        assert args.levels == ["x", "y", "z"]

    def test_out_flag(self):
        args = parse_args(["--roster", "r.csv", "--boats", "2", "--out", "out.csv"])
        assert args.out == "out.csv"

    def test_stage_seconds_flag(self):
        args = parse_args(["--roster", "r.csv", "--boats", "2", "--stage-seconds", "5"])
        assert args.stage_seconds == 5.0

    def test_roster_and_counts_mutually_exclusive(self):
        with pytest.raises(SystemExit):
            parse_args(["--roster", "r.csv", "--counts", "k=4", "--boats", "2"])

    def test_roster_or_counts_required(self):
        with pytest.raises(SystemExit):
            parse_args(["--boats", "2"])

    def test_boats_required(self):
        with pytest.raises(SystemExit):
            parse_args(["--roster", "r.csv"])


class TestMain:
    _FAST = ["--stage-seconds", "10", "--seconds", "10",
             "--restricted", "Sat", "--diverse", "Fri"]

    def test_returns_zero_on_success(self):
        rc = main(["--counts", "k=2", "b=2", "--boats", "2"] + self._FAST)
        assert rc == 0

    def test_result_header_in_stdout(self, capsys):
        main(["--counts", "k=2", "b=2", "--boats", "2"] + self._FAST)
        assert "RESULT" in capsys.readouterr().out

    def test_progress_to_stderr(self, capsys):
        main(["--counts", "k=2", "b=2", "--boats", "2"] + self._FAST)
        err = capsys.readouterr().err
        assert "A mixed" in err
        assert "B diversity" in err
        assert "C repeats" in err

    def test_writes_csv_file(self, tmp_path):
        out = str(tmp_path / "sched.csv")
        main(["--counts", "k=2", "b=2", "--boats", "2",
              "--out", out] + self._FAST)
        with open(out) as f:
            rows = list(csv.DictReader(f))
        assert len(rows) == 8  # 4 kids × 2 days
        assert set(rows[0]) >= {"day", "boat", "name", "level"}

    def test_csv_day_values(self, tmp_path):
        out = str(tmp_path / "sched.csv")
        main(["--counts", "k=2", "b=2", "--boats", "2",
              "--out", out] + self._FAST)
        with open(out) as f:
            rows = list(csv.DictReader(f))
        days = {r["day"] for r in rows}
        assert days == {"Sat", "Fri"}

    def test_bad_roster_returns_2(self, tmp_path):
        bad = tmp_path / "bad.csv"
        bad.write_text("wrong,columns\n1,2\n")
        rc = main(["--roster", str(bad), "--boats", "2"])
        assert rc == 2

    def test_indivisible_kids_returns_1(self):
        # 5 kids, 2 boats → ValidationError
        rc = main(["--counts", "k=5", "--boats", "2", "--levels", "k"] + self._FAST)
        assert rc == 1

    def test_uses_roster_file(self, tmp_path):
        roster = tmp_path / "roster.csv"
        roster.write_text("name,level\nAlice,k\nBob,k\n")
        rc = main(["--roster", str(roster), "--boats", "2",
                   "--levels", "k"] + self._FAST)
        assert rc == 0

    def test_counts_parsing(self, capsys):
        rc = main(["--counts", "k=2", "b=2", "--boats", "2"] + self._FAST)
        assert rc == 0

    def test_custom_levels_flag(self):
        rc = main(["--counts", "x=2", "y=2", "--boats", "2",
                   "--levels", "x", "y"] + self._FAST)
        assert rc == 0
