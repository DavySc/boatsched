"""Tests for db.add_or_update_members.

Each test asserts the core requirement: after the call every entry from the
input has a corresponding row in the member table with the correct level.
"""
import pytest

from boatsched.db import (get_connection, add_or_update_members,
                          create_season, list_seasons, save_schedule, init_db,
                          list_schedules, get_schedule, swap_assignment,
                          list_boats, rename_boat)
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


# ---------------------------------------------------------------------------
# Fixtures for season / schedule tests (need the full schema)
# ---------------------------------------------------------------------------

@pytest.fixture
def fconn():
    """In-memory DB with the complete schema via init_db."""
    return init_db(":memory:")


# Four members re-used across TestSaveSchedule
_MEMBERS_4 = [("Alice", "k"), ("Bob", "k"), ("Carol", "b"), ("Dave", "b")]
_RESULT_4 = [
    {"day": "Sat", "boat": 1, "name": "Alice", "level": "k"},
    {"day": "Sat", "boat": 1, "name": "Bob",   "level": "k"},
    {"day": "Sat", "boat": 2, "name": "Carol",  "level": "b"},
    {"day": "Sat", "boat": 2, "name": "Dave",   "level": "b"},
]


# ---------------------------------------------------------------------------
# create_season
# ---------------------------------------------------------------------------

class TestCreateSeason:
    def test_returns_integer_id(self, fconn):
        assert isinstance(create_season(fconn, 2026), int)

    def test_season_in_db(self, fconn):
        create_season(fconn, 2026)
        row = fconn.execute("SELECT year FROM season WHERE year=2026").fetchone()
        assert row is not None and row["year"] == 2026

    def test_distinct_ids_for_different_years(self, fconn):
        assert create_season(fconn, 2024) != create_season(fconn, 2025)

    def test_duplicate_year_raises(self, fconn):
        create_season(fconn, 2026)
        with pytest.raises(Exception):
            create_season(fconn, 2026)


# ---------------------------------------------------------------------------
# list_seasons
# ---------------------------------------------------------------------------

class TestListSeasons:
    def test_empty_when_no_seasons(self, fconn):
        assert list_seasons(fconn) == []

    def test_returns_created_season(self, fconn):
        create_season(fconn, 2026)
        seasons = list_seasons(fconn)
        assert len(seasons) == 1 and seasons[0]["year"] == 2026

    def test_has_id_and_year_keys(self, fconn):
        create_season(fconn, 2026)
        s = list_seasons(fconn)[0]
        assert "id" in s and "year" in s

    def test_newest_first(self, fconn):
        for y in [2023, 2025, 2024]:
            create_season(fconn, y)
        years = [s["year"] for s in list_seasons(fconn)]
        assert years == sorted(years, reverse=True)


# ---------------------------------------------------------------------------
# save_schedule
# ---------------------------------------------------------------------------

class TestSaveSchedule:
    def _seed(self, conn, year=2026):
        """Add the four standard members and one season; return season_id."""
        add_or_update_members(conn, _MEMBERS_4)
        return create_season(conn, year)

    def test_returns_integer_id(self, fconn):
        sid = self._seed(fconn)
        assert isinstance(save_schedule(fconn, sid, "{}", _RESULT_4), int)

    def test_schedule_row_linked_to_season(self, fconn):
        sid = self._seed(fconn)
        schedule_id = save_schedule(fconn, sid, "{}", _RESULT_4)
        row = fconn.execute(
            "SELECT season_id FROM schedule WHERE id=?", (schedule_id,)
        ).fetchone()
        assert row is not None and row["season_id"] == sid

    def test_config_json_stored(self, fconn):
        sid = self._seed(fconn)
        cfg = '{"boats": 2}'
        schedule_id = save_schedule(fconn, sid, cfg, _RESULT_4)
        row = fconn.execute(
            "SELECT config FROM schedule WHERE id=?", (schedule_id,)
        ).fetchone()
        assert row["config"] == cfg

    def test_boat_assignment_count(self, fconn):
        sid = self._seed(fconn)
        schedule_id = save_schedule(fconn, sid, "{}", _RESULT_4)
        count = fconn.execute(
            "SELECT COUNT(*) FROM boat_assignment WHERE schedule_id=?",
            (schedule_id,)
        ).fetchone()[0]
        assert count == len(_RESULT_4)

    def test_assignment_links_correct_member(self, fconn):
        sid = self._seed(fconn)
        schedule_id = save_schedule(fconn, sid, "{}", _RESULT_4)
        alice_id = fconn.execute(
            "SELECT id FROM member WHERE name='Alice'"
        ).fetchone()["id"]
        row = fconn.execute(
            "SELECT boat_id AS boat, day FROM boat_assignment"
            " WHERE schedule_id=? AND member_id=?",
            (schedule_id, alice_id),
        ).fetchone()
        assert row is not None
        assert row["boat"] == 1
        assert row["day"] == "Sat"

    def test_unknown_member_raises(self, fconn):
        sid = self._seed(fconn)
        with pytest.raises(RuntimeError, match="not found"):
            save_schedule(fconn, sid, "{}", [
                {"day": "Sat", "boat": 1, "name": "Nobody", "level": "k"},
            ])


# ---------------------------------------------------------------------------
# list_schedules
# ---------------------------------------------------------------------------

class TestListSchedules:
    def _seed(self, conn, year=2026):
        add_or_update_members(conn, _MEMBERS_4)
        return create_season(conn, year)

    def test_empty_when_no_schedules(self, fconn):
        sid = self._seed(fconn)
        assert list_schedules(fconn, sid) == []

    def test_returns_saved_schedule(self, fconn):
        sid = self._seed(fconn)
        save_schedule(fconn, sid, "{}", _RESULT_4)
        assert len(list_schedules(fconn, sid)) == 1

    def test_has_id_and_created_keys(self, fconn):
        sid = self._seed(fconn)
        save_schedule(fconn, sid, "{}", _RESULT_4)
        s = list_schedules(fconn, sid)[0]
        assert "id" in s and "created" in s

    def test_newest_first(self, fconn):
        sid = self._seed(fconn)
        id1 = save_schedule(fconn, sid, "{}", _RESULT_4)
        id2 = save_schedule(fconn, sid, "{}", _RESULT_4)
        ids = [s["id"] for s in list_schedules(fconn, sid)]
        assert ids == [id2, id1]

    def test_isolated_by_season(self, fconn):
        sid1 = self._seed(fconn, 2026)
        sid2 = create_season(fconn, 2027)
        save_schedule(fconn, sid1, "{}", _RESULT_4)
        assert list_schedules(fconn, sid2) == []


# ---------------------------------------------------------------------------
# get_schedule
# ---------------------------------------------------------------------------

class TestGetSchedule:
    def _seed(self, conn, year=2026):
        add_or_update_members(conn, _MEMBERS_4)
        sid = create_season(conn, year)
        return save_schedule(conn, sid, "{}", _RESULT_4)

    def test_returns_all_rows(self, fconn):
        schedule_id = self._seed(fconn)
        rows = get_schedule(fconn, schedule_id)
        assert len(rows) == len(_RESULT_4)

    def test_row_has_expected_keys(self, fconn):
        schedule_id = self._seed(fconn)
        row = get_schedule(fconn, schedule_id)[0]
        assert {"day", "boat", "name", "level"} <= set(row)

    def test_names_match_input(self, fconn):
        schedule_id = self._seed(fconn)
        names = {r["name"] for r in get_schedule(fconn, schedule_id)}
        assert names == {"Alice", "Bob", "Carol", "Dave"}

    def test_empty_for_unknown_id(self, fconn):
        assert get_schedule(fconn, 9999) == []


# ---------------------------------------------------------------------------
# swap_assignment
# ---------------------------------------------------------------------------

class TestSwapAssignment:
    def _seed(self, conn, year=2026):
        add_or_update_members(conn, _MEMBERS_4)
        sid = create_season(conn, year)
        return save_schedule(conn, sid, "{}", _RESULT_4)

    def _boat_of(self, conn, schedule_id, name, day="Sat"):
        return conn.execute(
            "SELECT ba.boat_id FROM boat_assignment ba"
            " JOIN member m ON m.id = ba.member_id"
            " WHERE ba.schedule_id=? AND ba.day=? AND m.name=?",
            (schedule_id, day, name),
        ).fetchone()["boat_id"]

    def test_swap_exchanges_boats(self, fconn):
        sid = self._seed(fconn)
        boat_alice_before = self._boat_of(fconn, sid, "Alice")
        boat_carol_before = self._boat_of(fconn, sid, "Carol")
        swap_assignment(fconn, sid, "Sat", "Alice", "Carol")
        assert self._boat_of(fconn, sid, "Alice") == boat_carol_before
        assert self._boat_of(fconn, sid, "Carol") == boat_alice_before

    def test_same_boat_is_noop(self, fconn):
        sid = self._seed(fconn)
        boat_before = self._boat_of(fconn, sid, "Alice")
        swap_assignment(fconn, sid, "Sat", "Alice", "Bob")
        assert self._boat_of(fconn, sid, "Alice") == boat_before

    def test_swap_is_reversible(self, fconn):
        sid = self._seed(fconn)
        boat_before = self._boat_of(fconn, sid, "Alice")
        swap_assignment(fconn, sid, "Sat", "Alice", "Carol")
        swap_assignment(fconn, sid, "Sat", "Alice", "Carol")
        assert self._boat_of(fconn, sid, "Alice") == boat_before

    def test_unknown_member_raises(self, fconn):
        sid = self._seed(fconn)
        with pytest.raises(RuntimeError, match="not found"):
            swap_assignment(fconn, sid, "Sat", "Alice", "Nobody")

    def test_row_count_unchanged_after_swap(self, fconn):
        sid = self._seed(fconn)
        before = fconn.execute(
            "SELECT COUNT(*) FROM boat_assignment WHERE schedule_id=?", (sid,)
        ).fetchone()[0]
        swap_assignment(fconn, sid, "Sat", "Alice", "Carol")
        after = fconn.execute(
            "SELECT COUNT(*) FROM boat_assignment WHERE schedule_id=?", (sid,)
        ).fetchone()[0]
        assert before == after


# ---------------------------------------------------------------------------
# list_boats / rename_boat
# ---------------------------------------------------------------------------

class TestBoats:
    def test_thirteen_boats_seeded(self, fconn):
        boats = list_boats(fconn)
        assert len(boats) == 13

    def test_boat_ids_are_one_to_thirteen(self, fconn):
        ids = [b["id"] for b in list_boats(fconn)]
        assert ids == list(range(1, 14))

    def test_boats_have_no_name_by_default(self, fconn):
        assert all(b["name"] is None for b in list_boats(fconn))

    def test_rename_sets_name(self, fconn):
        rename_boat(fconn, 1, "Blue Jay")
        boat = next(b for b in list_boats(fconn) if b["id"] == 1)
        assert boat["name"] == "Blue Jay"

    def test_rename_to_none_clears_name(self, fconn):
        rename_boat(fconn, 1, "Blue Jay")
        rename_boat(fconn, 1, None)
        boat = next(b for b in list_boats(fconn) if b["id"] == 1)
        assert boat["name"] is None

    def test_rename_empty_string_clears_name(self, fconn):
        rename_boat(fconn, 1, "Blue Jay")
        rename_boat(fconn, 1, "")
        boat = next(b for b in list_boats(fconn) if b["id"] == 1)
        assert boat["name"] is None

    def test_rename_only_affects_target_boat(self, fconn):
        rename_boat(fconn, 3, "Sunfish")
        others = [b for b in list_boats(fconn) if b["id"] != 3]
        assert all(b["name"] is None for b in others)
