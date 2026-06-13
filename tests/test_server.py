"""Tests for boatsched.server HTTP endpoints and helpers."""
import json
import time

import pytest
from fastapi.testclient import TestClient

from boatsched import Config
from boatsched import db as _db
from boatsched.server import _extract_multipart, create_app


ROSTER_CSV = b"name,level\nAlice,k\nBob,k\nCarol,b\nDave,b\n"


# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------

@pytest.fixture(scope="module")
def srv():
    cfg = Config(
        boats=2, levels=["k", "b"],
        restricted=["Sat"], diverse=["Fri"],
        stage_seconds=15, seconds=15,
    )
    app = create_app(cfg)
    with TestClient(app, raise_server_exceptions=True) as client:
        yield client


@pytest.fixture(scope="module")
def srv_db(tmp_path_factory):
    db_path = str(tmp_path_factory.mktemp("db") / "boatsched.db")
    db_conn = _db.init_db(db_path)
    # Carol and Dave are returning (non-k) members in ROSTER_CSV; they must
    # exist in the member table before any solve uploads the CSV.
    db_conn.executemany("INSERT INTO member (name) VALUES (?)",
                        [("Carol",), ("Dave",)])
    db_conn.commit()
    cfg = Config(boats=2, levels=["k", "b"],
                 restricted=["Sat"], diverse=["Fri"],
                 stage_seconds=15, seconds=15)
    app = create_app(cfg, db_conn)
    with TestClient(app, raise_server_exceptions=True) as client:
        yield client, db_conn


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def _get(client, path):
    r = client.get(path)
    return r.status_code, r.content


def _post(client, path, body, content_type="text/csv"):
    r = client.post(path, content=body, headers={"Content-Type": content_type})
    return r.status_code, json.loads(r.content)


def _get_json(client, path):
    status, body = _get(client, path)
    return status, json.loads(body)


def _poll_done(client, jid, timeout=60):
    deadline = time.time() + timeout
    data = {}
    while time.time() < deadline:
        _, data = _get_json(client, f"/status?id={jid}")
        if data["status"] != "running":
            break
        time.sleep(0.2)
    return data


def _poll_db(db_conn, sql, args, timeout=10):
    """Poll until the SQL query returns a row, then return it."""
    deadline = time.time() + timeout
    while time.time() < deadline:
        row = db_conn.execute(sql, args).fetchone()
        if row:
            return row
        time.sleep(0.1)
    return None


# ---------------------------------------------------------------------------
# _extract_multipart
# ---------------------------------------------------------------------------

class TestExtractMultipart:
    def test_extracts_file_part(self):
        ctype = "multipart/form-data; boundary=Boundary"
        body = (
            b"--Boundary\r\n"
            b'Content-Disposition: form-data; name="file"; filename="roster.csv"\r\n'
            b"\r\n"
            b"name,level\nAlice,k\n"
            b"\r\n"
            b"--Boundary--\r\n"
        )
        result = _extract_multipart(body, ctype)
        assert b"Alice" in result
        assert b"name,level" in result

    def test_falls_back_to_raw_when_no_filename_part(self):
        ctype = "multipart/form-data; boundary=X"
        raw = b"not a real multipart body"
        assert _extract_multipart(raw, ctype) == raw

    def test_quoted_boundary(self):
        ctype = 'multipart/form-data; boundary="MyBound"'
        body = (
            b"--MyBound\r\n"
            b'Content-Disposition: form-data; name="f"; filename="r.csv"\r\n'
            b"\r\n"
            b"data\r\n"
            b"--MyBound--\r\n"
        )
        result = _extract_multipart(body, ctype)
        assert b"data" in result


# ---------------------------------------------------------------------------
# HTTP endpoints
# ---------------------------------------------------------------------------

class TestGetRoot:
    def test_returns_200(self, srv):
        status, body = _get(srv, "/")
        assert status == 200

    def test_returns_html(self, srv):
        status, body = _get(srv, "/")
        assert b"<" in body  # minimal HTML check

    def test_index_html_alias(self, srv):
        status, _ = _get(srv, "/index.html")
        assert status == 200


class TestGetConfig:
    def test_returns_200(self, srv):
        status, _ = _get(srv, "/config")
        assert status == 200

    def test_contains_expected_fields(self, srv):
        _, body = _get(srv, "/config")
        data = json.loads(body)
        assert data["boats"] == 2
        assert "k" in data["levels"]
        assert "b" in data["levels"]
        assert "Sat" in data["restricted"]
        assert "Fri" in data["diverse"]


class TestGetUnknown:
    def test_returns_404(self, srv):
        status, body = _get(srv, "/no/such/path")
        assert status == 404


class TestPostSolve:
    def test_valid_csv_returns_job_id(self, srv):
        status, data = _post(srv, "/solve", ROSTER_CSV)
        assert status == 200
        assert "id" in data
        assert isinstance(data["id"], str)
        assert len(data["id"]) > 0

    def test_bad_csv_returns_400(self, srv):
        status, data = _post(srv, "/solve", b"wrong,columns\n1,2\n")
        assert status == 400
        assert "error" in data

    def test_unknown_path_returns_404(self, srv):
        status, data = _post(srv, "/other", ROSTER_CSV)
        assert status == 404

    def test_multipart_upload(self, srv):
        body = (
            b"--Boundary\r\n"
            b'Content-Disposition: form-data; name="file"; filename="r.csv"\r\n'
            b"\r\n"
            + ROSTER_CSV +
            b"\r\n"
            b"--Boundary--\r\n"
        )
        status, data = _post(srv, "/solve", body,
                              content_type="multipart/form-data; boundary=Boundary")
        assert status == 200
        assert "id" in data


class TestGetStatus:
    def test_unknown_job_returns_404(self, srv):
        status, data = _get_json(srv, "/status?id=doesnotexist")
        assert status == 404
        assert "error" in data

    def test_freshly_submitted_job_has_status(self, srv):
        _, submit = _post(srv, "/solve", ROSTER_CSV)
        jid = submit["id"]
        status, data = _get_json(srv, f"/status?id={jid}")
        assert status == 200
        assert data["status"] in ("running", "done")

    def test_job_completes_with_result(self, srv):
        _, submit = _post(srv, "/solve", ROSTER_CSV)
        data = _poll_done(srv, submit["id"])
        assert data["status"] == "done"
        assert "result" in data
        assert "stats" in data

    def test_completed_result_row_count(self, srv):
        _, submit = _post(srv, "/solve", ROSTER_CSV)
        data = _poll_done(srv, submit["id"])
        assert len(data["result"]) == 8  # 4 kids × 2 days

    def test_completed_result_has_expected_keys(self, srv):
        _, submit = _post(srv, "/solve", ROSTER_CSV)
        data = _poll_done(srv, submit["id"])
        row = data["result"][0]
        assert set(row) >= {"day", "boat", "name", "level"}

    def test_completed_stats_keys(self, srv):
        _, submit = _post(srv, "/solve", ROSTER_CSV)
        data = _poll_done(srv, submit["id"])
        assert {"mixed", "distinct", "repeats", "status_C"} <= set(data["stats"])


# ---------------------------------------------------------------------------
# /config db field
# ---------------------------------------------------------------------------

class TestConfigDbField:
    def test_db_false_without_db(self, srv):
        _, body = _get(srv, "/config")
        assert json.loads(body)["db"] is False

    def test_db_true_with_db(self, srv_db):
        addr, _ = srv_db
        _, body = _get(addr, "/config")
        assert json.loads(body)["db"] is True


# ---------------------------------------------------------------------------
# GET /seasons and POST /seasons without a DB
# ---------------------------------------------------------------------------

class TestSeasonsNoDb:
    def test_get_seasons_returns_empty_list(self, srv):
        _, body = _get(srv, "/seasons")
        assert json.loads(body) == []

    def test_post_seasons_returns_503(self, srv):
        status, _ = _post(srv, "/seasons", b'{"year": 2026}', "application/json")
        assert status == 503


# ---------------------------------------------------------------------------
# GET /seasons and POST /seasons with a DB
# ---------------------------------------------------------------------------

class TestSeasonsApi:
    def test_create_returns_id_and_year(self, srv_db):
        addr, _ = srv_db
        status, data = _post(addr, "/seasons", b'{"year": 2040}', "application/json")
        assert status == 200
        assert data["year"] == 2040
        assert isinstance(data["id"], int)

    def test_created_season_appears_in_list(self, srv_db):
        addr, _ = srv_db
        _post(addr, "/seasons", b'{"year": 2041}', "application/json")
        _, body = _get(addr, "/seasons")
        years = [s["year"] for s in json.loads(body)]
        assert 2041 in years

    def test_list_newest_first(self, srv_db):
        addr, _ = srv_db
        for y in [b'{"year": 2043}', b'{"year": 2044}', b'{"year": 2045}']:
            _post(addr, "/seasons", y, "application/json")
        _, body = _get(addr, "/seasons")
        years = [s["year"] for s in json.loads(body)]
        assert years == sorted(years, reverse=True)

    def test_duplicate_year_returns_500(self, srv_db):
        addr, _ = srv_db
        _post(addr, "/seasons", b'{"year": 2042}', "application/json")
        status, _ = _post(addr, "/seasons", b'{"year": 2042}', "application/json")
        assert status == 500

    def test_bad_body_returns_400(self, srv_db):
        addr, _ = srv_db
        status, _ = _post(addr, "/seasons", b"not json", "application/json")
        assert status == 400


# ---------------------------------------------------------------------------
# POST /solve persists to DB
# ---------------------------------------------------------------------------

class TestSolveWithDb:
    def test_schedule_row_persisted(self, srv_db):
        addr, db_conn = srv_db
        _, season = _post(addr, "/seasons", b'{"year": 2050}', "application/json")
        season_id = season["id"]
        _, submit = _post(addr, f"/solve?season_id={season_id}", ROSTER_CSV)
        _poll_done(addr, submit["id"])
        row = _poll_db(db_conn,
                       "SELECT id FROM schedule WHERE season_id=?", (season_id,))
        assert row is not None

    def test_boat_assignments_persisted(self, srv_db):
        addr, db_conn = srv_db
        _, season = _post(addr, "/seasons", b'{"year": 2051}', "application/json")
        season_id = season["id"]
        _, submit = _post(addr, f"/solve?season_id={season_id}", ROSTER_CSV)
        _poll_done(addr, submit["id"])
        schedule_row = _poll_db(db_conn,
                                "SELECT id FROM schedule WHERE season_id=?",
                                (season_id,))
        assert schedule_row is not None
        count = db_conn.execute(
            "SELECT COUNT(*) FROM boat_assignment WHERE schedule_id=?",
            (schedule_row["id"],)
        ).fetchone()[0]
        assert count == 8  # 4 kids × 2 days

    def test_schedule_config_json_is_valid(self, srv_db):
        addr, db_conn = srv_db
        _, season = _post(addr, "/seasons", b'{"year": 2052}', "application/json")
        season_id = season["id"]
        _, submit = _post(addr, f"/solve?season_id={season_id}", ROSTER_CSV)
        _poll_done(addr, submit["id"])
        row = _poll_db(db_conn,
                       "SELECT config FROM schedule WHERE season_id=?", (season_id,))
        assert row is not None
        cfg = json.loads(row["config"])
        assert "boats" in cfg

    def test_done_response_includes_schedule_id(self, srv_db):
        addr, _ = srv_db
        _, season = _post(addr, "/seasons", b'{"year": 2053}', "application/json")
        _, submit = _post(addr, f"/solve?season_id={season['id']}", ROSTER_CSV)
        data = _poll_done(addr, submit["id"])
        assert "schedule_id" in data
        assert isinstance(data["schedule_id"], int)


# ---------------------------------------------------------------------------
# GET /schedules and GET /schedule
# ---------------------------------------------------------------------------

class TestSchedulesApi:
    def test_empty_before_any_solve(self, srv_db):
        addr, _ = srv_db
        _, season = _post(addr, "/seasons", b'{"year": 2060}', "application/json")
        _, body = _get(addr, f"/schedules?season_id={season['id']}")
        assert json.loads(body) == []

    def test_schedule_appears_after_solve(self, srv_db):
        addr, _ = srv_db
        _, season = _post(addr, "/seasons", b'{"year": 2061}', "application/json")
        _, submit = _post(addr, f"/solve?season_id={season['id']}", ROSTER_CSV)
        _poll_done(addr, submit["id"])
        _, body = _get(addr, f"/schedules?season_id={season['id']}")
        scheds = json.loads(body)
        assert len(scheds) == 1
        assert "id" in scheds[0] and "created" in scheds[0]

    def test_get_schedule_returns_rows(self, srv_db):
        addr, _ = srv_db
        _, season = _post(addr, "/seasons", b'{"year": 2062}', "application/json")
        _, submit = _post(addr, f"/solve?season_id={season['id']}", ROSTER_CSV)
        data = _poll_done(addr, submit["id"])
        schedule_id = data["schedule_id"]
        _, body = _get(addr, f"/schedule?id={schedule_id}")
        result = json.loads(body)
        assert "rows" in result
        assert len(result["rows"]) == 8  # 4 kids × 2 days

    def test_get_schedule_row_has_expected_keys(self, srv_db):
        addr, _ = srv_db
        _, season = _post(addr, "/seasons", b'{"year": 2063}', "application/json")
        _, submit = _post(addr, f"/solve?season_id={season['id']}", ROSTER_CSV)
        data = _poll_done(addr, submit["id"])
        _, body = _get(addr, f"/schedule?id={data['schedule_id']}")
        row = json.loads(body)["rows"][0]
        assert {"day", "boat", "name", "level"} <= set(row)

    def test_schedules_no_db_returns_empty(self, srv):
        _, body = _get(srv, "/schedules?season_id=1")
        assert json.loads(body) == []

    def test_schedule_no_db_returns_503(self, srv):
        status, _ = _get(srv, "/schedule?id=1")
        assert status == 503


# ---------------------------------------------------------------------------
# POST /swap
# ---------------------------------------------------------------------------

class TestSwapApi:
    def _setup(self, srv_db, year):
        addr, _ = srv_db
        _, season = _post(addr, "/seasons",
                          json.dumps({"year": year}).encode(), "application/json")
        _, submit = _post(addr, f"/solve?season_id={season['id']}", ROSTER_CSV)
        data = _poll_done(addr, submit["id"])
        return addr, data["schedule_id"]

    def test_swap_returns_ok(self, srv_db):
        addr, schedule_id = self._setup(srv_db, 2070)
        _, body = _get(addr, f"/schedule?id={schedule_id}")
        rows = json.loads(body)["rows"]
        sat_rows = [r for r in rows if r["day"] == "Sat"]
        boats = {r["name"]: r["boat"] for r in sat_rows}
        names = list(boats)
        a, b = (names[0], next(n for n in names[1:] if boats[n] != boats[names[0]]))
        payload = json.dumps({"schedule_id": schedule_id, "day": "Sat",
                              "name_a": a, "name_b": b}).encode()
        status, data = _post(addr, "/swap", payload, "application/json")
        assert status == 200
        assert data.get("ok") is True

    def test_swap_actually_exchanges_boats(self, srv_db):
        addr, schedule_id = self._setup(srv_db, 2071)
        _, body = _get(addr, f"/schedule?id={schedule_id}")
        rows = json.loads(body)["rows"]
        sat_rows = [r for r in rows if r["day"] == "Sat"]
        boats = {r["name"]: r["boat"] for r in sat_rows}
        names = list(boats)
        a, b = (names[0], next(n for n in names[1:] if boats[n] != boats[names[0]]))
        payload = json.dumps({"schedule_id": schedule_id, "day": "Sat",
                              "name_a": a, "name_b": b}).encode()
        _post(addr, "/swap", payload, "application/json")
        _, body2 = _get(addr, f"/schedule?id={schedule_id}")
        rows2 = json.loads(body2)["rows"]
        boats2 = {r["name"]: r["boat"] for r in rows2 if r["day"] == "Sat"}
        assert boats2[a] == boats[b]
        assert boats2[b] == boats[a]

    def test_swap_no_db_returns_503(self, srv):
        payload = json.dumps({"schedule_id": 1, "day": "Sat",
                              "name_a": "Alice", "name_b": "Carol"}).encode()
        status, _ = _post(srv, "/swap", payload, "application/json")
        assert status == 503

    def test_swap_bad_body_returns_400(self, srv_db):
        addr, _ = srv_db
        status, _ = _post(addr, "/swap", b"not json", "application/json")
        assert status == 400

    def test_swap_unknown_member_returns_500(self, srv_db):
        addr, schedule_id = self._setup(srv_db, 2072)
        payload = json.dumps({"schedule_id": schedule_id, "day": "Sat",
                              "name_a": "Alice", "name_b": "Nobody"}).encode()
        status, _ = _post(addr, "/swap", payload, "application/json")
        assert status == 500
