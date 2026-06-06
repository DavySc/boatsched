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


@pytest.fixture
def open_client(tmp_path):
    """App with DB, one instructor, but no passwords set (open/bootstrap mode)."""
    db_conn = _db.init_db(str(tmp_path / "open.db"))
    _make_instructor(db_conn, "OpenUser")  # at least one instructor so / shows viewer
    app = create_app(CFG, db_conn)
    with TestClient(app, raise_server_exceptions=True) as client:
        yield client, db_conn


@pytest.fixture
def empty_client(tmp_path):
    """App with DB but no instructors at all (triggers /setup redirect)."""
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

    def test_config_always_public(self, open_client):
        client, _ = open_client
        assert client.get("/config").status_code == 200

    def test_login_page_accessible_in_open_mode(self, open_client):
        client, _ = open_client
        r = client.get("/login")
        assert r.status_code == 200
        assert b"Log in" in r.content

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

    def test_login_in_open_mode_redirects_to_root(self, open_client):
        client, _ = open_client
        r = client.post("/login", data={"name": "x", "password": "y"},
                        follow_redirects=False)
        assert r.status_code == 303
        assert r.headers["location"] == "/"


# ---------------------------------------------------------------------------
# Auth mode — at least one password set
# ---------------------------------------------------------------------------

class TestAuthMode:
    def test_root_redirects_when_not_logged_in(self, auth_client):
        client, _ = auth_client
        r = client.get("/", follow_redirects=False)
        assert r.status_code == 303
        assert r.headers["location"] == "/login"

    def test_api_returns_401_when_not_logged_in(self, auth_client):
        client, _ = auth_client
        assert client.get("/seasons").status_code == 401

    def test_config_still_public_in_auth_mode(self, auth_client):
        client, _ = auth_client
        assert client.get("/config").status_code == 200

    def test_login_page_accessible_in_auth_mode(self, auth_client):
        client, _ = auth_client
        r = client.get("/login")
        assert r.status_code == 200

    def test_valid_login_sets_session_cookie(self, auth_client):
        client, _ = auth_client
        r = client.post("/login", data={"name": "Alice", "password": "secret123"},
                        follow_redirects=False)
        assert r.status_code == 303
        assert r.headers["location"] == "/"
        assert "bsession" in r.cookies

    def test_invalid_password_redirects_with_error(self, auth_client):
        client, _ = auth_client
        r = client.post("/login", data={"name": "Alice", "password": "wrong"},
                        follow_redirects=False)
        assert r.status_code == 303
        assert "error=1" in r.headers["location"]
        assert "bsession" not in r.cookies

    def test_wrong_name_redirects_with_error(self, auth_client):
        client, _ = auth_client
        r = client.post("/login", data={"name": "Nobody", "password": "pw"},
                        follow_redirects=False)
        assert r.status_code == 303
        assert "error=1" in r.headers["location"]

    def test_authenticated_root_returns_viewer(self, auth_client):
        client, _ = auth_client
        client.post("/login", data={"name": "Alice", "password": "secret123"})
        r = client.get("/", follow_redirects=False)
        assert r.status_code == 200

    def test_authenticated_api_returns_data(self, auth_client):
        client, _ = auth_client
        client.post("/login", data={"name": "Alice", "password": "secret123"})
        assert client.get("/seasons").status_code == 200

    def test_logout_clears_session(self, auth_client):
        client, _ = auth_client
        client.post("/login", data={"name": "Alice", "password": "secret123"})
        assert client.get("/", follow_redirects=False).status_code == 200
        client.post("/logout")
        r = client.get("/", follow_redirects=False)
        assert r.status_code == 303

    def test_set_password_requires_auth_in_auth_mode(self, auth_client):
        client, db_conn = auth_client
        _make_instructor(db_conn, "Dave")
        r = client.post("/set-password", json={"name": "Dave", "password": "new"})
        assert r.status_code == 401

    def test_set_password_succeeds_when_authenticated(self, auth_client):
        client, db_conn = auth_client
        _make_instructor(db_conn, "Eve")
        client.post("/login", data={"name": "Alice", "password": "secret123"})
        r = client.post("/set-password", json={"name": "Eve", "password": "evepw"})
        assert r.status_code == 200
        assert r.json()["ok"] is True

    def test_login_error_page_shows_error_message(self, auth_client):
        client, _ = auth_client
        r = client.get("/login?error=1")
        assert r.status_code == 200
        assert b"Incorrect" in r.content


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
        client.post("/login", data={"name": "Alice", "password": "secret123"})
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
        client.post("/login", data={"name": "Alice", "password": "secret123"})
        r = client.post("/instructor", json={"name": "Bob", "password": "bobpw"})
        assert r.status_code == 200
        assert r.json()["ok"] is True
        assert _db.verify_instructor_login(db_conn, "Bob", "bobpw") is not None

    def test_create_instructor_promotes_existing_member(self, auth_client):
        client, db_conn = auth_client
        client.post("/login", data={"name": "Alice", "password": "secret123"})
        # Add a regular member first
        _db.add_or_update_members(db_conn, [("Carol", "k")])
        r = client.post("/instructor", json={"name": "Carol", "password": "carolpw"})
        assert r.status_code == 200
        assert r.json()["ok"] is True

    def test_create_duplicate_instructor_returns_409(self, auth_client):
        client, _ = auth_client
        client.post("/login", data={"name": "Alice", "password": "secret123"})
        r = client.post("/instructor", json={"name": "Alice", "password": "newpw"})
        assert r.status_code == 409

    def test_create_instructor_empty_fields_returns_400(self, auth_client):
        client, _ = auth_client
        client.post("/login", data={"name": "Alice", "password": "secret123"})
        r = client.post("/instructor", json={"name": "", "password": ""})
        assert r.status_code == 400

    def test_new_instructor_appears_in_list(self, auth_client):
        client, _ = auth_client
        client.post("/login", data={"name": "Alice", "password": "secret123"})
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
        # Alice already has a password (from fixture); add Frank without one
        _make_instructor(db_conn, "Frank")
        client.post("/login", data={"name": "Alice", "password": "secret123"})
        r = client.get("/instructors")
        assert r.status_code == 200
        data = {i["name"]: i for i in r.json()}
        assert data["Alice"]["has_password"] == 1
        assert data["Frank"]["has_password"] == 0


# ---------------------------------------------------------------------------
# First-time setup endpoint
# ---------------------------------------------------------------------------

class TestSetupEndpoint:
    def test_root_redirects_to_setup_when_no_instructors(self, empty_client):
        client, _ = empty_client
        r = client.get("/", follow_redirects=False)
        assert r.status_code == 303
        assert r.headers["location"] == "/setup"

    def test_setup_page_accessible(self, empty_client):
        client, _ = empty_client
        r = client.get("/setup")
        assert r.status_code == 200
        assert b"Create account" in r.content

    def test_setup_creates_instructor_sets_cookie_and_redirects(self, empty_client):
        client, db_conn = empty_client
        r = client.post("/setup", data={"name": "admin", "password": "adminpw"},
                        follow_redirects=False)
        assert r.status_code == 303
        assert r.headers["location"] == "/"
        assert "bsession" in r.cookies
        assert _db.verify_instructor_login(db_conn, "admin", "adminpw") is not None

    def test_setup_activates_auth_mode(self, empty_client):
        client, _ = empty_client
        client.post("/setup", data={"name": "admin", "password": "pw"})
        client.post("/logout")
        r = client.get("/", follow_redirects=False)
        assert r.status_code == 303
        assert r.headers["location"] == "/login"

    def test_setup_inaccessible_once_account_exists(self, empty_client):
        client, _ = empty_client
        client.post("/setup", data={"name": "admin", "password": "pw"})
        r = client.get("/setup", follow_redirects=False)
        assert r.status_code == 303
        assert r.headers["location"] == "/login"

    def test_setup_empty_fields_redirects_with_error(self, empty_client):
        client, _ = empty_client
        r = client.post("/setup", data={"name": "", "password": ""},
                        follow_redirects=False)
        assert r.status_code == 303
        assert "error=1" in r.headers["location"]

    def test_setup_error_page_shows_message(self, empty_client):
        client, _ = empty_client
        r = client.get("/setup?error=1")
        assert r.status_code == 200
        assert b"required" in r.content


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
        assert b"plaintextpw" not in ph  # hash is opaque binary
        assert len(ph) == 64             # scrypt default dklen
        assert len(ps) == 32             # 32-byte random salt

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
        assert h1 != h2  # different salts → different hashes
