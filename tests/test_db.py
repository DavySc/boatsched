"""Tests for db.add_or_update_members.

Each test asserts the core requirement: after the call every entry from the
input has a corresponding row in the member table with the correct level.
"""
import pytest

from boatsched.db import get_connection, add_or_update_members
from boatsched.roster import parse_roster_bytes, roster_from_counts


@pytest.fixture
def conn():
    """In-memory DB with the member table and unique index on name."""
    c = get_connection(":memory:")
    c.executescript("""
        CREATE TABLE member (
            id     INTEGER PRIMARY KEY,
            name   TEXT NOT NULL,
            level  TEXT,
            active BOOLEAN NOT NULL DEFAULT 1,
            joined DATE NOT NULL DEFAULT (date('now'))
        );
        CREATE UNIQUE INDEX ix_member_name ON member(name);
    """)
    return c


def _db_members(conn):
    """Return {name: level} for every row currently in the member table."""
    return {row["name"]: row["level"]
            for row in conn.execute("SELECT name, level FROM member")}


def _assert_all_present(conn, members):
    """Assert every (name, level) pair is in the DB with the exact level."""
    db = _db_members(conn)
    for name, level in members:
        assert name in db, f"{name!r} missing from member table"
        assert db[name] == level, (
            f"{name!r}: expected level {level!r}, found {db[name]!r}")


# ---------------------------------------------------------------------------
# Insert
# ---------------------------------------------------------------------------

class TestInsert:
    def test_single_member(self, conn):
        add_or_update_members(conn, [("Alice", "k")])
        _assert_all_present(conn, [("Alice", "k")])

    def test_multiple_members(self, conn):
        members = [("Alice", "k"), ("Bob", "b"), ("Carol", "A"), ("Dave", "B")]
        add_or_update_members(conn, members)
        _assert_all_present(conn, members)

    def test_row_count_matches_input(self, conn):
        members = [("Alice", "k"), ("Bob", "b"), ("Carol", "A")]
        add_or_update_members(conn, members)
        count = conn.execute("SELECT COUNT(*) FROM member").fetchone()[0]
        assert count == len(members)

    def test_empty_list_inserts_nothing(self, conn):
        add_or_update_members(conn, [])
        count = conn.execute("SELECT COUNT(*) FROM member").fetchone()[0]
        assert count == 0


# ---------------------------------------------------------------------------
# Update on conflict
# ---------------------------------------------------------------------------

class TestUpdate:
    def test_level_updated_on_name_conflict(self, conn):
        add_or_update_members(conn, [("Alice", "k")])
        add_or_update_members(conn, [("Alice", "b")])
        _assert_all_present(conn, [("Alice", "b")])

    def test_no_duplicate_row_created(self, conn):
        add_or_update_members(conn, [("Alice", "k")])
        add_or_update_members(conn, [("Alice", "b")])
        count = conn.execute(
            "SELECT COUNT(*) FROM member WHERE name = 'Alice'"
        ).fetchone()[0]
        assert count == 1

    def test_unchanged_member_stays_intact(self, conn):
        add_or_update_members(conn, [("Alice", "k"), ("Bob", "b")])
        add_or_update_members(conn, [("Alice", "A")])
        _assert_all_present(conn, [("Alice", "A"), ("Bob", "b")])

    def test_same_level_repeat_is_idempotent(self, conn):
        add_or_update_members(conn, [("Alice", "k")])
        add_or_update_members(conn, [("Alice", "k")])
        _assert_all_present(conn, [("Alice", "k")])


# ---------------------------------------------------------------------------
# CSV round-trip — the stated requirement
# ---------------------------------------------------------------------------

class TestCsvRoundTrip:
    def test_all_csv_entries_in_db_with_correct_level(self, conn):
        csv = b"name,level\nAlice,k\nBob,b\nCarol,A\nDave,B\n"
        members = parse_roster_bytes(csv)
        add_or_update_members(conn, members)
        _assert_all_present(conn, members)

    def test_csv_row_count_matches_db(self, conn):
        csv = b"name,level\nAlice,k\nBob,b\nCarol,A\nDave,B\n"
        members = parse_roster_bytes(csv)
        add_or_update_members(conn, members)
        count = conn.execute("SELECT COUNT(*) FROM member").fetchone()[0]
        assert count == len(members)

    def test_csv_re_upload_updates_levels(self, conn):
        first  = b"name,level\nAlice,k\nBob,b\n"
        second = b"name,level\nAlice,b\nBob,A\n"
        add_or_update_members(conn, parse_roster_bytes(first))
        members = parse_roster_bytes(second)
        add_or_update_members(conn, members)
        _assert_all_present(conn, members)

    def test_large_roster_all_entries_present(self, conn):
        members = roster_from_counts({"k": 12, "b": 16, "A": 12, "B": 12})
        add_or_update_members(conn, members)
        _assert_all_present(conn, members)

    def test_large_roster_exact_count(self, conn):
        members = roster_from_counts({"k": 12, "b": 16, "A": 12, "B": 12})
        add_or_update_members(conn, members)
        count = conn.execute("SELECT COUNT(*) FROM member").fetchone()[0]
        assert count == len(members)
