"""Tests for Yjs collaborative editing — WebSocket sync and commit endpoint."""
import json
import time

import pycrdt
import pytest
from fastapi.testclient import TestClient

from boatsched import Config
from boatsched import db as _db
from boatsched.server import (
    MSG_AWARENESS, MSG_SYNC,
    SYNC_STEP1, SYNC_STEP2, SYNC_UPDATE,
    _encode_sync_step1, _encode_sync_step2, _encode_update_msg,
    _parse_message,
    create_app,
)

ROSTER_CSV = b"name,level\nAlice,k\nBob,k\nCarol,b\nDave,b\n"
CFG = Config(
    boats=2, levels=["k", "b"],
    restricted=["Sat"], diverse=["Fri"],
    stage_seconds=15, seconds=15,
)


@pytest.fixture
def collab_client(tmp_path):
    db_conn = _db.init_db(str(tmp_path / "collab.db"))
    # Carol and Dave are returning (non-k) members in ROSTER_CSV; pre-seed them.
    db_conn.executemany("INSERT INTO member (name) VALUES (?)",
                        [("Carol",), ("Dave",)])
    db_conn.commit()
    app = create_app(CFG, db_conn)
    with TestClient(app, raise_server_exceptions=True) as client:
        yield client, db_conn


def _post_json(client, path, payload):
    r = client.post(path, content=json.dumps(payload).encode(),
                    headers={"Content-Type": "application/json"})
    return r.status_code, r.json()


def _solve_and_wait(client, season_id, timeout=60):
    r = client.post(f"/solve?season_id={season_id}",
                    content=ROSTER_CSV, headers={"Content-Type": "text/csv"})
    jid = r.json()["id"]
    deadline = time.time() + timeout
    while time.time() < deadline:
        j = client.get(f"/status?id={jid}").json()
        if j["status"] != "running":
            break
        time.sleep(0.2)
    return j


def _create_schedule(client):
    _, s = _post_json(client, "/seasons", {"year": 2300})
    j = _solve_and_wait(client, s["id"])
    assert j["status"] == "done", f"solve failed: {j}"
    return j["schedule_id"]


# ---------------------------------------------------------------------------
# WebSocket sync protocol
# ---------------------------------------------------------------------------

class TestWebSocketSync:
    def test_server_sends_sync_step1_on_connect(self, collab_client):
        client, _ = collab_client
        sid = _create_schedule(client)
        with client.websocket_connect(f"/collab/{sid}") as ws:
            msg = ws.receive_bytes()
            parsed = _parse_message(msg)
            assert parsed is not None
            msg_type, sub_type, sv = parsed
            assert msg_type == MSG_SYNC
            assert sub_type == SYNC_STEP1
            assert len(sv) > 0  # non-empty state vector

    def test_client_sync_step1_gets_step2_response(self, collab_client):
        client, _ = collab_client
        sid = _create_schedule(client)
        with client.websocket_connect(f"/collab/{sid}") as ws:
            ws.receive_bytes()  # server's step1

            # Send our step1 (empty doc)
            doc = pycrdt.Doc()
            ws.send_bytes(_encode_sync_step1(doc.get_state()))

            # Server should reply with step2 (full update)
            reply = ws.receive_bytes()
            parsed = _parse_message(reply)
            assert parsed is not None
            msg_type, sub_type, payload = parsed
            assert msg_type == MSG_SYNC
            assert sub_type == SYNC_STEP2
            assert len(payload) > 0  # has something to send

    def test_sync_step2_populates_doc(self, collab_client):
        client, _ = collab_client
        sid = _create_schedule(client)
        with client.websocket_connect(f"/collab/{sid}") as ws:
            ws.receive_bytes()  # server's step1

            doc = pycrdt.Doc()
            ws.send_bytes(_encode_sync_step1(doc.get_state()))

            reply = ws.receive_bytes()
            _, _, update = _parse_message(reply)
            doc.apply_update(update)

            sch = doc.get("schedule", type=pycrdt.Map)
            keys = list(sch.keys())
            assert len(keys) > 0  # schedule has entries
            # All keys must be "{day}:{boat}" format
            for k in keys:
                assert ":" in k

    def test_schedule_has_correct_member_count(self, collab_client):
        client, _ = collab_client
        sid = _create_schedule(client)
        with client.websocket_connect(f"/collab/{sid}") as ws:
            ws.receive_bytes()
            doc = pycrdt.Doc()
            ws.send_bytes(_encode_sync_step1(doc.get_state()))
            _, _, update = _parse_message(ws.receive_bytes())
            doc.apply_update(update)
            sch = doc.get("schedule", type=pycrdt.Map)
            total = sum(len(list(v)) for v in sch.values())
            assert total == 8  # 4 kids × 2 days


# ---------------------------------------------------------------------------
# Two-client convergence
# ---------------------------------------------------------------------------

def _full_sync(ws, doc=None):
    """Perform a full sync with the server, return the populated doc."""
    if doc is None:
        doc = pycrdt.Doc()
    ws.receive_bytes()  # server's step1 (drain it)
    ws.send_bytes(_encode_sync_step1(doc.get_state()))
    reply = ws.receive_bytes()
    parsed = _parse_message(reply)
    if parsed and parsed[1] == SYNC_STEP2:
        doc.apply_update(parsed[2])
    return doc


class TestTwoClientConvergence:
    def test_both_clients_see_same_initial_state(self, collab_client):
        client, _ = collab_client
        sid = _create_schedule(client)
        with (client.websocket_connect(f"/collab/{sid}") as ws1,
              client.websocket_connect(f"/collab/{sid}") as ws2):
            doc1 = _full_sync(ws1)
            doc2 = _full_sync(ws2)

            sch1 = doc1.get("schedule", type=pycrdt.Map)
            sch2 = doc2.get("schedule", type=pycrdt.Map)
            assert sorted(sch1.keys()) == sorted(sch2.keys())

    def test_update_from_one_client_reaches_other(self, collab_client):
        client, _ = collab_client
        sid = _create_schedule(client)
        with (client.websocket_connect(f"/collab/{sid}") as ws1,
              client.websocket_connect(f"/collab/{sid}") as ws2):
            doc1 = _full_sync(ws1)
            doc2 = _full_sync(ws2)

            # ws1 makes an edit: add a sentinel key to the schedule
            sv_before = doc1.get_state()
            sch1 = doc1.get("schedule", type=pycrdt.Map)
            with doc1.transaction():
                sch1["_sentinel"] = pycrdt.Array(["test_marker"])
            incremental = doc1.get_update(sv_before)

            # ws1 sends the incremental update
            ws1.send_bytes(_encode_update_msg(incremental))

            # ws2 should receive a broadcast
            broadcast = ws2.receive_bytes()
            parsed = _parse_message(broadcast)
            assert parsed is not None
            assert parsed[0] == MSG_SYNC
            assert parsed[1] == SYNC_UPDATE

            # Apply to doc2 and verify convergence
            doc2.apply_update(parsed[2])
            sch2 = doc2.get("schedule", type=pycrdt.Map)
            assert "_sentinel" in sch2
            assert list(sch2["_sentinel"]) == ["test_marker"]

    def test_update_persisted_to_db(self, collab_client):
        client, db_conn = collab_client
        sid = _create_schedule(client)
        with client.websocket_connect(f"/collab/{sid}") as ws:
            doc = _full_sync(ws)
            sv_before = doc.get_state()
            sch = doc.get("schedule", type=pycrdt.Map)
            with doc.transaction():
                sch["_db_check"] = pycrdt.Array(["persisted"])
            ws.send_bytes(_encode_update_msg(doc.get_update(sv_before)))
            # Give the server a moment to persist
            time.sleep(0.2)

        # Verify the state was saved to the DB
        state = _db.get_yjs_state(db_conn, sid)
        assert state is not None
        doc2 = pycrdt.Doc()
        doc2.apply_update(state)
        sch2 = doc2.get("schedule", type=pycrdt.Map)
        assert "_db_check" in sch2


# ---------------------------------------------------------------------------
# Commit endpoint
# ---------------------------------------------------------------------------

class TestCommitEndpoint:
    def test_commit_no_db_returns_503(self, tmp_path):
        app = create_app(CFG)  # no db_conn
        with TestClient(app) as client:
            r = client.post("/schedule/1/commit")
            assert r.status_code == 503

    def test_commit_valid_state_returns_ok(self, collab_client):
        client, _ = collab_client
        sid = _create_schedule(client)
        # The schedule was initialized from the solve result, commit it
        r = client.post(f"/schedule/{sid}/commit")
        assert r.status_code == 200
        assert r.json()["ok"] is True

    def test_commit_updates_boat_assignments(self, collab_client):
        client, db_conn = collab_client
        sid = _create_schedule(client)

        # Get original state
        rows_before = _db.get_schedule(db_conn, sid)
        assert len(rows_before) == 8

        # Commit without modification (should succeed and preserve rows)
        r = client.post(f"/schedule/{sid}/commit")
        assert r.json()["ok"] is True

        rows_after = _db.get_schedule(db_conn, sid)
        assert len(rows_after) == 8

    def test_commit_detects_conflict(self, collab_client):
        client, db_conn = collab_client
        sid = _create_schedule(client)

        # Inject a conflict via the WebSocket: put Alice in two boats on the same day
        with client.websocket_connect(f"/collab/{sid}") as ws:
            doc = _full_sync(ws)
            sch = doc.get("schedule", type=pycrdt.Map)

            # Find a Sat key and add Alice to a second one
            sat_keys = sorted(k for k in sch.keys() if k.startswith("Sat:"))
            assert len(sat_keys) >= 2
            k1, k2 = sat_keys[0], sat_keys[1]

            # Get Alice's name from k1
            members_k1 = list(sch[k1])
            assert len(members_k1) > 0
            alice = members_k1[0]

            sv_before = doc.get_state()
            with doc.transaction():
                sch[k2].append(alice)  # duplicate: alice is now in both k1 and k2
            ws.send_bytes(_encode_update_msg(doc.get_update(sv_before)))
            time.sleep(0.1)

        r = client.post(f"/schedule/{sid}/commit")
        data = r.json()
        assert data["ok"] is False
        assert "conflicts" in data
        assert len(data["conflicts"]) >= 1
        conflict = data["conflicts"][0]
        assert "name" in conflict and "day" in conflict and "boats" in conflict

    def test_commit_late_join_sees_current_state(self, collab_client):
        """A WebSocket connecting after edits should see the updated doc."""
        client, _ = collab_client
        sid = _create_schedule(client)

        # Make an edit via ws1
        with client.websocket_connect(f"/collab/{sid}") as ws1:
            doc1 = _full_sync(ws1)
            sv = doc1.get_state()
            sch1 = doc1.get("schedule", type=pycrdt.Map)
            with doc1.transaction():
                sch1["_late_join_key"] = pycrdt.Array(["late_marker"])
            ws1.send_bytes(_encode_update_msg(doc1.get_update(sv)))
            time.sleep(0.2)

        # Connect ws2 after ws1 has disconnected
        with client.websocket_connect(f"/collab/{sid}") as ws2:
            doc2 = _full_sync(ws2)
            sch2 = doc2.get("schedule", type=pycrdt.Map)
            assert "_late_join_key" in sch2
