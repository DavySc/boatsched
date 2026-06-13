"""Tests for production security hardening knobs on create_app()."""
import time

import pytest
from fastapi.testclient import TestClient

from boatsched import Config
from boatsched import db as _db
from boatsched.server import create_app

CFG = Config(
    boats=1, levels=["k"], restricted=["Sat"], diverse=["Fri"],
    stage_seconds=5, seconds=5,
)


def _seed_admin(db_conn, name="Admin", pw="adminpw123"):
    _db.add_admin_column(db_conn)
    _db.ensure_admin(db_conn, name, pw)
    return name, pw


# ---------------------------------------------------------------------------
# require_auth disables bootstrap/open mode entirely
# ---------------------------------------------------------------------------

class TestRequireAuth:
    @pytest.fixture
    def client(self, tmp_path):
        db_conn = _db.init_db(str(tmp_path / "ra.db"))
        _seed_admin(db_conn)
        app = create_app(CFG, db_conn, require_auth=True)
        with TestClient(app, raise_server_exceptions=True) as c:
            yield c

    def test_protected_route_blocked_without_login(self, client):
        assert client.get("/seasons").status_code == 401

    def test_anonymous_login_rejected(self, client):
        # In open mode any creds would be accepted; require_auth forbids that.
        r = client.post("/api/auth/login", json={"name": "x", "password": "y"})
        assert r.status_code == 401

    def test_seeded_admin_can_login(self, client):
        r = client.post("/api/auth/login",
                        json={"name": "Admin", "password": "adminpw123"})
        assert r.status_code == 200
        assert "bsession" in r.cookies

    def test_me_not_anonymous_admin(self, client):
        # Without login, /api/auth/me must be 401 — never the open-mode admin.
        assert client.get("/api/auth/me").status_code == 401

    def test_no_passwordless_member_grants_access(self, tmp_path):
        """A member who is an instructor but has no password cannot get in."""
        db_conn = _db.init_db(str(tmp_path / "ra2.db"))
        _seed_admin(db_conn)
        # Add a passwordless instructor
        db_conn.execute("INSERT INTO member (name) VALUES ('NoPw')")
        mid = db_conn.execute("SELECT id FROM member WHERE name='NoPw'").fetchone()["id"]
        db_conn.execute("INSERT INTO instructor (member_id, is_admin) VALUES (?, 0)", (mid,))
        db_conn.commit()
        app = create_app(CFG, db_conn, require_auth=True)
        with TestClient(app) as c:
            r = c.post("/api/auth/login", json={"name": "NoPw", "password": ""})
            assert r.status_code == 401


# ---------------------------------------------------------------------------
# Secure cookie flag
# ---------------------------------------------------------------------------

class TestSecureCookie:
    def test_secure_flag_set_when_enabled(self, tmp_path):
        db_conn = _db.init_db(str(tmp_path / "sc.db"))
        _seed_admin(db_conn)
        app = create_app(CFG, db_conn, require_auth=True, secure_cookies=True)
        with TestClient(app) as c:
            r = c.post("/api/auth/login",
                       json={"name": "Admin", "password": "adminpw123"})
            set_cookie = r.headers.get("set-cookie", "")
            assert "Secure" in set_cookie
            assert "HttpOnly" in set_cookie

    def test_secure_flag_absent_by_default(self, tmp_path):
        db_conn = _db.init_db(str(tmp_path / "sc2.db"))
        app = create_app(CFG, db_conn)  # dev defaults
        with TestClient(app) as c:
            r = c.post("/api/auth/login", json={"name": "x", "password": ""})
            assert "Secure" not in r.headers.get("set-cookie", "")


# ---------------------------------------------------------------------------
# Session TTL
# ---------------------------------------------------------------------------

class TestSessionTtl:
    def test_session_expires(self, tmp_path):
        db_conn = _db.init_db(str(tmp_path / "ttl.db"))
        _seed_admin(db_conn)
        app = create_app(CFG, db_conn, require_auth=True, session_ttl=1)
        with TestClient(app) as c:
            c.post("/api/auth/login",
                   json={"name": "Admin", "password": "adminpw123"})
            assert c.get("/seasons").status_code == 200
            time.sleep(1.2)
            assert c.get("/seasons").status_code == 401


# ---------------------------------------------------------------------------
# Login rate limiting
# ---------------------------------------------------------------------------

class TestLoginRateLimit:
    def test_blocks_after_max_failures(self, tmp_path):
        db_conn = _db.init_db(str(tmp_path / "rl.db"))
        _seed_admin(db_conn)
        app = create_app(CFG, db_conn, require_auth=True,
                         login_max_failures=3, login_window=300)
        with TestClient(app) as c:
            for _ in range(3):
                r = c.post("/api/auth/login",
                           json={"name": "Admin", "password": "wrong"})
                assert r.status_code == 401
            # 4th attempt is throttled, even with the *correct* password
            r = c.post("/api/auth/login",
                       json={"name": "Admin", "password": "adminpw123"})
            assert r.status_code == 429

    def test_successful_login_resets_counter(self, tmp_path):
        db_conn = _db.init_db(str(tmp_path / "rl2.db"))
        _seed_admin(db_conn)
        app = create_app(CFG, db_conn, require_auth=True,
                         login_max_failures=3, login_window=300)
        with TestClient(app) as c:
            c.post("/api/auth/login", json={"name": "Admin", "password": "wrong"})
            c.post("/api/auth/login", json={"name": "Admin", "password": "wrong"})
            # Correct login clears the counter
            ok = c.post("/api/auth/login",
                        json={"name": "Admin", "password": "adminpw123"})
            assert ok.status_code == 200
            c.post("/api/auth/logout")
            # Two more failures should not trip the limit (counter was reset)
            c.post("/api/auth/login", json={"name": "Admin", "password": "wrong"})
            r = c.post("/api/auth/login", json={"name": "Admin", "password": "wrong"})
            assert r.status_code == 401  # not yet blocked


# ---------------------------------------------------------------------------
# Upload size cap
# ---------------------------------------------------------------------------

class TestUploadSizeCap:
    def _admin_client(self, tmp_path, **kwargs):
        db_conn = _db.init_db(str(tmp_path / "up.db"))
        _seed_admin(db_conn)
        app = create_app(CFG, db_conn, require_auth=True,
                         max_upload_bytes=1000, **kwargs)
        c = TestClient(app)
        c.post("/api/auth/login", json={"name": "Admin", "password": "adminpw123"})
        return c

    def test_solve_rejects_oversized_body(self, tmp_path):
        c = self._admin_client(tmp_path)
        big = b"name,level\n" + b"x,k\n" * 5000  # > 1000 bytes
        r = c.post("/solve", content=big, headers={"Content-Type": "text/csv"})
        assert r.status_code == 413

    def test_import_rejects_oversized_body(self, tmp_path):
        c = self._admin_client(tmp_path)
        big = b"name\n" + b"someverylongname\n" * 1000
        r = c.post("/api/members/import", content=big,
                   headers={"Content-Type": "text/csv"})
        assert r.status_code == 413

    def test_normal_upload_accepted(self, tmp_path):
        c = self._admin_client(tmp_path)
        small = b"name,level\nAlice,k\nBob,k\n"
        r = c.post("/solve", content=small, headers={"Content-Type": "text/csv"})
        assert r.status_code == 200


# ---------------------------------------------------------------------------
# Password policy
# ---------------------------------------------------------------------------

class TestPasswordPolicy:
    @pytest.fixture
    def client(self, tmp_path):
        db_conn = _db.init_db(str(tmp_path / "pp.db"))
        _seed_admin(db_conn)
        app = create_app(CFG, db_conn, require_auth=True, min_password_len=8)
        c = TestClient(app)
        c.post("/api/auth/login", json={"name": "Admin", "password": "adminpw123"})
        return c

    def test_create_instructor_rejects_short_password(self, client):
        r = client.post("/instructor", json={"name": "New", "password": "short"})
        assert r.status_code == 400

    def test_create_instructor_accepts_long_password(self, client):
        r = client.post("/instructor", json={"name": "New", "password": "longenough1"})
        assert r.status_code == 200

    def test_set_password_rejects_short(self, client):
        r = client.post("/set-password", json={"name": "Admin", "password": "abc"})
        assert r.status_code == 400


# ---------------------------------------------------------------------------
# Security headers and public health
# ---------------------------------------------------------------------------

class TestSecurityHeaders:
    @pytest.fixture
    def client(self, tmp_path):
        db_conn = _db.init_db(str(tmp_path / "sh.db"))
        _seed_admin(db_conn)
        app = create_app(CFG, db_conn, require_auth=True)
        with TestClient(app) as c:
            yield c

    def test_headers_present_on_response(self, client):
        r = client.get("/api/health")
        assert r.headers.get("x-content-type-options") == "nosniff"
        assert r.headers.get("x-frame-options") == "DENY"
        assert r.headers.get("referrer-policy") == "no-referrer"

    def test_health_is_public(self, client):
        # No login — still reachable
        assert client.get("/api/health").status_code == 200

    def test_health_has_no_sensitive_data(self, client):
        data = client.get("/api/health").json()
        assert set(data.keys()) <= {"status", "timestamp", "version"}


# ---------------------------------------------------------------------------
# Generic 500 errors (no internal detail leaked)
# ---------------------------------------------------------------------------

class TestGenericErrors:
    def test_duplicate_season_500_is_generic(self, tmp_path):
        db_conn = _db.init_db(str(tmp_path / "ge.db"))
        _seed_admin(db_conn)
        app = create_app(CFG, db_conn, require_auth=True)
        with TestClient(app) as c:
            c.post("/api/auth/login",
                   json={"name": "Admin", "password": "adminpw123"})
            c.post("/seasons", json={"year": 2222})
            r = c.post("/seasons", json={"year": 2222})  # duplicate → 500
            assert r.status_code == 500
            # Must not leak the SQLite UNIQUE-constraint message
            assert r.json()["error"] == "internal server error"
            assert "UNIQUE" not in str(r.json())


# ---------------------------------------------------------------------------
# ensure_admin (db helper)
# ---------------------------------------------------------------------------

class TestEnsureAdmin:
    def test_creates_admin_with_password(self, tmp_path):
        db_conn = _db.init_db(str(tmp_path / "ea.db"))
        _db.add_admin_column(db_conn)
        created = _db.ensure_admin(db_conn, "Boss", "bosspw1234")
        assert created is True
        assert _db.verify_instructor_login(db_conn, "Boss", "bosspw1234") is not None
        row = db_conn.execute(
            "SELECT is_admin FROM instructor i JOIN member m ON m.id=i.member_id"
            " WHERE m.name='Boss'").fetchone()
        assert row["is_admin"] == 1

    def test_idempotent_keeps_existing_password(self, tmp_path):
        db_conn = _db.init_db(str(tmp_path / "ea2.db"))
        _db.add_admin_column(db_conn)
        _db.ensure_admin(db_conn, "Boss", "originalpw1")
        # Second call with a different password must NOT overwrite
        created = _db.ensure_admin(db_conn, "Boss", "differentpw2")
        assert created is False
        assert _db.verify_instructor_login(db_conn, "Boss", "originalpw1") is not None
        assert _db.verify_instructor_login(db_conn, "Boss", "differentpw2") is None

    def test_promotes_existing_member(self, tmp_path):
        db_conn = _db.init_db(str(tmp_path / "ea3.db"))
        _db.add_admin_column(db_conn)
        db_conn.execute("INSERT INTO member (name) VALUES ('Existing')")
        db_conn.commit()
        _db.ensure_admin(db_conn, "Existing", "newadminpw1")
        # No duplicate member row created
        count = db_conn.execute(
            "SELECT COUNT(*) FROM member WHERE name='Existing'").fetchone()[0]
        assert count == 1

    def test_empty_password_raises(self, tmp_path):
        db_conn = _db.init_db(str(tmp_path / "ea4.db"))
        _db.add_admin_column(db_conn)
        with pytest.raises(ValueError):
            _db.ensure_admin(db_conn, "Boss", "")


# ---------------------------------------------------------------------------
# main() production bootstrap (env-var admin seeding)
# ---------------------------------------------------------------------------

class TestMainBootstrap:
    def test_main_seeds_admin_from_env(self, tmp_path, monkeypatch):
        from unittest.mock import patch
        from boatsched.server import main
        db_path = str(tmp_path / "boot.db")
        monkeypatch.setenv("BOATSCHED_ADMIN", "Skipper")
        monkeypatch.setenv("BOATSCHED_ADMIN_PW", "skipperpw123")
        with patch("boatsched.server.uvicorn.run"):
            rc = main(["--db", db_path, "--boats", "1"])
        assert rc == 0
        conn = _db.init_db(db_path)
        # The seeded admin can authenticate and is flagged admin.
        assert _db.verify_instructor_login(conn, "Skipper", "skipperpw123") is not None
        row = conn.execute(
            "SELECT is_admin FROM instructor i JOIN member m ON m.id=i.member_id"
            " WHERE m.name='Skipper'").fetchone()
        assert row["is_admin"] == 1

    def test_main_warns_when_no_admin_and_require_auth(self, tmp_path, monkeypatch, capsys):
        from unittest.mock import patch
        from boatsched.server import main
        db_path = str(tmp_path / "boot2.db")
        monkeypatch.delenv("BOATSCHED_ADMIN", raising=False)
        monkeypatch.delenv("BOATSCHED_ADMIN_PW", raising=False)
        with patch("boatsched.server.uvicorn.run"):
            rc = main(["--db", db_path, "--boats", "1"])
        assert rc == 0
        err = capsys.readouterr().err
        assert "nobody" in err.lower() or "no admin" in err.lower()


# ---------------------------------------------------------------------------
# WebSocket frame-size cap (collab DoS guard)
# ---------------------------------------------------------------------------

class TestWebSocketSizeCap:
    def _make_schedule(self, db_conn, app_client):
        import json
        # create season + solve to produce a committed schedule with yjs state
        r = app_client.post("/seasons", json={"year": 2399})
        sid = r.json()["id"]
        roster = b"name,level\nAlice,k\nBob,k\n"
        sub = app_client.post(f"/solve?season_id={sid}", content=roster,
                              headers={"Content-Type": "text/csv"})
        jid = sub.json()["id"]
        for _ in range(300):
            j = app_client.get(f"/status?id={jid}").json()
            if j["status"] != "running":
                break
            time.sleep(0.05)
        assert j["status"] == "done"
        return j["schedule_id"]

    def test_oversized_frame_is_dropped(self, tmp_path):
        import pycrdt
        from boatsched.server import (
            _encode_sync_step1, _encode_update_msg, _parse_message, SYNC_STEP2,
        )
        db_conn = _db.init_db(str(tmp_path / "ws.db"))
        # cap frames at 200 bytes for the test
        app = create_app(CFG, db_conn, max_ws_message_bytes=200)
        with TestClient(app, raise_server_exceptions=True) as client:
            schedule_id = self._make_schedule(db_conn, client)
            with client.websocket_connect(f"/collab/{schedule_id}") as ws:
                ws.receive_bytes()  # server step1
                doc = pycrdt.Doc()
                ws.send_bytes(_encode_sync_step1(doc.get_state()))
                reply = ws.receive_bytes()
                _, sub, update = _parse_message(reply)
                if sub == SYNC_STEP2:
                    doc.apply_update(update)
                # Craft an oversized update (a big string value)
                sv = doc.get_state()
                sch = doc.get("schedule", type=pycrdt.Map)
                with doc.transaction():
                    sch["_huge"] = pycrdt.Array(["x" * 5000])
                big = _encode_update_msg(doc.get_update(sv))
                assert len(big) > 200
                ws.send_bytes(big)
                # Give the server a moment; the frame must NOT be persisted.
                time.sleep(0.3)
        # Re-open the persisted state — the oversized key must be absent.
        state = _db.get_yjs_state(db_conn, schedule_id)
        check = pycrdt.Doc()
        check.apply_update(state)
        assert "_huge" not in check.get("schedule", type=pycrdt.Map)

