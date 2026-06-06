"""Tests for boatsched.server HTTP endpoints and helpers."""
import http.client
import json
import threading
import time

import pytest

from boatsched import Config
from boatsched.server import _extract_multipart, make_handler
from http.server import ThreadingHTTPServer


ROSTER_CSV = b"name,level\nAlice,k\nBob,k\nCarol,b\nDave,b\n"


# ---------------------------------------------------------------------------
# Fixture: live server on a random port
# ---------------------------------------------------------------------------

@pytest.fixture(scope="module")
def srv():
    cfg = Config(
        boats=2, levels=["k", "b"],
        restricted=["Sat"], diverse=["Fri"],
        stage_seconds=15, seconds=15,
    )
    server = ThreadingHTTPServer(("127.0.0.1", 0), make_handler(cfg))
    port = server.server_address[1]
    t = threading.Thread(target=server.serve_forever, daemon=True)
    t.start()
    yield "127.0.0.1", port
    server.shutdown()


def _get(srv, path):
    conn = http.client.HTTPConnection(srv[0], srv[1], timeout=5)
    conn.request("GET", path)
    r = conn.getresponse()
    body = r.read()
    conn.close()
    return r.status, body


def _post(srv, path, body, content_type="text/csv"):
    conn = http.client.HTTPConnection(srv[0], srv[1], timeout=5)
    headers = {"Content-Type": content_type, "Content-Length": str(len(body))}
    conn.request("POST", path, body=body, headers=headers)
    r = conn.getresponse()
    body = r.read()
    conn.close()
    return r.status, json.loads(body)


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
        data = json.loads(body)
        assert "error" in data


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


def _get_json(srv, path):
    status, body = _get(srv, path)
    return status, json.loads(body)


def _poll_done(srv, jid, timeout=60):
    deadline = time.time() + timeout
    data = {}
    while time.time() < deadline:
        _, data = _get_json(srv, f"/status?id={jid}")
        if data["status"] != "running":
            break
        time.sleep(0.2)
    return data


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
