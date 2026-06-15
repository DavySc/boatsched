"""Tests for admin-only boat-assignment authz and student level editing.

Covers the eight required scenarios:
  1. boat assignment succeeds for admin, schedule unlocked
  2. boat assignment 403 for non-admin, schedule unlocked
  3. boat assignment 403 for non-admin, schedule locked
  4. boat assignment succeeds for admin, schedule locked
  5. level update succeeds for admin
  6. level update 403 for non-admin (locked and unlocked)
  7. export returns only non-instructor members
  8. export 403 for non-admin
"""
import csv
import io
import time

import pytest
from fastapi.testclient import TestClient

from boatsched import Config
from boatsched import db as _db
from boatsched.server import create_app

CFG = Config(
    boats=2, levels=["k", "b"],
    restricted=["Sat"], diverse=["Fri"],
    stage_seconds=15, seconds=15,
)

ROSTER_CSV = b"name,level\nAlice,k\nBob,k\nCarol,b\nDave,b\n"


def _make_instructor(db_conn, name, password, is_admin=False):
    db_conn.execute("INSERT INTO member (name) VALUES (?)", (name,))
    mid = db_conn.execute(
        "SELECT id FROM member WHERE name=?", (name,)).fetchone()["id"]
    db_conn.execute("INSERT INTO instructor (member_id, is_admin) VALUES (?, ?)",
                    (mid, 1 if is_admin else 0))
    db_conn.commit()
    _db.set_instructor_password(db_conn, mid, password)
    return mid


def _login(client, name, password):
    return client.post("/api/auth/login", json={"name": name, "password": password})


@pytest.fixture
def auth_db(tmp_path):
    db_conn = _db.init_db(str(tmp_path / "lvl.db"))
    _db.add_admin_column(db_conn)
    _make_instructor(db_conn, "Admin", "adminpw", is_admin=True)
    _make_instructor(db_conn, "NonAdmin", "nonadminpw", is_admin=False)
    # Returning (non-k) students must pre-exist before the roster solve.
    db_conn.executemany("INSERT INTO member (name) VALUES (?)",
                        [("Carol",), ("Dave",)])
    db_conn.commit()
    app = create_app(CFG, db_conn)
    with TestClient(app, raise_server_exceptions=True) as client:
        yield client, db_conn


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def _solve_schedule(client, db_conn, year):
    """As the logged-in admin: create a season, solve the roster, return ids."""
    client.post("/seasons", json={"year": year})
    sid = db_conn.execute(
        "SELECT id FROM season WHERE year=?", (year,)).fetchone()["id"]
    r = client.post(f"/solve?season_id={sid}", content=ROSTER_CSV,
                    headers={"Content-Type": "text/csv"})
    jid = r.json()["id"]
    deadline = time.time() + 60
    while time.time() < deadline:
        j = client.get(f"/status?id={jid}").json()
        if j["status"] != "running":
            break
        time.sleep(0.2)
    assert j["status"] == "done", j
    return sid, j["schedule_id"]


def _sat_pair(client, schedule_id):
    """Two students on Sat who are in different boats — a valid swap pair."""
    rows = client.get(f"/schedule?id={schedule_id}").json()["rows"]
    sat = [r for r in rows if r["day"] == "Sat"]
    a = sat[0]
    b = next(r for r in sat[1:] if r["boat"] != a["boat"])
    return a["name"], b["name"]


def _lock(client, schedule_id):
    """Set a start date in the past so the schedule is 'locked'."""
    r = client.post(f"/schedule/{schedule_id}/start-date",
                    json={"start_date": "2000-01-01"})
    assert r.status_code == 200


# ---------------------------------------------------------------------------
# Part 1 — boat assignment authorization (scenarios 1–4)
# ---------------------------------------------------------------------------

class TestBoatAssignmentAuthz:
    def test_1_admin_unlocked_swap_succeeds(self, auth_db):
        client, db_conn = auth_db
        _login(client, "Admin", "adminpw")
        _, schedule_id = _solve_schedule(client, db_conn, 5001)
        a, b = _sat_pair(client, schedule_id)
        r = client.post("/swap", json={"schedule_id": schedule_id, "day": "Sat",
                                       "name_a": a, "name_b": b})
        assert r.status_code == 200
        assert r.json()["ok"] is True

    def test_2_non_admin_unlocked_swap_403(self, auth_db):
        client, db_conn = auth_db
        _login(client, "Admin", "adminpw")
        _, schedule_id = _solve_schedule(client, db_conn, 5002)
        a, b = _sat_pair(client, schedule_id)
        _login(client, "NonAdmin", "nonadminpw")   # switch to non-admin
        r = client.post("/swap", json={"schedule_id": schedule_id, "day": "Sat",
                                       "name_a": a, "name_b": b})
        assert r.status_code == 403
        assert r.json()["detail"] == "Only admins may edit boat assignments"

    def test_3_non_admin_locked_swap_403(self, auth_db):
        client, db_conn = auth_db
        _login(client, "Admin", "adminpw")
        _, schedule_id = _solve_schedule(client, db_conn, 5003)
        a, b = _sat_pair(client, schedule_id)
        _lock(client, schedule_id)
        _login(client, "NonAdmin", "nonadminpw")
        r = client.post("/swap", json={"schedule_id": schedule_id, "day": "Sat",
                                       "name_a": a, "name_b": b})
        assert r.status_code == 403

    def test_4_admin_locked_swap_succeeds(self, auth_db):
        client, db_conn = auth_db
        _login(client, "Admin", "adminpw")
        _, schedule_id = _solve_schedule(client, db_conn, 5004)
        a, b = _sat_pair(client, schedule_id)
        _lock(client, schedule_id)
        # Admin bypasses the start-date lock.
        r = client.post("/swap", json={"schedule_id": schedule_id, "day": "Sat",
                                       "name_a": a, "name_b": b})
        assert r.status_code == 200
        assert r.json()["ok"] is True

    def test_non_admin_commit_403(self, auth_db):
        client, db_conn = auth_db
        _login(client, "Admin", "adminpw")
        _, schedule_id = _solve_schedule(client, db_conn, 5005)
        _login(client, "NonAdmin", "nonadminpw")
        r = client.post(f"/schedule/{schedule_id}/commit")
        assert r.status_code == 403
        assert r.json()["detail"] == "Only admins may edit boat assignments"


# ---------------------------------------------------------------------------
# Part 2 — student level editing (scenarios 5–6)
# ---------------------------------------------------------------------------

class TestLevelEditing:
    def _student_id(self, db_conn, name):
        return db_conn.execute(
            "SELECT id FROM member WHERE name=?", (name,)).fetchone()["id"]

    def test_5_admin_level_update_succeeds_unlocked(self, auth_db):
        # Scenario 5 — exercised on an UNLOCKED schedule.
        client, db_conn = auth_db
        _login(client, "Admin", "adminpw")
        season_id, _ = _solve_schedule(client, db_conn, 5101)
        carol = self._student_id(db_conn, "Carol")  # enrolled at level 'b'
        r = client.patch(f"/enrollment/{season_id}/member/{carol}/level",
                         json={"level": "A"})
        assert r.status_code == 200
        assert r.json()["level"] == "A"
        lvl = db_conn.execute(
            "SELECT level FROM enrollment WHERE member_id=? AND season_id=?",
            (carol, season_id)).fetchone()["level"]
        assert lvl == "A"

    def test_6_non_admin_level_update_403_unlocked(self, auth_db):
        client, db_conn = auth_db
        _login(client, "Admin", "adminpw")
        season_id, _ = _solve_schedule(client, db_conn, 5102)
        carol = self._student_id(db_conn, "Carol")
        _login(client, "NonAdmin", "nonadminpw")
        r = client.patch(f"/enrollment/{season_id}/member/{carol}/level",
                         json={"level": "A"})
        assert r.status_code == 403
        assert r.json()["detail"] == "Only admins may edit student levels"

    def test_6_non_admin_level_update_403_locked(self, auth_db):
        # Levels are admin-only regardless of lock state.
        client, db_conn = auth_db
        _login(client, "Admin", "adminpw")
        season_id, schedule_id = _solve_schedule(client, db_conn, 5103)
        carol = self._student_id(db_conn, "Carol")
        _lock(client, schedule_id)
        _login(client, "NonAdmin", "nonadminpw")
        r = client.patch(f"/enrollment/{season_id}/member/{carol}/level",
                         json={"level": "A"})
        assert r.status_code == 403
        assert r.json()["detail"] == "Only admins may edit student levels"

    def test_invalid_level_rejected_400(self, auth_db):
        client, db_conn = auth_db
        _login(client, "Admin", "adminpw")
        season_id, _ = _solve_schedule(client, db_conn, 5104)
        carol = self._student_id(db_conn, "Carol")
        r = client.patch(f"/enrollment/{season_id}/member/{carol}/level",
                         json={"level": "beginner"})
        assert r.status_code == 400

    def test_instructor_not_a_student_404(self, auth_db):
        # An instructor enrolled in the season is not a "student" → 404.
        client, db_conn = auth_db
        _login(client, "Admin", "adminpw")
        season_id, _ = _solve_schedule(client, db_conn, 5105)
        admin_mid = db_conn.execute(
            "SELECT id FROM member WHERE name=?", ("Admin",)).fetchone()["id"]
        _db.add_enrollment(db_conn, admin_mid, season_id, "b")
        r = client.patch(f"/enrollment/{season_id}/member/{admin_mid}/level",
                         json={"level": "A"})
        assert r.status_code == 404

    def test_member_not_enrolled_404(self, auth_db):
        client, db_conn = auth_db
        _login(client, "Admin", "adminpw")
        season_id, _ = _solve_schedule(client, db_conn, 5106)
        db_conn.execute("INSERT INTO member (name) VALUES ('Ghost')")
        db_conn.commit()
        ghost = db_conn.execute(
            "SELECT id FROM member WHERE name='Ghost'").fetchone()["id"]
        r = client.patch(f"/enrollment/{season_id}/member/{ghost}/level",
                         json={"level": "A"})
        assert r.status_code == 404


# ---------------------------------------------------------------------------
# Part 2 — CSV export (scenarios 7–8)
# ---------------------------------------------------------------------------

class TestStudentExport:
    def test_7_export_excludes_instructors(self, auth_db):
        client, db_conn = auth_db
        _login(client, "Admin", "adminpw")
        season_id, _ = _solve_schedule(client, db_conn, 5201)
        # Enroll the NonAdmin instructor in the season; export must exclude them.
        nonadmin = db_conn.execute(
            "SELECT id FROM member WHERE name=?", ("NonAdmin",)).fetchone()["id"]
        _db.add_enrollment(db_conn, nonadmin, season_id, "b")

        r = client.get(f"/enrollment/{season_id}/students/export")
        assert r.status_code == 200
        assert r.headers["content-type"].startswith("text/csv")
        reader = list(csv.reader(io.StringIO(r.text)))
        assert reader[0] == ["name", "level"]
        names = {row[0] for row in reader[1:]}
        assert names == {"Alice", "Bob", "Carol", "Dave"}
        assert "NonAdmin" not in names
        assert "Admin" not in names

    def test_8_export_403_for_non_admin(self, auth_db):
        client, db_conn = auth_db
        _login(client, "Admin", "adminpw")
        season_id, _ = _solve_schedule(client, db_conn, 5202)
        _login(client, "NonAdmin", "nonadminpw")
        r = client.get(f"/enrollment/{season_id}/students/export")
        assert r.status_code == 403
        assert r.json()["detail"] == "Only admins may export students"
