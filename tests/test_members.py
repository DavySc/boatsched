"""Tests for member management API endpoints and admin role enforcement."""
import json

import pytest
from fastapi.testclient import TestClient

from boatsched import Config
from boatsched import db as _db
from boatsched.server import create_app

CFG = Config(
    boats=1, levels=["k"], restricted=["Sat"], diverse=["Fri"],
    stage_seconds=5, seconds=5,
)


def _make_member(db_conn, name):
    db_conn.execute("INSERT INTO member (name) VALUES (?)", (name,))
    db_conn.commit()
    return db_conn.execute("SELECT id FROM member WHERE name=?", (name,)).fetchone()["id"]


def _make_instructor(db_conn, name, password, is_admin=False):
    mid = _make_member(db_conn, name)
    db_conn.execute("INSERT INTO instructor (member_id, is_admin) VALUES (?, ?)",
                    (mid, 1 if is_admin else 0))
    db_conn.commit()
    _db.set_instructor_password(db_conn, mid, password)
    return mid


def _login(client, name, password):
    r = client.post("/api/auth/login", json={"name": name, "password": password})
    assert r.status_code == 200
    return r


@pytest.fixture
def admin_client(tmp_path):
    db_conn = _db.init_db(str(tmp_path / "members.db"))
    _db.add_admin_column(db_conn)  # migration normally run by create_app
    _make_instructor(db_conn, "Admin", "adminpw", is_admin=True)
    _make_instructor(db_conn, "NonAdmin", "nonadminpw", is_admin=False)
    _make_member(db_conn, "Alice")
    _make_member(db_conn, "Bob")
    app = create_app(CFG, db_conn)
    with TestClient(app, raise_server_exceptions=True) as client:
        _login(client, "Admin", "adminpw")
        yield client, db_conn


@pytest.fixture
def nonadmin_client(tmp_path):
    db_conn = _db.init_db(str(tmp_path / "nonadmin.db"))
    _db.add_admin_column(db_conn)  # migration normally run by create_app
    _make_instructor(db_conn, "Admin", "adminpw", is_admin=True)
    _make_instructor(db_conn, "NonAdmin", "nonadminpw", is_admin=False)
    app = create_app(CFG, db_conn)
    with TestClient(app, raise_server_exceptions=True) as client:
        _login(client, "NonAdmin", "nonadminpw")
        yield client, db_conn


# ---------------------------------------------------------------------------
# GET /api/members
# ---------------------------------------------------------------------------

class TestListMembers:
    def test_returns_list_of_active_members(self, admin_client):
        client, _ = admin_client
        r = client.get("/api/members")
        assert r.status_code == 200
        names = [m["name"] for m in r.json()]
        assert "Alice" in names
        assert "Bob" in names

    def test_each_member_has_id_and_name(self, admin_client):
        client, _ = admin_client
        r = client.get("/api/members")
        for m in r.json():
            assert "id" in m and "name" in m

    def test_deleted_member_not_in_list(self, admin_client):
        client, db_conn = admin_client
        mid = _make_member(db_conn, "ToBeDeleted")
        _db.delete_member(db_conn, mid)
        names = [m["name"] for m in client.get("/api/members").json()]
        assert "ToBeDeleted" not in names

    def test_no_db_returns_503(self, tmp_path):
        app = create_app(CFG)
        with TestClient(app) as client:
            r = client.get("/api/members")
            assert r.status_code == 503


# ---------------------------------------------------------------------------
# DELETE /api/members/{id}
# ---------------------------------------------------------------------------

class TestDeleteMember:
    def test_delete_sets_active_zero(self, admin_client):
        client, db_conn = admin_client
        mid = _make_member(db_conn, "DeleteMe")
        r = client.delete(f"/api/members/{mid}")
        assert r.status_code == 200
        assert r.json()["ok"] is True
        row = db_conn.execute("SELECT active FROM member WHERE id=?", (mid,)).fetchone()
        assert row["active"] == 0

    def test_deleted_member_disappears_from_list(self, admin_client):
        client, db_conn = admin_client
        mid = _make_member(db_conn, "SoftDeleteMe")
        client.delete(f"/api/members/{mid}")
        names = [m["name"] for m in client.get("/api/members").json()]
        assert "SoftDeleteMe" not in names

    def test_cannot_delete_instructor(self, admin_client):
        client, db_conn = admin_client
        r = client.delete(f"/api/members/{_get_member_id(db_conn, 'Admin')}")
        assert r.status_code == 409
        assert "instructeur" in r.json()["error"].lower()

    def test_non_admin_cannot_delete(self, nonadmin_client):
        client, db_conn = nonadmin_client
        mid = _make_member(db_conn, "CannotDelete")
        r = client.delete(f"/api/members/{mid}")
        assert r.status_code == 403

    def test_no_db_returns_503(self, tmp_path):
        app = create_app(CFG)
        with TestClient(app) as client:
            r = client.delete("/api/members/1")
            assert r.status_code == 503


def _get_member_id(db_conn, name):
    return db_conn.execute("SELECT id FROM member WHERE name=?", (name,)).fetchone()["id"]


# ---------------------------------------------------------------------------
# PUT /api/members/{id}
# ---------------------------------------------------------------------------

class TestRenameMember:
    def test_rename_updates_name_in_db(self, admin_client):
        client, db_conn = admin_client
        mid = _make_member(db_conn, "OldName")
        r = client.put(f"/api/members/{mid}", json={"name": "NewName"})
        assert r.status_code == 200
        assert r.json()["ok"] is True
        row = db_conn.execute("SELECT name FROM member WHERE id=?", (mid,)).fetchone()
        assert row["name"] == "NewName"

    def test_renamed_member_appears_in_list(self, admin_client):
        client, db_conn = admin_client
        mid = _make_member(db_conn, "BeforeRename")
        client.put(f"/api/members/{mid}", json={"name": "AfterRename"})
        names = [m["name"] for m in client.get("/api/members").json()]
        assert "AfterRename" in names
        assert "BeforeRename" not in names

    def test_duplicate_name_returns_409(self, admin_client):
        client, db_conn = admin_client
        _make_member(db_conn, "Existing")
        mid = _make_member(db_conn, "RenameToExisting")
        r = client.put(f"/api/members/{mid}", json={"name": "Existing"})
        assert r.status_code == 409

    def test_empty_name_returns_400(self, admin_client):
        client, db_conn = admin_client
        mid = _make_member(db_conn, "HasName")
        r = client.put(f"/api/members/{mid}", json={"name": "   "})
        assert r.status_code == 400

    def test_non_admin_cannot_rename(self, nonadmin_client):
        client, db_conn = nonadmin_client
        mid = _make_member(db_conn, "ProtectedName")
        r = client.put(f"/api/members/{mid}", json={"name": "Hacked"})
        assert r.status_code == 403

    def test_no_db_returns_503(self, tmp_path):
        app = create_app(CFG)
        with TestClient(app) as client:
            r = client.put("/api/members/1", json={"name": "X"})
            assert r.status_code == 503


# ---------------------------------------------------------------------------
# POST /api/members/import
# ---------------------------------------------------------------------------

class TestImportMembers:
    def _csv(self, names):
        header = "name\n"
        return (header + "\n".join(names)).encode()

    def test_import_creates_new_members(self, admin_client):
        client, db_conn = admin_client
        r = client.post(
            "/api/members/import",
            content=self._csv(["Imported1", "Imported2"]),
            headers={"Content-Type": "text/csv"},
        )
        assert r.status_code == 200
        data = r.json()
        assert data["created"] == 2
        names = [m["name"] for m in client.get("/api/members").json()]
        assert "Imported1" in names
        assert "Imported2" in names

    def test_import_existing_member_counts_as_existing(self, admin_client):
        client, _ = admin_client
        client.post("/api/members/import", content=self._csv(["ExistCheck"]),
                    headers={"Content-Type": "text/csv"})
        r = client.post("/api/members/import", content=self._csv(["ExistCheck"]),
                        headers={"Content-Type": "text/csv"})
        assert r.json()["created"] == 0
        assert r.json()["existing"] == 1

    def test_total_reflects_whole_member_table(self, admin_client):
        client, db_conn = admin_client
        before = db_conn.execute("SELECT COUNT(*) FROM member WHERE active=1").fetchone()[0]
        r = client.post("/api/members/import", content=self._csv(["TotalCheck"]),
                        headers={"Content-Type": "text/csv"})
        assert r.json()["total"] >= before + 1

    def test_empty_upload_returns_400(self, admin_client):
        client, _ = admin_client
        r = client.post("/api/members/import", content=b"name\n",
                        headers={"Content-Type": "text/csv"})
        assert r.status_code == 400

    def test_non_admin_cannot_import(self, nonadmin_client):
        client, _ = nonadmin_client
        r = client.post("/api/members/import",
                        content=b"name\nShouldNotAppear",
                        headers={"Content-Type": "text/csv"})
        assert r.status_code == 403

    def test_no_db_returns_503(self, tmp_path):
        app = create_app(CFG)
        with TestClient(app) as client:
            r = client.post("/api/members/import", content=b"name\nX",
                            headers={"Content-Type": "text/csv"})
            assert r.status_code == 503


# ---------------------------------------------------------------------------
# GET /api/members/progression
# ---------------------------------------------------------------------------

class TestMembersProgression:
    def test_returns_seasons_and_members_keys(self, admin_client):
        client, _ = admin_client
        r = client.get("/api/members/progression")
        assert r.status_code == 200
        data = r.json()
        assert "seasons" in data and "members" in data

    def test_excludes_instructors(self, admin_client):
        client, _ = admin_client
        data = client.get("/api/members/progression").json()
        names = [m["name"] for m in data["members"]]
        assert "Admin" not in names
        assert "NonAdmin" not in names

    def test_includes_non_instructor_members(self, admin_client):
        client, _ = admin_client
        data = client.get("/api/members/progression").json()
        names = [m["name"] for m in data["members"]]
        assert "Alice" in names
        assert "Bob" in names

    def test_levels_populated_after_enrollment(self, admin_client):
        client, db_conn = admin_client
        mid = _make_member(db_conn, "EnrolledMember")
        sid = _db.create_season(db_conn, 2100)
        _db.add_enrollment(db_conn, mid, sid, "k")
        data = client.get("/api/members/progression").json()
        seasons = {s["id"]: s["year"] for s in data["seasons"]}
        assert sid in seasons
        member = next(m for m in data["members"] if m["name"] == "EnrolledMember")
        assert member["levels"][str(sid)] == "k"

    def test_season_ids_are_strings_in_levels(self, admin_client):
        """Levels dict keys must be strings (JSON only has string keys)."""
        client, db_conn = admin_client
        mid = _make_member(db_conn, "KeyCheckMember")
        sid = _db.create_season(db_conn, 2101)
        _db.add_enrollment(db_conn, mid, sid, "b")
        data = client.get("/api/members/progression").json()
        member = next(m for m in data["members"] if m["name"] == "KeyCheckMember")
        assert str(sid) in member["levels"]

    def test_no_db_returns_503(self, tmp_path):
        app = create_app(CFG)
        with TestClient(app) as client:
            r = client.get("/api/members/progression")
            assert r.status_code == 503


# ---------------------------------------------------------------------------
# POST /api/instructors/{member_id}/set-admin
# ---------------------------------------------------------------------------

class TestSetAdmin:
    def test_promote_non_admin_instructor(self, admin_client):
        client, db_conn = admin_client
        mid = _get_member_id(db_conn, "NonAdmin")
        r = client.post(f"/api/instructors/{mid}/set-admin", json={"is_admin": True})
        assert r.status_code == 200
        assert r.json()["ok"] is True
        row = db_conn.execute("SELECT is_admin FROM instructor WHERE member_id=?",
                              (mid,)).fetchone()
        assert row["is_admin"] == 1

    def test_demote_other_admin_instructor(self, tmp_path):
        db_conn = _db.init_db(str(tmp_path / "demote.db"))
        _db.add_admin_column(db_conn)
        a1 = _make_instructor(db_conn, "A1", "pw1", is_admin=True)
        a2 = _make_instructor(db_conn, "A2", "pw2", is_admin=True)
        app = create_app(CFG, db_conn)
        with TestClient(app, raise_server_exceptions=True) as client:
            _login(client, "A1", "pw1")
            r = client.post(f"/api/instructors/{a2}/set-admin", json={"is_admin": False})
            assert r.status_code == 200
            row = db_conn.execute("SELECT is_admin FROM instructor WHERE member_id=?",
                                  (a2,)).fetchone()
            assert row["is_admin"] == 0

    def test_cannot_remove_own_admin(self, admin_client):
        client, db_conn = admin_client
        mid = _get_member_id(db_conn, "Admin")
        r = client.post(f"/api/instructors/{mid}/set-admin", json={"is_admin": False})
        assert r.status_code == 400
        assert "own" in r.json()["error"].lower() or "self" in r.json()["error"].lower() \
               or "eigen" in r.json()["error"].lower()

    def test_not_instructor_returns_404(self, admin_client):
        client, db_conn = admin_client
        mid = _make_member(db_conn, "JustAMember")
        r = client.post(f"/api/instructors/{mid}/set-admin", json={"is_admin": True})
        assert r.status_code == 404

    def test_non_admin_cannot_set_admin(self, nonadmin_client):
        client, db_conn = nonadmin_client
        mid = _get_member_id(db_conn, "Admin")
        r = client.post(f"/api/instructors/{mid}/set-admin", json={"is_admin": False})
        assert r.status_code == 403

    def test_no_db_returns_503(self, tmp_path):
        app = create_app(CFG)
        with TestClient(app) as client:
            r = client.post("/api/instructors/1/set-admin", json={"is_admin": True})
            assert r.status_code == 503
