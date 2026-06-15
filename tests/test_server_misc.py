"""Tests for miscellaneous server.py paths not covered elsewhere."""
import json
import time
from unittest.mock import patch

import pytest
from fastapi.testclient import TestClient

from boatsched import Config
from boatsched import db as _db
from boatsched.server import _parse_names_csv, create_app, parse_args

CFG = Config(
    boats=2, levels=["k", "b"],
    restricted=["Sat"], diverse=["Fri"],
    stage_seconds=15, seconds=15,
)

ROSTER_CSV = b"name,level\nAlice,k\nBob,k\nCarol,b\nDave,b\n"


def _make_instructor(db_conn, name, password, is_admin=False):
    db_conn.execute("INSERT INTO member (name) VALUES (?)", (name,))
    mid = db_conn.execute("SELECT id FROM member WHERE name=?", (name,)).fetchone()["id"]
    db_conn.execute("INSERT INTO instructor (member_id, is_admin) VALUES (?, ?)",
                    (mid, 1 if is_admin else 0))
    db_conn.commit()
    _db.set_instructor_password(db_conn, mid, password)
    return mid


def _login(client, name, password):
    return client.post("/api/auth/login", json={"name": name, "password": password})


@pytest.fixture
def auth_db(tmp_path):
    db_conn = _db.init_db(str(tmp_path / "misc.db"))
    _db.add_admin_column(db_conn)  # migration normally run by create_app
    _make_instructor(db_conn, "Admin", "adminpw", is_admin=True)
    _make_instructor(db_conn, "NonAdmin", "nonadminpw", is_admin=False)
    db_conn.executemany("INSERT INTO member (name) VALUES (?)",
                        [("Carol",), ("Dave",)])
    db_conn.commit()
    app = create_app(CFG, db_conn)
    with TestClient(app, raise_server_exceptions=True) as client:
        yield client, db_conn


# ---------------------------------------------------------------------------
# _parse_names_csv
# ---------------------------------------------------------------------------

class TestParseNamesCsv:
    def test_name_column_extracted(self):
        csv = "name,level\nAlice,k\nBob,b\n"
        assert _parse_names_csv(csv) == ["Alice", "Bob"]

    def test_name_column_case_insensitive(self):
        csv = "Name,level\nCarol,k\n"
        assert _parse_names_csv(csv) == ["Carol"]

    def test_fallback_first_token_when_no_name_header(self):
        csv = "Alice\nBob\nCarol\n"
        result = _parse_names_csv(csv)
        assert "Alice" in result and "Bob" in result and "Carol" in result

    def test_skips_header_in_fallback_mode(self):
        csv = "name\nAlice\nBob\n"
        result = _parse_names_csv(csv)
        assert "name" not in [r.lower() for r in result]

    def test_empty_string_returns_empty(self):
        assert _parse_names_csv("") == []

    def test_blank_lines_ignored(self):
        csv = "name\nAlice\n\n\nBob\n"
        result = _parse_names_csv(csv)
        assert "" not in result
        assert "Alice" in result and "Bob" in result

    def test_csv_with_comma_separated_first_column(self):
        csv = "Alice,some,extra\nBob,more\n"
        result = _parse_names_csv(csv)
        assert "Alice" in result and "Bob" in result
        # extra columns not included
        assert "some" not in result and "extra" not in result


# ---------------------------------------------------------------------------
# GET /api/auth/me
# ---------------------------------------------------------------------------

class TestApiMe:
    def test_unauthenticated_returns_401(self, auth_db):
        client, _ = auth_db
        r = client.get("/api/auth/me")
        assert r.status_code == 401

    def test_admin_is_admin_true(self, auth_db):
        client, _ = auth_db
        _login(client, "Admin", "adminpw")
        r = client.get("/api/auth/me")
        assert r.status_code == 200
        data = r.json()
        assert data["is_admin"] is True
        assert data["name"] == "Admin"

    def test_non_admin_is_admin_false(self, auth_db):
        client, _ = auth_db
        _login(client, "NonAdmin", "nonadminpw")
        r = client.get("/api/auth/me")
        assert r.status_code == 200
        assert r.json()["is_admin"] is False

    def test_open_mode_is_admin_true(self, tmp_path):
        db_conn = _db.init_db(str(tmp_path / "open.db"))
        app = create_app(CFG, db_conn)
        with TestClient(app) as client:
            # open mode — login with anything
            client.post("/api/auth/login", json={"name": "x", "password": ""})
            r = client.get("/api/auth/me")
            assert r.status_code == 200
            assert r.json()["is_admin"] is True

    def test_me_returns_member_id(self, auth_db):
        client, db_conn = auth_db
        _login(client, "Admin", "adminpw")
        r = client.get("/api/auth/me")
        expected_id = db_conn.execute(
            "SELECT id FROM member WHERE name='Admin'"
        ).fetchone()["id"]
        assert r.json()["member_id"] == expected_id


# ---------------------------------------------------------------------------
# POST /api/auth/login — edge cases
# ---------------------------------------------------------------------------

class TestApiLoginEdgeCases:
    def test_bad_body_returns_400(self, auth_db):
        client, _ = auth_db
        r = client.post("/api/auth/login", content=b"not json",
                        headers={"Content-Type": "application/json"})
        assert r.status_code == 400

    def test_wrong_password_no_cookie(self, auth_db):
        client, _ = auth_db
        r = _login(client, "Admin", "wrongpassword")
        assert r.status_code == 401
        assert "bsession" not in r.cookies


# ---------------------------------------------------------------------------
# POST /api/auth/logout — edge cases
# ---------------------------------------------------------------------------

class TestApiLogoutEdgeCases:
    def test_logout_without_cookie_still_returns_ok(self, auth_db):
        client, _ = auth_db
        # Send logout with no session cookie at all
        r = client.post("/api/auth/logout", cookies={})
        assert r.status_code == 200
        assert r.json()["ok"] is True

    def test_logout_clears_cookie(self, auth_db):
        client, _ = auth_db
        _login(client, "Admin", "adminpw")
        client.post("/api/auth/logout")
        r = client.get("/api/auth/me")
        assert r.status_code == 401


# ---------------------------------------------------------------------------
# No-db responses for various endpoints
# ---------------------------------------------------------------------------

class TestNoDbEndpoints:
    @pytest.fixture
    def no_db_client(self):
        app = create_app(CFG)
        with TestClient(app) as client:
            yield client

    def test_get_instructors_returns_empty(self, no_db_client):
        r = no_db_client.get("/instructors")
        assert r.status_code == 200
        assert r.json() == []

    def test_create_instructor_returns_503(self, no_db_client):
        r = no_db_client.post("/instructor", json={"name": "X", "password": "pw"})
        assert r.status_code == 503

    def test_get_boats_returns_empty(self, no_db_client):
        r = no_db_client.get("/boats")
        assert r.status_code == 200
        assert r.json() == []

    def test_rename_boat_returns_503(self, no_db_client):
        r = no_db_client.post("/boat", json={"id": 1, "name": "Boat1"})
        assert r.status_code == 503

    def test_get_schedule_missing_id_returns_400(self, no_db_client):
        # With no DB this returns 503; but with DB, no id → 400. Test both:
        r = no_db_client.get("/schedule")
        assert r.status_code == 503

    def test_set_password_returns_503(self, no_db_client):
        r = no_db_client.post("/set-password", json={"name": "X", "password": "pw"})
        assert r.status_code == 503


# ---------------------------------------------------------------------------
# GET /schedule — missing id with DB
# ---------------------------------------------------------------------------

class TestGetScheduleEdgeCases:
    def test_missing_id_returns_400(self, auth_db):
        client, _ = auth_db
        _login(client, "Admin", "adminpw")
        r = client.get("/schedule")
        assert r.status_code == 400

    def test_nonexistent_id_returns_empty_rows(self, auth_db):
        client, _ = auth_db
        _login(client, "Admin", "adminpw")
        r = client.get("/schedule?id=99999")
        assert r.status_code == 200
        assert r.json()["rows"] == []


# ---------------------------------------------------------------------------
# POST /set-password edge cases
# ---------------------------------------------------------------------------

class TestSetPasswordEdgeCases:
    def test_bad_body_returns_400(self, auth_db):
        client, _ = auth_db
        _login(client, "Admin", "adminpw")
        r = client.post("/set-password", content=b"not json",
                        headers={"Content-Type": "application/json"})
        assert r.status_code == 400

    def test_empty_password_returns_400(self, auth_db):
        client, _ = auth_db
        _login(client, "Admin", "adminpw")
        r = client.post("/set-password", json={"name": "Admin", "password": ""})
        assert r.status_code == 400

    def test_non_admin_changing_other_password_returns_403(self, auth_db):
        client, _ = auth_db
        _login(client, "NonAdmin", "nonadminpw")
        r = client.post("/set-password", json={"name": "Admin", "password": "hacked"})
        assert r.status_code == 403

    def test_non_admin_can_change_own_password(self, auth_db):
        client, db_conn = auth_db
        _login(client, "NonAdmin", "nonadminpw")
        r = client.post("/set-password", json={"name": "NonAdmin", "password": "newpw"})
        assert r.status_code == 200
        assert r.json()["ok"] is True
        assert _db.verify_instructor_login(db_conn, "NonAdmin", "newpw") is not None


# ---------------------------------------------------------------------------
# POST /seasons — admin guard
# ---------------------------------------------------------------------------

class TestCreateSeasonAdminGuard:
    def test_non_admin_cannot_create_season(self, auth_db):
        client, _ = auth_db
        _login(client, "NonAdmin", "nonadminpw")
        r = client.post("/seasons", json={"year": 3000})
        assert r.status_code == 403

    def test_admin_can_create_season(self, auth_db):
        client, _ = auth_db
        _login(client, "Admin", "adminpw")
        r = client.post("/seasons", json={"year": 3001})
        assert r.status_code == 200
        assert r.json()["year"] == 3001


# ---------------------------------------------------------------------------
# Rename boat endpoint
# ---------------------------------------------------------------------------

class TestRenameBoat:
    def test_rename_boat_success(self, auth_db):
        client, db_conn = auth_db
        _login(client, "Admin", "adminpw")
        boats = client.get("/boats").json()
        bid = boats[0]["id"]
        r = client.post("/boat", json={"id": bid, "name": "MyBoat"})
        assert r.status_code == 200
        assert r.json()["ok"] is True
        row = db_conn.execute("SELECT name FROM boat WHERE boat_id=?", (bid,)).fetchone()
        assert row["name"] == "MyBoat"

    def test_rename_boat_bad_body_returns_400(self, auth_db):
        client, _ = auth_db
        _login(client, "Admin", "adminpw")
        r = client.post("/boat", content=b"not json",
                        headers={"Content-Type": "application/json"})
        assert r.status_code == 400


# ---------------------------------------------------------------------------
# POST /schedule/{id}/start-date and locking
# ---------------------------------------------------------------------------

class TestStartDateAndLocking:
    def _solve_and_get_schedule(self, client, db_conn, year):
        client.post("/seasons", json={"year": year})
        sid = db_conn.execute("SELECT id FROM season WHERE year=?", (year,)).fetchone()["id"]
        r = client.post(f"/solve?season_id={sid}", content=ROSTER_CSV,
                        headers={"Content-Type": "text/csv"})
        jid = r.json()["id"]
        deadline = time.time() + 60
        while time.time() < deadline:
            j = client.get(f"/status?id={jid}").json()
            if j["status"] != "running":
                break
            time.sleep(0.2)
        assert j["status"] == "done"
        return j["schedule_id"]

    def test_set_start_date_returns_ok(self, auth_db):
        client, db_conn = auth_db
        _login(client, "Admin", "adminpw")
        schedule_id = self._solve_and_get_schedule(client, db_conn, 4001)
        r = client.post(f"/schedule/{schedule_id}/start-date",
                        json={"start_date": "2099-01-01"})
        assert r.status_code == 200
        assert r.json()["ok"] is True

    def test_start_date_persisted_in_db(self, auth_db):
        client, db_conn = auth_db
        _login(client, "Admin", "adminpw")
        schedule_id = self._solve_and_get_schedule(client, db_conn, 4002)
        client.post(f"/schedule/{schedule_id}/start-date",
                    json={"start_date": "2099-06-01"})
        row = db_conn.execute("SELECT start_date FROM schedule WHERE id=?",
                              (schedule_id,)).fetchone()
        assert row["start_date"] == "2099-06-01"

    def test_past_start_date_does_not_block_admin_swap(self, auth_db):
        # Boat assignments are now admin-only and admins bypass the start-date
        # lock entirely (feature: student level editing, Part 1). A past start
        # date no longer freezes edits for an admin.
        client, db_conn = auth_db
        _login(client, "Admin", "adminpw")
        schedule_id = self._solve_and_get_schedule(client, db_conn, 4003)
        client.post(f"/schedule/{schedule_id}/start-date",
                    json={"start_date": "2000-01-01"})
        rows = client.get(f"/schedule?id={schedule_id}").json()["rows"]
        sat = [r for r in rows if r["day"] == "Sat"]
        a, b = sat[0]["name"], next(r["name"] for r in sat[1:] if r["boat"] != sat[0]["boat"])
        r = client.post("/swap", json={"schedule_id": schedule_id, "day": "Sat",
                                       "name_a": a, "name_b": b})
        assert r.status_code == 200
        assert r.json()["ok"] is True

    def test_past_start_date_does_not_block_admin_commit(self, auth_db):
        client, db_conn = auth_db
        _login(client, "Admin", "adminpw")
        schedule_id = self._solve_and_get_schedule(client, db_conn, 4004)
        client.post(f"/schedule/{schedule_id}/start-date",
                    json={"start_date": "2000-01-01"})
        r = client.post(f"/schedule/{schedule_id}/commit")
        assert r.status_code == 200

    def test_no_db_start_date_returns_503(self, tmp_path):
        app = create_app(CFG)
        with TestClient(app) as client:
            r = client.post("/schedule/1/start-date", json={"start_date": "2099-01-01"})
            assert r.status_code == 503


# ---------------------------------------------------------------------------
# POST /solve — admin guard in auth mode
# ---------------------------------------------------------------------------

class TestSolveAdminGuard:
    def test_non_admin_cannot_solve(self, auth_db):
        client, _ = auth_db
        _login(client, "NonAdmin", "nonadminpw")
        r = client.post("/solve", content=ROSTER_CSV,
                        headers={"Content-Type": "text/csv"})
        assert r.status_code == 403

    def test_admin_can_solve(self, auth_db):
        client, _ = auth_db
        _login(client, "Admin", "adminpw")
        r = client.post("/solve", content=ROSTER_CSV,
                        headers={"Content-Type": "text/csv"})
        assert r.status_code == 200
        assert "id" in r.json()


# ---------------------------------------------------------------------------
# GET /api/health
# ---------------------------------------------------------------------------

class TestApiHealth:
    def test_health_returns_healthy(self, auth_db):
        client, _ = auth_db
        _login(client, "Admin", "adminpw")
        r = client.get("/api/health")
        assert r.status_code == 200
        assert r.json()["status"] == "healthy"

    def test_health_has_timestamp(self, auth_db):
        client, _ = auth_db
        _login(client, "Admin", "adminpw")
        r = client.get("/api/health")
        assert "timestamp" in r.json()


# ---------------------------------------------------------------------------
# parse_args
# ---------------------------------------------------------------------------

class TestParseArgs:
    def test_defaults(self):
        a = parse_args([])
        assert a.port == 8000
        assert a.boats == 13
        assert a.db is None

    def test_custom_values(self):
        a = parse_args(["--port", "9000", "--boats", "5", "--db", "/tmp/x.db"])
        assert a.port == 9000
        assert a.boats == 5
        assert a.db == "/tmp/x.db"

    def test_levels_list(self):
        a = parse_args(["--levels", "k", "b"])
        assert a.levels == ["k", "b"]


# ---------------------------------------------------------------------------
# main()
# ---------------------------------------------------------------------------

class TestMain:
    def test_main_calls_uvicorn_run(self, tmp_path):
        from boatsched.server import main
        db = str(tmp_path / "main.db")
        with patch("boatsched.server.uvicorn.run") as mock_run:
            main(["--port", "9999", "--boats", "2", "--db", db])
            assert mock_run.called
            _, kwargs = mock_run.call_args
            assert kwargs.get("port") == 9999

    def test_main_without_db(self):
        from boatsched.server import main
        with patch("boatsched.server.uvicorn.run"):
            result = main(["--port", "9998"])
            assert result == 0
