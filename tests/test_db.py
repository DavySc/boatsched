"""Tests for db helpers: members, enrollment, seasons, schedules, boats."""
import sqlite3

import pytest

from boatsched.db import (
    get_connection, add_or_update_members, add_enrollment, get_season_roster,
    create_season, list_seasons, save_schedule, init_db,
    list_schedules, get_schedule, swap_assignment,
    list_boats, rename_boat,
    seed_members, list_members, delete_member, rename_member,
    set_admin, get_yjs_state, set_yjs_state, set_schedule_start_date,
    get_schedule_start_date, replace_assignments,
    add_admin_column, auto_grant_first_admin, add_yjs_state_column,
    add_start_date_column,
)
from boatsched.roster import parse_roster_bytes, roster_from_counts


# ---------------------------------------------------------------------------
# Minimal fixture — member table only (no level column)
# ---------------------------------------------------------------------------

@pytest.fixture
def conn():
    """In-memory DB with member table (no level) and unique index on name."""
    c = get_connection(":memory:")
    c.executescript("""
        CREATE TABLE member (
            id     INTEGER PRIMARY KEY,
            name   TEXT NOT NULL,
            active BOOLEAN NOT NULL DEFAULT 1,
            joined DATE NOT NULL DEFAULT (date('now'))
        );
        CREATE UNIQUE INDEX ix_member_name ON member(name);
    """)
    return c


def _member_names(conn):
    """Return the set of names currently in the member table."""
    return {row["name"] for row in conn.execute("SELECT name FROM member")}


def _assert_names_present(conn, members):
    """Assert every name from members is present in the member table."""
    db = _member_names(conn)
    for name, _ in members:
        assert name in db, f"{name!r} missing from member table"


# ---------------------------------------------------------------------------
# Full-schema fixture
# ---------------------------------------------------------------------------

@pytest.fixture
def fconn():
    """In-memory DB with the complete schema via init_db."""
    return init_db(":memory:")


# ---------------------------------------------------------------------------
# Four members re-used across schedule tests
# ---------------------------------------------------------------------------

_MEMBERS_4 = [("Alice", "k"), ("Bob", "k"), ("Carol", "b"), ("Dave", "b")]
_RESULT_4 = [
    {"day": "Sat", "boat": 1, "name": "Alice", "level": "k"},
    {"day": "Sat", "boat": 1, "name": "Bob",   "level": "k"},
    {"day": "Sat", "boat": 2, "name": "Carol",  "level": "b"},
    {"day": "Sat", "boat": 2, "name": "Dave",   "level": "b"},
]


# ---------------------------------------------------------------------------
# Member insertion (no enrollment — just name persistence)
# ---------------------------------------------------------------------------

class TestInsert:
    def test_single_member(self, conn):
        add_or_update_members(conn, [("Alice", "k")])
        assert "Alice" in _member_names(conn)

    def test_multiple_members(self, conn):
        members = [("Alice", "k"), ("Bob", "b"), ("Carol", "A"), ("Dave", "B")]
        conn.executemany("INSERT INTO member (name) VALUES (?)",
                         [(name,) for name, level in members if level != "k"])
        conn.commit()
        add_or_update_members(conn, members)
        _assert_names_present(conn, members)

    def test_row_count_matches_input(self, conn):
        members = [("Alice", "k"), ("Bob", "b"), ("Carol", "A")]
        conn.executemany("INSERT INTO member (name) VALUES (?)",
                         [(name,) for name, level in members if level != "k"])
        conn.commit()
        add_or_update_members(conn, members)
        count = conn.execute("SELECT COUNT(*) FROM member").fetchone()[0]
        assert count == len(members)

    def test_non_k_member_not_found_raises(self, conn):
        with pytest.raises(RuntimeError, match="not found"):
            add_or_update_members(conn, [("Ghost", "b")])

    def test_empty_list_inserts_nothing(self, conn):
        add_or_update_members(conn, [])
        count = conn.execute("SELECT COUNT(*) FROM member").fetchone()[0]
        assert count == 0


class TestUpdate:
    def test_no_duplicate_row_on_name_conflict(self, conn):
        add_or_update_members(conn, [("Alice", "k")])
        add_or_update_members(conn, [("Alice", "b")])
        count = conn.execute(
            "SELECT COUNT(*) FROM member WHERE name = 'Alice'"
        ).fetchone()[0]
        assert count == 1

    def test_unchanged_member_stays_intact(self, conn):
        conn.execute("INSERT INTO member (name) VALUES ('Bob')")
        conn.commit()
        add_or_update_members(conn, [("Alice", "k"), ("Bob", "b")])
        add_or_update_members(conn, [("Alice", "A")])
        _assert_names_present(conn, [("Alice", "A"), ("Bob", "b")])

    def test_repeat_insert_is_idempotent(self, conn):
        add_or_update_members(conn, [("Alice", "k")])
        add_or_update_members(conn, [("Alice", "k")])
        assert conn.execute("SELECT COUNT(*) FROM member").fetchone()[0] == 1


class TestCsvRoundTrip:
    def test_all_csv_names_in_db(self, conn):
        csv = b"name,level\nAlice,k\nBob,b\nCarol,A\nDave,B\n"
        members = parse_roster_bytes(csv)
        conn.executemany("INSERT INTO member (name) VALUES (?)",
                         [(name,) for name, level in members if level != "k"])
        conn.commit()
        add_or_update_members(conn, members)
        _assert_names_present(conn, members)

    def test_csv_row_count_matches_db(self, conn):
        csv = b"name,level\nAlice,k\nBob,b\nCarol,A\nDave,B\n"
        members = parse_roster_bytes(csv)
        conn.executemany("INSERT INTO member (name) VALUES (?)",
                         [(name,) for name, level in members if level != "k"])
        conn.commit()
        add_or_update_members(conn, members)
        count = conn.execute("SELECT COUNT(*) FROM member").fetchone()[0]
        assert count == len(members)

    def test_large_roster_all_names_present(self, conn):
        members = roster_from_counts({"k": 12, "b": 16, "A": 12, "B": 12})
        conn.executemany("INSERT INTO member (name) VALUES (?)",
                         [(name,) for name, level in members if level != "k"])
        conn.commit()
        add_or_update_members(conn, members)
        _assert_names_present(conn, members)


# ---------------------------------------------------------------------------
# Enrollment — level is now per-season
# ---------------------------------------------------------------------------

class TestEnrollment:
    def test_level_stored_in_enrollment(self, fconn):
        sid = create_season(fconn, 2026)
        add_or_update_members(fconn, [("Alice", "k")], sid)
        row = fconn.execute(
            "SELECT e.level FROM enrollment e JOIN member m ON m.id = e.member_id"
            " WHERE m.name='Alice' AND e.season_id=?", (sid,)
        ).fetchone()
        assert row is not None
        assert row["level"] == "k"

    def test_enrollment_level_updated_on_conflict(self, fconn):
        sid = create_season(fconn, 2026)
        add_or_update_members(fconn, [("Alice", "k")], sid)
        add_or_update_members(fconn, [("Alice", "b")], sid)
        row = fconn.execute(
            "SELECT e.level FROM enrollment e JOIN member m ON m.id = e.member_id"
            " WHERE m.name='Alice' AND e.season_id=?", (sid,)
        ).fetchone()
        assert row["level"] == "b"

    def test_no_enrollment_without_season_id(self, fconn):
        add_or_update_members(fconn, [("Alice", "k")])
        count = fconn.execute("SELECT COUNT(*) FROM enrollment").fetchone()[0]
        assert count == 0

    def test_multiple_seasons_are_independent(self, fconn):
        sid1 = create_season(fconn, 2026)
        sid2 = create_season(fconn, 2027)
        add_or_update_members(fconn, [("Alice", "k")], sid1)
        add_or_update_members(fconn, [("Alice", "b")], sid2)
        lev = lambda sid: fconn.execute(
            "SELECT e.level FROM enrollment e JOIN member m ON m.id = e.member_id"
            " WHERE m.name='Alice' AND e.season_id=?", (sid,)
        ).fetchone()["level"]
        assert lev(sid1) == "k"
        assert lev(sid2) == "b"

    def test_add_enrollment_helper(self, fconn):
        sid = create_season(fconn, 2026)
        fconn.execute("INSERT INTO member (name) VALUES ('Bob')")
        bob_id = fconn.execute("SELECT id FROM member WHERE name='Bob'").fetchone()["id"]
        fconn.commit()
        add_enrollment(fconn, bob_id, sid, "A")
        row = fconn.execute(
            "SELECT level FROM enrollment WHERE member_id=? AND season_id=?",
            (bob_id, sid)
        ).fetchone()
        assert row["level"] == "A"

    def test_get_season_roster_returns_enrolled_members(self, fconn):
        sid = create_season(fconn, 2026)
        fconn.execute("INSERT INTO member (name) VALUES ('Bob')")
        fconn.commit()
        add_or_update_members(fconn, [("Alice", "k"), ("Bob", "b")], sid)
        roster = get_season_roster(fconn, sid)
        assert sorted(roster) == sorted([("Alice", "k"), ("Bob", "b")])

    def test_get_season_roster_empty_when_none_enrolled(self, fconn):
        sid = create_season(fconn, 2026)
        assert get_season_roster(fconn, sid) == []

    def test_get_season_roster_excludes_other_seasons(self, fconn):
        sid1 = create_season(fconn, 2026)
        sid2 = create_season(fconn, 2027)
        add_or_update_members(fconn, [("Alice", "k")], sid1)
        assert get_season_roster(fconn, sid2) == []


# ---------------------------------------------------------------------------
# Required: FK prevents skill_result without enrollment
# ---------------------------------------------------------------------------

class TestSkillResultForeignKey:
    def test_skill_result_requires_enrollment(self, fconn):
        """INSERT into skill_result without a matching enrollment raises FK error."""
        sid = create_season(fconn, 2026)
        fconn.execute("INSERT INTO member (name) VALUES ('Alice')")
        alice_id = fconn.execute(
            "SELECT id FROM member WHERE name='Alice'"
        ).fetchone()["id"]
        fconn.execute(
            "INSERT INTO skill (level, code, label, ordinal) VALUES ('k','k1','Knoop',1)"
        )
        skill_id = fconn.execute(
            "SELECT id FROM skill WHERE code='k1'"
        ).fetchone()["id"]
        fconn.commit()

        with pytest.raises(sqlite3.IntegrityError):
            fconn.execute(
                "INSERT INTO skill_result (member_id, season_id, skill_id, status)"
                " VALUES (?, ?, ?, 'done')", (alice_id, sid, skill_id)
            )
            fconn.commit()


# ---------------------------------------------------------------------------
# Required: get_schedule raises on missing enrollment
# ---------------------------------------------------------------------------

def test_get_schedule_raises_on_missing_enrollment(fconn):
    """get_schedule raises RuntimeError if any scheduled member lacks enrollment."""
    sid = create_season(fconn, 2026)
    fconn.execute("INSERT INTO member (name) VALUES ('Alice')")
    alice_id = fconn.execute(
        "SELECT id FROM member WHERE name='Alice'"
    ).fetchone()["id"]
    fconn.commit()
    # Insert schedule and assignment without creating enrollment
    sch_id = fconn.execute(
        "INSERT INTO schedule (season_id, config) VALUES (?, ?)", (sid, "{}")
    ).lastrowid
    fconn.execute(
        "INSERT INTO boat_assignment (schedule_id, member_id, boat_id, day)"
        " VALUES (?, ?, 1, 'Sat')", (sch_id, alice_id)
    )
    fconn.commit()

    with pytest.raises(RuntimeError, match="not enrolled"):
        get_schedule(fconn, sch_id)


# ---------------------------------------------------------------------------
# Required: retiring a skill preserves existing skill_result rows
# ---------------------------------------------------------------------------

class TestSkillRetirement:
    def _seed_skill(self, fconn, member_name, level, code, season_year=2026):
        """Create member + enrollment + skill, return (member_id, season_id, skill_id)."""
        sid = create_season(fconn, season_year)
        fconn.execute("INSERT INTO member (name) VALUES (?)", (member_name,))
        mid = fconn.execute(
            "SELECT id FROM member WHERE name=?", (member_name,)
        ).fetchone()["id"]
        fconn.execute(
            "INSERT INTO enrollment (member_id, season_id, level) VALUES (?, ?, ?)",
            (mid, sid, level)
        )
        fconn.execute(
            "INSERT INTO skill (level, code, label, ordinal) VALUES (?, ?, ?, 1)",
            (level, code, "Label")
        )
        skill_id = fconn.execute(
            "SELECT id FROM skill WHERE code=?", (code,)
        ).fetchone()["id"]
        fconn.commit()
        return mid, sid, skill_id

    def test_retiring_skill_keeps_existing_results(self, fconn):
        mid, sid, skill_id = self._seed_skill(fconn, "Alice", "k", "k_retire")
        fconn.execute(
            "INSERT INTO skill_result (member_id, season_id, skill_id, status)"
            " VALUES (?, ?, ?, 'done')", (mid, sid, skill_id)
        )
        fconn.commit()

        fconn.execute("UPDATE skill SET retired=1 WHERE id=?", (skill_id,))
        fconn.commit()

        count = fconn.execute(
            "SELECT COUNT(*) FROM skill_result WHERE skill_id=?", (skill_id,)
        ).fetchone()[0]
        assert count == 1

    def test_retired_skill_hidden_in_view(self, fconn):
        mid, sid, skill_id = self._seed_skill(fconn, "Bob", "k", "k_hide")
        fconn.execute("UPDATE skill SET retired=1 WHERE id=?", (skill_id,))
        fconn.commit()

        rows = fconn.execute(
            "SELECT * FROM v_level_card WHERE member_id=? AND season_id=?",
            (mid, sid)
        ).fetchall()
        assert all(r["skill_id"] != skill_id for r in rows)

    def test_replacement_skill_appears_in_view(self, fconn):
        mid, sid, old_id = self._seed_skill(fconn, "Carol", "k", "k_old")
        fconn.execute("UPDATE skill SET retired=1 WHERE id=?", (old_id,))
        fconn.execute(
            "INSERT INTO skill (level, code, label, ordinal) VALUES ('k','k_new','New',2)"
        )
        fconn.commit()

        rows = fconn.execute(
            "SELECT skill_code FROM v_level_card WHERE member_id=? AND season_id=?",
            (mid, sid)
        ).fetchall()
        codes = [r["skill_code"] for r in rows]
        assert "k_new" in codes
        assert "k_old" not in codes


# ---------------------------------------------------------------------------
# Required: v_level_card shows one row per live skill with NULL for unfilled
# ---------------------------------------------------------------------------

class TestVLevelCard:
    def test_one_row_per_live_skill_null_status(self, fconn):
        sid = create_season(fconn, 2026)
        fconn.execute("INSERT INTO member (name) VALUES ('Dave')")
        mid = fconn.execute(
            "SELECT id FROM member WHERE name='Dave'"
        ).fetchone()["id"]
        fconn.execute(
            "INSERT INTO enrollment (member_id, season_id, level) VALUES (?, ?, 'k')",
            (mid, sid)
        )
        fconn.executemany(
            "INSERT INTO skill (level, code, label, ordinal) VALUES ('k', ?, ?, ?)",
            [("k1", "Skill1", 1), ("k2", "Skill2", 2), ("k3", "Skill3", 3)]
        )
        fconn.commit()

        rows = fconn.execute(
            "SELECT * FROM v_level_card WHERE member_id=? AND season_id=?",
            (mid, sid)
        ).fetchall()
        assert len(rows) == 3
        assert all(r["status"] is None for r in rows)
        assert all(r["note"] is None for r in rows)

    def test_filled_skill_shows_status_and_note(self, fconn):
        sid = create_season(fconn, 2026)
        fconn.execute("INSERT INTO member (name) VALUES ('Eve')")
        mid = fconn.execute(
            "SELECT id FROM member WHERE name='Eve'"
        ).fetchone()["id"]
        fconn.execute(
            "INSERT INTO enrollment (member_id, season_id, level) VALUES (?, ?, 'b')",
            (mid, sid)
        )
        fconn.execute(
            "INSERT INTO skill (level, code, label, ordinal) VALUES ('b','b1','Sturen',1)"
        )
        skill_id = fconn.execute(
            "SELECT id FROM skill WHERE code='b1'"
        ).fetchone()["id"]
        fconn.execute(
            "INSERT INTO skill_result (member_id, season_id, skill_id, status, note)"
            " VALUES (?, ?, ?, 'done', 'goed')", (mid, sid, skill_id)
        )
        fconn.commit()

        row = fconn.execute(
            "SELECT status, note FROM v_level_card WHERE member_id=? AND season_id=?",
            (mid, sid)
        ).fetchone()
        assert row["status"] == "done"
        assert row["note"] == "goed"

    def test_unfilled_skills_return_null_alongside_filled(self, fconn):
        sid = create_season(fconn, 2026)
        fconn.execute("INSERT INTO member (name) VALUES ('Frank')")
        mid = fconn.execute(
            "SELECT id FROM member WHERE name='Frank'"
        ).fetchone()["id"]
        fconn.execute(
            "INSERT INTO enrollment (member_id, season_id, level) VALUES (?, ?, 'A')",
            (mid, sid)
        )
        fconn.executemany(
            "INSERT INTO skill (level, code, label, ordinal) VALUES ('A', ?, ?, ?)",
            [("a1", "SkillA1", 1), ("a2", "SkillA2", 2)]
        )
        skill_id = fconn.execute(
            "SELECT id FROM skill WHERE code='a1'"
        ).fetchone()["id"]
        fconn.execute(
            "INSERT INTO skill_result (member_id, season_id, skill_id, status)"
            " VALUES (?, ?, ?, 'ok')", (mid, sid, skill_id)
        )
        fconn.commit()

        rows = fconn.execute(
            "SELECT skill_code, status FROM v_level_card"
            " WHERE member_id=? AND season_id=? ORDER BY skill_code",
            (mid, sid)
        ).fetchall()
        assert len(rows) == 2
        statuses = {r["skill_code"]: r["status"] for r in rows}
        assert statuses["a1"] == "ok"
        assert statuses["a2"] is None


# ---------------------------------------------------------------------------
# create_season / list_seasons
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
# save_schedule — seed must create season first, then enroll members
# ---------------------------------------------------------------------------

class TestSaveSchedule:
    def _seed(self, conn, year=2026):
        """Create season, add four members with enrollment, return season_id."""
        sid = create_season(conn, year)
        conn.executemany("INSERT OR IGNORE INTO member (name) VALUES (?)",
                         [("Carol",), ("Dave",)])
        conn.commit()
        add_or_update_members(conn, _MEMBERS_4, sid)
        return sid

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
        sid = create_season(conn, year)
        conn.executemany("INSERT OR IGNORE INTO member (name) VALUES (?)",
                         [("Carol",), ("Dave",)])
        conn.commit()
        add_or_update_members(conn, _MEMBERS_4, sid)
        return sid

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
# get_schedule — level now comes from enrollment
# ---------------------------------------------------------------------------

class TestGetSchedule:
    def _seed(self, conn, year=2026):
        sid = create_season(conn, year)
        conn.executemany("INSERT OR IGNORE INTO member (name) VALUES (?)",
                         [("Carol",), ("Dave",)])
        conn.commit()
        add_or_update_members(conn, _MEMBERS_4, sid)
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

    def test_levels_come_from_enrollment(self, fconn):
        schedule_id = self._seed(fconn)
        rows = get_schedule(fconn, schedule_id)
        level_map = {r["name"]: r["level"] for r in rows}
        assert level_map["Alice"] == "k"
        assert level_map["Carol"] == "b"

    def test_empty_for_unknown_id(self, fconn):
        assert get_schedule(fconn, 9999) == []


# ---------------------------------------------------------------------------
# swap_assignment
# ---------------------------------------------------------------------------

class TestSwapAssignment:
    def _seed(self, conn, year=2026):
        sid = create_season(conn, year)
        conn.executemany("INSERT OR IGNORE INTO member (name) VALUES (?)",
                         [("Carol",), ("Dave",)])
        conn.commit()
        add_or_update_members(conn, _MEMBERS_4, sid)
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


# ---------------------------------------------------------------------------
# Migration idempotency
# ---------------------------------------------------------------------------

class TestMigrationIdempotency:
    def test_add_admin_column_is_idempotent(self, fconn):
        """Column already exists after init_db — second call must not raise."""
        add_admin_column(fconn)  # should be a no-op (catches OperationalError)
        rows = fconn.execute("PRAGMA table_info(instructor)").fetchall()
        col_names = [r["name"] for r in rows]
        assert "is_admin" in col_names

    def test_add_yjs_state_column_is_idempotent(self, fconn):
        add_yjs_state_column(fconn)
        rows = fconn.execute("PRAGMA table_info(schedule)").fetchall()
        assert any(r["name"] == "yjs_state" for r in rows)

    def test_add_start_date_column_is_idempotent(self, fconn):
        add_start_date_column(fconn)
        rows = fconn.execute("PRAGMA table_info(schedule)").fetchall()
        assert any(r["name"] == "start_date" for r in rows)

    def test_auto_grant_first_admin_skips_when_admin_exists(self, fconn):
        """With an admin already present it must leave counts unchanged."""
        add_admin_column(fconn)  # ensure column exists (init_db doesn't run migrations)
        fconn.execute("INSERT INTO member (name) VALUES ('Root')")
        fconn.commit()
        mid = fconn.execute("SELECT id FROM member WHERE name='Root'").fetchone()["id"]
        fconn.execute("INSERT INTO instructor (member_id, is_admin) VALUES (?, 1)", (mid,))
        fconn.commit()
        auto_grant_first_admin(fconn)  # must be a no-op
        count = fconn.execute("SELECT COUNT(*) FROM instructor WHERE is_admin=1").fetchone()[0]
        assert count == 1

    def test_auto_grant_first_admin_promotes_all_when_none(self, fconn):
        add_admin_column(fconn)
        for name in ("I1", "I2"):
            fconn.execute("INSERT INTO member (name) VALUES (?)", (name,))
            mid = fconn.execute("SELECT id FROM member WHERE name=?", (name,)).fetchone()["id"]
            fconn.execute("INSERT INTO instructor (member_id, is_admin) VALUES (?, 0)", (mid,))
        fconn.commit()
        auto_grant_first_admin(fconn)
        count = fconn.execute("SELECT COUNT(*) FROM instructor WHERE is_admin=0").fetchone()[0]
        assert count == 0


# ---------------------------------------------------------------------------
# seed_members
# ---------------------------------------------------------------------------

class TestSeedMembers:
    def test_inserts_new_names(self, fconn):
        result = seed_members(fconn, ["Alpha", "Beta"])
        assert result["created"] == 2
        names = {r["name"] for r in fconn.execute("SELECT name FROM member")}
        assert {"Alpha", "Beta"} <= names

    def test_existing_names_count_as_existing(self, fconn):
        fconn.execute("INSERT INTO member (name) VALUES ('Gamma')")
        fconn.commit()
        result = seed_members(fconn, ["Gamma", "Delta"])
        assert result["created"] == 1
        assert result["existing"] == 1

    def test_total_reflects_full_table(self, fconn):
        seed_members(fconn, ["E1", "E2"])
        result = seed_members(fconn, ["E3"])
        assert result["total"] >= 3

    def test_empty_list_returns_zero_created(self, fconn):
        result = seed_members(fconn, [])
        assert result["created"] == 0
        assert result["existing"] == 0

    def test_blank_names_filtered_out(self, fconn):
        result = seed_members(fconn, ["  ", "", None, "Valid"])
        assert result["created"] == 1

    def test_strips_whitespace_from_names(self, fconn):
        seed_members(fconn, ["  Trimmed  "])
        names = {r["name"] for r in fconn.execute("SELECT name FROM member")}
        assert "Trimmed" in names
        assert "  Trimmed  " not in names


# ---------------------------------------------------------------------------
# list_members
# ---------------------------------------------------------------------------

class TestListMembers:
    def test_returns_active_members(self, fconn):
        fconn.executemany("INSERT INTO member (name) VALUES (?)", [("L1",), ("L2",)])
        fconn.commit()
        members = list_members(fconn)
        names = [m["name"] for m in members]
        assert "L1" in names and "L2" in names

    def test_inactive_members_excluded(self, fconn):
        fconn.execute("INSERT INTO member (name) VALUES ('Inactive')")
        fconn.commit()
        mid = fconn.execute("SELECT id FROM member WHERE name='Inactive'").fetchone()["id"]
        fconn.execute("UPDATE member SET active=0 WHERE id=?", (mid,))
        fconn.commit()
        names = [m["name"] for m in list_members(fconn)]
        assert "Inactive" not in names

    def test_ordered_by_name(self, fconn):
        fconn.executemany("INSERT INTO member (name) VALUES (?)", [("Zara",), ("Anna",)])
        fconn.commit()
        names = [m["name"] for m in list_members(fconn)]
        assert names == sorted(names)

    def test_has_id_and_name_keys(self, fconn):
        fconn.execute("INSERT INTO member (name) VALUES ('KeyCheck')")
        fconn.commit()
        m = next(x for x in list_members(fconn) if x["name"] == "KeyCheck")
        assert "id" in m and "name" in m


# ---------------------------------------------------------------------------
# delete_member / rename_member
# ---------------------------------------------------------------------------

class TestDeleteMemberDb:
    def test_sets_active_to_zero(self, fconn):
        fconn.execute("INSERT INTO member (name) VALUES ('Del1')")
        fconn.commit()
        mid = fconn.execute("SELECT id FROM member WHERE name='Del1'").fetchone()["id"]
        delete_member(fconn, mid)
        active = fconn.execute("SELECT active FROM member WHERE id=?", (mid,)).fetchone()["active"]
        assert active == 0

    def test_raises_if_instructor(self, fconn):
        fconn.execute("INSERT INTO member (name) VALUES ('Inst1')")
        fconn.commit()
        mid = fconn.execute("SELECT id FROM member WHERE name='Inst1'").fetchone()["id"]
        fconn.execute("INSERT INTO instructor (member_id) VALUES (?)", (mid,))
        fconn.commit()
        with pytest.raises(ValueError, match="instructeur"):
            delete_member(fconn, mid)


class TestRenameMemberDb:
    def test_rename_updates_name(self, fconn):
        fconn.execute("INSERT INTO member (name) VALUES ('OldName')")
        fconn.commit()
        mid = fconn.execute("SELECT id FROM member WHERE name='OldName'").fetchone()["id"]
        rename_member(fconn, mid, "NewName")
        row = fconn.execute("SELECT name FROM member WHERE id=?", (mid,)).fetchone()
        assert row["name"] == "NewName"

    def test_empty_name_raises(self, fconn):
        fconn.execute("INSERT INTO member (name) VALUES ('HasName')")
        fconn.commit()
        mid = fconn.execute("SELECT id FROM member WHERE name='HasName'").fetchone()["id"]
        with pytest.raises(ValueError, match="empty"):
            rename_member(fconn, mid, "   ")

    def test_collision_raises(self, fconn):
        fconn.executemany("INSERT INTO member (name) VALUES (?)", [("Exists",), ("ToRename",)])
        fconn.commit()
        mid = fconn.execute("SELECT id FROM member WHERE name='ToRename'").fetchone()["id"]
        with pytest.raises(ValueError, match="bestaat"):
            rename_member(fconn, mid, "Exists")


# ---------------------------------------------------------------------------
# set_admin
# ---------------------------------------------------------------------------

class TestSetAdminDb:
    def _make_instructor(self, fconn, name, is_admin=False):
        add_admin_column(fconn)  # migration not run by init_db
        fconn.execute("INSERT INTO member (name) VALUES (?)", (name,))
        fconn.commit()
        mid = fconn.execute("SELECT id FROM member WHERE name=?", (name,)).fetchone()["id"]
        fconn.execute("INSERT INTO instructor (member_id, is_admin) VALUES (?, ?)",
                      (mid, 1 if is_admin else 0))
        fconn.commit()
        return mid

    def test_promote_to_admin(self, fconn):
        mid = self._make_instructor(fconn, "Promo", is_admin=False)
        set_admin(fconn, mid, True)
        row = fconn.execute("SELECT is_admin FROM instructor WHERE member_id=?", (mid,)).fetchone()
        assert row["is_admin"] == 1

    def test_demote_from_admin(self, fconn):
        mid = self._make_instructor(fconn, "Demote", is_admin=True)
        set_admin(fconn, mid, False)
        row = fconn.execute("SELECT is_admin FROM instructor WHERE member_id=?", (mid,)).fetchone()
        assert row["is_admin"] == 0


# ---------------------------------------------------------------------------
# get/set_yjs_state
# ---------------------------------------------------------------------------

class TestYjsState:
    def _make_schedule(self, fconn):
        add_yjs_state_column(fconn)  # migration not run by init_db
        sid = create_season(fconn, 9001)
        fconn.execute("INSERT INTO schedule (season_id, config) VALUES (?, ?)", (sid, "{}"))
        fconn.commit()
        return fconn.execute("SELECT id FROM schedule WHERE season_id=?", (sid,)).fetchone()["id"]

    def test_returns_none_when_not_set(self, fconn):
        sch_id = self._make_schedule(fconn)
        assert get_yjs_state(fconn, sch_id) is None

    def test_returns_bytes_after_set(self, fconn):
        sch_id = self._make_schedule(fconn)
        payload = b"\x00\x01\x02state"
        set_yjs_state(fconn, sch_id, payload)
        result = get_yjs_state(fconn, sch_id)
        assert result == payload

    def test_returns_none_for_unknown_id(self, fconn):
        add_yjs_state_column(fconn)
        assert get_yjs_state(fconn, 99999) is None


# ---------------------------------------------------------------------------
# get/set_schedule_start_date
# ---------------------------------------------------------------------------

class TestScheduleStartDate:
    def _make_schedule(self, fconn):
        add_start_date_column(fconn)  # migration not run by init_db
        sid = create_season(fconn, 9002)
        fconn.execute("INSERT INTO schedule (season_id, config) VALUES (?, ?)", (sid, "{}"))
        fconn.commit()
        return fconn.execute("SELECT id FROM schedule WHERE season_id=?", (sid,)).fetchone()["id"]

    def test_returns_none_when_not_set(self, fconn):
        sch_id = self._make_schedule(fconn)
        assert get_schedule_start_date(fconn, sch_id) is None

    def test_set_and_get_roundtrip(self, fconn):
        sch_id = self._make_schedule(fconn)
        set_schedule_start_date(fconn, sch_id, "2099-07-01")
        assert get_schedule_start_date(fconn, sch_id) == "2099-07-01"

    def test_overwrite_date(self, fconn):
        sch_id = self._make_schedule(fconn)
        set_schedule_start_date(fconn, sch_id, "2099-07-01")
        set_schedule_start_date(fconn, sch_id, "2099-08-01")
        assert get_schedule_start_date(fconn, sch_id) == "2099-08-01"


# ---------------------------------------------------------------------------
# replace_assignments
# ---------------------------------------------------------------------------

class TestReplaceAssignments:
    def _seed(self, fconn):
        sid = create_season(fconn, 9003)
        for name in ("Alice", "Bob"):
            fconn.execute("INSERT INTO member (name) VALUES (?)", (name,))
        fconn.commit()
        alice_id = fconn.execute("SELECT id FROM member WHERE name='Alice'").fetchone()["id"]
        bob_id = fconn.execute("SELECT id FROM member WHERE name='Bob'").fetchone()["id"]
        fconn.executemany(
            "INSERT INTO enrollment (member_id, season_id, level) VALUES (?, ?, 'k')",
            [(alice_id, sid), (bob_id, sid)],
        )
        fconn.execute("INSERT INTO schedule (season_id, config) VALUES (?, ?)", (sid, "{}"))
        fconn.commit()
        sch_id = fconn.execute("SELECT id FROM schedule WHERE season_id=?", (sid,)).fetchone()["id"]
        fconn.executemany(
            "INSERT INTO boat_assignment (schedule_id, member_id, boat_id, day) VALUES (?, ?, 1, 'Sat')",
            [(sch_id, alice_id), (sch_id, bob_id)],
        )
        fconn.commit()
        return sch_id

    def test_replaces_all_rows(self, fconn):
        sch_id = self._seed(fconn)
        replace_assignments(fconn, sch_id, [("Alice", "Sat", 2), ("Bob", "Sat", 2)])
        rows = fconn.execute(
            "SELECT boat_id FROM boat_assignment WHERE schedule_id=?", (sch_id,)
        ).fetchall()
        assert all(r["boat_id"] == 2 for r in rows)

    def test_unknown_member_raises(self, fconn):
        sch_id = self._seed(fconn)
        with pytest.raises(RuntimeError, match="not found"):
            replace_assignments(fconn, sch_id, [("NoSuchMember", "Sat", 1)])
