"""Expanded tests for boatsched.roster."""
import pytest

from boatsched.roster import load_roster, parse_roster_bytes, roster_from_counts, RosterError


class TestParseRosterBytes:
    def test_bom_stripped(self):
        kids = parse_roster_bytes(b"\xef\xbb\xbfname,level\nAlice,k\n")
        assert kids == [("Alice", "k")]

    def test_case_insensitive_headers(self):
        kids = parse_roster_bytes(b"Name,Level\nAlice,k\n")
        assert kids == [("Alice", "k")]

    def test_whitespace_stripped_from_values(self):
        kids = parse_roster_bytes(b"name,level\n  Alice  ,  k  \n")
        assert kids[0] == ("Alice", "k")

    def test_missing_name_column(self):
        with pytest.raises(RosterError):
            parse_roster_bytes(b"person,level\nAlice,k\n")

    def test_missing_level_column(self):
        with pytest.raises(RosterError):
            parse_roster_bytes(b"name,grade\nAlice,k\n")

    def test_empty_input(self):
        with pytest.raises(RosterError):
            parse_roster_bytes(b"")

    def test_header_row_only_no_data(self):
        with pytest.raises(RosterError):
            parse_roster_bytes(b"name,level\n")

    def test_skips_rows_with_blank_name(self):
        kids = parse_roster_bytes(b"name,level\nAlice,k\n,b\nBob,b\n")
        assert [n for n, _ in kids] == ["Alice", "Bob"]

    def test_multiple_kids(self):
        kids = parse_roster_bytes(b"name,level\nA,k\nB,b\nC,A\n")
        assert len(kids) == 3
        assert kids[2] == ("C", "A")


class TestRosterFromCounts:
    def test_correct_total(self):
        kids = roster_from_counts({"k": 3, "b": 2})
        assert len(kids) == 5

    def test_level_counts(self):
        kids = roster_from_counts({"k": 2, "b": 3, "A": 1})
        levels = [l for _, l in kids]
        assert levels.count("k") == 2
        assert levels.count("b") == 3
        assert levels.count("A") == 1

    def test_names_are_unique(self):
        kids = roster_from_counts({"k": 10})
        names = [n for n, _ in kids]
        assert len(names) == len(set(names))

    def test_names_unique_across_levels(self):
        kids = roster_from_counts({"k": 5, "b": 5})
        names = [n for n, _ in kids]
        assert len(names) == len(set(names))

    def test_empty_returns_empty(self):
        assert roster_from_counts({}) == []


class TestLoadRoster:
    def test_loads_from_file(self, tmp_path):
        f = tmp_path / "roster.csv"
        f.write_text("name,level\nAlice,k\nBob,b\n")
        assert load_roster(str(f)) == [("Alice", "k"), ("Bob", "b")]

    def test_bom_file(self, tmp_path):
        f = tmp_path / "roster.csv"
        f.write_bytes(b"\xef\xbb\xbfname,level\nAlice,k\n")
        assert load_roster(str(f)) == [("Alice", "k")]

    def test_missing_file_raises(self):
        with pytest.raises(OSError):
            load_roster("/no/such/file.csv")
