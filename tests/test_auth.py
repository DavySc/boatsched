"""Tests for instructor password authentication."""
import pytest
from fastapi.testclient import TestClient

from boatsched import Config
from boatsched import db as _db
from boatsched.server import create_app

CFG = Config(
    boats=1, levels=["k"], restricted=["Sat"], diverse=["Fri"],
    stage_seconds=15, seconds=15,
)


def _make_instructor(db_conn, name, level="k", password=None):
    _db.add_or_update_members(db_conn, [(name, level)])
    row = db_conn.execute("SELECT id FROM member WHERE name=?", (name,)).fetchone()
    db_conn.execute("INSERT OR IGNORE INTO instructor (member_id) VALUES (?)", (row["id"],))
    db_conn.commit()
    if password is not None:
        _db.set_instructor_password(db_conn, row["id"], password)
    return row["id"]


def _login(client, name, password):
    return client.post("/api/auth/login", json={"name": name, "password": password})


@pytest.fixture
def open_client(tmp_path):
    """App with DB, one instructor, but no passwords set (open/bootstrap mode)."""
    db_conn = _db.init_db(str(tmp_path / "open.db"))
    _make_instructor(db_conn, "OpenUser")
    app = create_app(CFG, db_conn)
    with TestClient(app, raise_server_exceptions=True) as client:
        yield client, db_conn


@pytest.fixture
def empty_client(tmp_path):
    """App with DB but no instructors at all (still open/bootstrap mode)."""
    db_conn = _db.init_db(str(tmp_path / "empty.db"))
    app = create_app(CFG, db_conn)
    with TestClient(app, raise_server_exceptions=True) as client:
        yield client, db_conn


@pytest.fixture
def auth_client(tmp_path):
    """App with DB where Alice has a password (auth mode)."""
    db_conn = _db.init_db(str(tmp_path / "auth.db"))
    _make_instructor(db_conn, "Alice", password="secret123")
    app = create_app(CFG, db_conn)
    with TestClient(app, raise_server_exceptions=True) as client:
        yield client, db_conn


# ---------------------------------------------------------------------------
# Open mode — no passwords set
# ---------------------------------------------------------------------------

class TestOpenMode:
    def test_root_accessible_without_login(self, open_client):
        client, _ = open_client
        r = client.get("/", follow_redirects=False)
        assert r.status_code == 200

    def test_api_accessible_without_login(self, open_client):
        client, _ = open_client
        assert client.get("/seasons").status_code == 200

    def test_config_accessible_in_open_mode(self, open_client):
        client, _ = open_client
        assert client.get("/config").status_code == 200

    def test_api_login_in_open_mode_accepts_any_credentials(self, open_client):
        client, _ = open_client
        r = client.post("/api/auth/login", json={"name": "anyone", "password": ""})
        assert r.status_code == 200
        assert r.json()["ok"] is True
        assert "bsession" in r.cookies

    def test_set_password_in_open_mode(self, open_client):
        client, db_conn = open_client
        _make_instructor(db_conn, "Bob")
        r = client.post("/set-password", json={"name": "Bob", "password": "pw123"})
        assert r.status_code == 200
        assert r.json()["ok"] is True
        assert _db.verify_instructor_login(db_conn, "Bob", "pw123") is not None

    def test_set_password_for_non_instructor_returns_404(self, open_client):
        client, db_conn = open_client
        _db.add_or_update_members(db_conn, [("Carol", "k")])  # member but NOT instructor
        r = client.post("/set-password", json={"name": "Carol", "password": "pw"})
        assert r.status_code == 404


# ---------------------------------------------------------------------------
# Auth mode — at least one password set
# ---------------------------------------------------------------------------

class TestAuthMode:
    def test_root_always_returns_viewer_html(self, auth_client):
        """/ always returns viewer.html; JS shows login overlay when unauthenticated."""
        client, _ = auth_client
        r = client.get("/", follow_redirects=False)
        assert r.status_code == 200

    def test_api_returns_401_when_not_logged_in(self, auth_client):
        client, _ = auth_client
        assert client.get("/seasons").status_code == 401

    def test_config_returns_401_when_not_logged_in(self, auth_client):
        client, _ = auth_client
        assert client.get("/config").status_code == 401

    def test_valid_json_login_sets_session_cookie(self, auth_client):
        client, _ = auth_client
        r = _login(client, "Alice", "secret123")
        assert r.status_code == 200
        assert r.json()["ok"] is True
        assert "bsession" in r.cookies

    def test_invalid_password_returns_401(self, auth_client):
        client, _ = auth_client
        r = _login(client, "Alice", "wrong")
        assert r.status_code == 401
        assert "bsession" not in r.cookies

    def test_wrong_name_returns_401(self, auth_client):
        client, _ = auth_client
        r = _login(client, "Nobody", "pw")
        assert r.status_code == 401

    def test_authenticated_root_returns_viewer(self, auth_client):
        client, _ = auth_client
        _login(client, "Alice", "secret123")
        r = client.get("/", follow_redirects=False)
        assert r.status_code == 200

    def test_authenticated_api_returns_data(self, auth_client):
        client, _ = auth_client
        _login(client, "Alice", "secret123")
        assert client.get("/seasons").status_code == 200

    def test_logout_clears_session(self, auth_client):
        client, _ = auth_client
        _login(client, "Alice", "secret123")
        assert client.get("/seasons").status_code == 200
        client.post("/api/auth/logout")
        assert client.get("/seasons").status_code == 401

    def test_set_password_requires_auth_in_auth_mode(self, auth_client):
        client, db_conn = auth_client
        _make_instructor(db_conn, "Dave")
        r = client.post("/set-password", json={"name": "Dave", "password": "new"})
        assert r.status_code == 401

    def test_set_password_succeeds_when_authenticated(self, auth_client):
        client, db_conn = auth_client
        _make_instructor(db_conn, "Eve")
        _login(client, "Alice", "secret123")
        r = client.post("/set-password", json={"name": "Eve", "password": "evepw"})
        assert r.status_code == 200
        assert r.json()["ok"] is True


# ---------------------------------------------------------------------------
# /me endpoint
# ---------------------------------------------------------------------------

class TestMeEndpoint:
    def test_me_returns_zero_in_open_mode(self, open_client):
        client, _ = open_client
        r = client.get("/me")
        assert r.status_code == 200
        assert r.json()["member_id"] == 0

    def test_me_returns_401_when_not_logged_in(self, auth_client):
        client, _ = auth_client
        assert client.get("/me").status_code == 401

    def test_me_returns_name_after_login(self, auth_client):
        client, _ = auth_client
        _login(client, "Alice", "secret123")
        r = client.get("/me")
        assert r.status_code == 200
        assert r.json()["name"] == "Alice"


# ---------------------------------------------------------------------------
# POST /instructor — create new instructor
# ---------------------------------------------------------------------------

class TestCreateInstructor:
    def test_create_instructor_in_open_mode(self, open_client):
        client, db_conn = open_client
        r = client.post("/instructor", json={"name": "NewGuy", "password": "pw1"})
        assert r.status_code == 200
        assert r.json()["ok"] is True
        assert _db.verify_instructor_login(db_conn, "NewGuy", "pw1") is not None

    def test_create_instructor_requires_auth_in_auth_mode(self, auth_client):
        client, _ = auth_client
        r = client.post("/instructor", json={"name": "NewGuy", "password": "pw1"})
        assert r.status_code == 401

    def test_create_instructor_authenticated(self, auth_client):
        client, db_conn = auth_client
        _login(client, "Alice", "secret123")
        r = client.post("/instructor", json={"name": "Bob", "password": "bobpw"})
        assert r.status_code == 200
        assert r.json()["ok"] is True
        assert _db.verify_instructor_login(db_conn, "Bob", "bobpw") is not None

    def test_create_instructor_promotes_existing_member(self, auth_client):
        client, db_conn = auth_client
        _login(client, "Alice", "secret123")
        _db.add_or_update_members(db_conn, [("Carol", "k")])
        r = client.post("/instructor", json={"name": "Carol", "password": "carolpw"})
        assert r.status_code == 200
        assert r.json()["ok"] is True

    def test_create_duplicate_instructor_returns_409(self, auth_client):
        client, _ = auth_client
        _login(client, "Alice", "secret123")
        r = client.post("/instructor", json={"name": "Alice", "password": "newpw"})
        assert r.status_code == 409

    def test_create_instructor_empty_fields_returns_400(self, auth_client):
        client, _ = auth_client
        _login(client, "Alice", "secret123")
        r = client.post("/instructor", json={"name": "", "password": ""})
        assert r.status_code == 400

    def test_new_instructor_appears_in_list(self, auth_client):
        client, _ = auth_client
        _login(client, "Alice", "secret123")
        client.post("/instructor", json={"name": "Dave", "password": "davepw"})
        instructors = client.get("/instructors").json()
        names = [i["name"] for i in instructors]
        assert "Dave" in names


# ---------------------------------------------------------------------------
# /instructors endpoint
# ---------------------------------------------------------------------------

class TestInstructorsEndpoint:
    def test_lists_instructors_with_password_status(self, auth_client):
        client, db_conn = auth_client
        _make_instructor(db_conn, "Frank")
        _login(client, "Alice", "secret123")
        r = client.get("/instructors")
        assert r.status_code == 200
        data = {i["name"]: i for i in r.json()}
        assert data["Alice"]["has_password"] == 1
        assert data["Frank"]["has_password"] == 0


# ---------------------------------------------------------------------------
# First-time setup (empty DB — no instructors)
# ---------------------------------------------------------------------------

class TestBootstrapMode:
    def test_root_accessible_with_no_instructors(self, empty_client):
        """No instructors means open/bootstrap mode — / should serve the app."""
        client, _ = empty_client
        r = client.get("/", follow_redirects=False)
        assert r.status_code == 200

    def test_api_accessible_in_bootstrap_mode(self, empty_client):
        client, _ = empty_client
        assert client.get("/seasons").status_code == 200

    def test_can_create_first_instructor_in_bootstrap_mode(self, empty_client):
        client, db_conn = empty_client
        r = client.post("/instructor", json={"name": "admin", "password": "adminpw"})
        assert r.status_code == 200
        assert r.json()["ok"] is True
        assert _db.verify_instructor_login(db_conn, "admin", "adminpw") is not None

    def test_after_first_instructor_api_login_works(self, empty_client):
        client, _ = empty_client
        client.post("/instructor", json={"name": "admin", "password": "adminpw"})
        r = _login(client, "admin", "adminpw")
        assert r.status_code == 200
        assert "bsession" in r.cookies


# ---------------------------------------------------------------------------
# JSON auth endpoints (/api/auth/*)
# ---------------------------------------------------------------------------

class TestApiAuth:
    def test_api_me_returns_401_when_not_authenticated(self, auth_client):
        client, _ = auth_client
        r = client.get("/api/auth/me")
        assert r.status_code == 401

    def test_api_me_returns_user_after_login(self, auth_client):
        client, _ = auth_client
        _login(client, "Alice", "secret123")
        r = client.get("/api/auth/me")
        assert r.status_code == 200
        assert r.json()["name"] == "Alice"

    def test_api_login_returns_ok_and_cookie(self, auth_client):
        client, _ = auth_client
        r = client.post("/api/auth/login", json={"name": "Alice", "password": "secret123"})
        assert r.status_code == 200
        assert r.json()["ok"] is True
        assert "bsession" in r.cookies

    def test_api_logout_clears_session(self, auth_client):
        client, _ = auth_client
        _login(client, "Alice", "secret123")
        assert client.get("/api/auth/me").status_code == 200
        client.post("/api/auth/logout")
        assert client.get("/api/auth/me").status_code == 401


# ---------------------------------------------------------------------------
# Password hashing
# ---------------------------------------------------------------------------

class TestPasswordHashing:
    def test_hash_is_not_plaintext(self, open_client):
        _, db_conn = open_client
        mid = _make_instructor(db_conn, "Hank")
        _db.set_instructor_password(db_conn, mid, "plaintextpw")
        row = db_conn.execute(
            "SELECT password_hash, password_salt FROM instructor WHERE member_id=?", (mid,)
        ).fetchone()
        ph = bytes(row["password_hash"])
        ps = bytes(row["password_salt"])
        assert b"plaintextpw" not in ph
        assert len(ph) == 64
        assert len(ps) == 32

    def test_wrong_password_returns_none(self, open_client):
        _, db_conn = open_client
        mid = _make_instructor(db_conn, "Iris")
        _db.set_instructor_password(db_conn, mid, "correct")
        assert _db.verify_instructor_login(db_conn, "Iris", "wrong") is None

    def test_correct_password_returns_member_id(self, open_client):
        _, db_conn = open_client
        mid = _make_instructor(db_conn, "Jack")
        _db.set_instructor_password(db_conn, mid, "mypassword")
        assert _db.verify_instructor_login(db_conn, "Jack", "mypassword") == mid

    def test_two_instructors_same_password_get_different_hashes(self, open_client):
        _, db_conn = open_client
        mid1 = _make_instructor(db_conn, "Kate")
        mid2 = _make_instructor(db_conn, "Leo")
        _db.set_instructor_password(db_conn, mid1, "shared")
        _db.set_instructor_password(db_conn, mid2, "shared")
        h1 = db_conn.execute("SELECT password_hash FROM instructor WHERE member_id=?",
                             (mid1,)).fetchone()["password_hash"]
        h2 = db_conn.execute("SELECT password_hash FROM instructor WHERE member_id=?",
                             (mid2,)).fetchone()["password_hash"]
        assert h1 != h2
