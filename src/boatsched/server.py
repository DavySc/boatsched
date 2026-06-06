"""Web server for boatsched — FastAPI/uvicorn.

HTTP endpoints (all preserved from prior stdlib version):
  GET  /                          → viewer.html
  GET  /config                    → solver config JSON
  GET  /seasons                   → list seasons
  POST /seasons                   → create season
  GET  /boats                     → list boats
  POST /boat                      → rename boat
  GET  /schedules                 → list schedules for season
  GET  /schedule                  → get schedule rows
  POST /solve                     → start background solve job
  GET  /status                    → poll solve job

New endpoints:
  POST /swap                      → swap two kids between boats
  WS   /collab/{schedule_id}      → Yjs collaborative editing
  POST /schedule/{schedule_id}/commit → validate & write Yjs state to DB
"""
from __future__ import annotations

import argparse
import asyncio
import importlib.resources as resources
import json
import sys
import threading
import traceback
import uuid
from collections import defaultdict
from dataclasses import asdict
from typing import Optional

import pycrdt
import uvicorn
from fastapi import FastAPI, Request, WebSocket, WebSocketDisconnect
from fastapi.responses import HTMLResponse, JSONResponse

from .solver import Config, solve
from .roster import parse_roster_bytes, RosterError
from . import db as _db

JOBS: dict = {}
JOBS_LOCK = threading.Lock()


def _viewer_html() -> str:
    return resources.files("boatsched").joinpath("viewer.html").read_text(
        encoding="utf-8")


def _extract_multipart(raw: bytes, ctype: str) -> bytes:
    boundary = ctype.split("boundary=")[-1].strip().strip('"').encode()
    sep = b"--" + boundary
    for part in raw.split(sep):
        head = part.split(b"\r\n\r\n", 1)[0]
        if b"filename=" in head:
            body = part.split(b"\r\n\r\n", 1)[1]
            return body.rsplit(b"\r\n", 1)[0]
    return raw


def _init_doc_from_result(result: list) -> pycrdt.Doc:
    """Build a Y.Doc from solver result rows: Map 'schedule' keyed '{day}:{boat}'."""
    doc = pycrdt.Doc()
    sch = doc.get("schedule", type=pycrdt.Map)
    groups: dict[str, list] = {}
    for row in result:
        key = f"{row['day']}:{row['boat']}"
        groups.setdefault(key, []).append(row["name"])
    with doc.transaction():
        for key, names in sorted(groups.items()):
            sch[key] = pycrdt.Array(names)
    return doc


def _run_job(job_id: str, kids, cfg: Config,
             db_conn=None, season_id=None) -> None:
    try:
        rows, stats = solve(kids, cfg)
        result = [{"day": d, "boat": b, "name": n, "level": l}
                  for (d, b, n, l) in rows]
        schedule_id = None
        if db_conn is not None and season_id is not None:
            try:
                schedule_id = _db.save_schedule(
                    db_conn, int(season_id), json.dumps(asdict(cfg)), result)
                doc = _init_doc_from_result(result)
                _db.set_yjs_state(db_conn, schedule_id, doc.get_update())
            except Exception as exc:
                print(f"[db] save_schedule failed: {exc}", file=sys.stderr)
        with JOBS_LOCK:
            JOBS[job_id].update(status="done", result=result, stats=stats,
                                schedule_id=schedule_id)
    except BaseException as e:
        with JOBS_LOCK:
            JOBS[job_id].update(status="error",
                                error=str(e) or e.__class__.__name__,
                                trace=traceback.format_exc())


# ---------------------------------------------------------------------------
# y-websocket binary protocol helpers (lib0 varint encoding)
# ---------------------------------------------------------------------------

MSG_SYNC = 0
MSG_AWARENESS = 1
SYNC_STEP1 = 0
SYNC_STEP2 = 1
SYNC_UPDATE = 2


def _write_varint(n: int) -> bytes:
    out = []
    while n > 127:
        out.append((n & 0x7F) | 0x80)
        n >>= 7
    out.append(n & 0x7F)
    return bytes(out)


def _read_varint(buf: bytes, pos: int) -> tuple:
    result, shift = 0, 0
    while True:
        b = buf[pos]; pos += 1
        result |= (b & 0x7F) << shift
        if not (b & 0x80):
            break
        shift += 7
    return result, pos


def _write_buf(data: bytes) -> bytes:
    return _write_varint(len(data)) + data


def _read_buf(buf: bytes, pos: int) -> tuple:
    n, pos = _read_varint(buf, pos)
    return buf[pos:pos + n], pos + n


def _encode_sync_step1(sv: bytes) -> bytes:
    return _write_varint(MSG_SYNC) + _write_varint(SYNC_STEP1) + _write_buf(sv)


def _encode_sync_step2(update: bytes) -> bytes:
    return _write_varint(MSG_SYNC) + _write_varint(SYNC_STEP2) + _write_buf(update)


def _encode_update_msg(update: bytes) -> bytes:
    return _write_varint(MSG_SYNC) + _write_varint(SYNC_UPDATE) + _write_buf(update)


def _parse_message(buf: bytes):
    """Return (msg_type, sub_type, payload) or None on error."""
    try:
        pos = 0
        msg_type, pos = _read_varint(buf, pos)
        sub_type, pos = _read_varint(buf, pos)
        payload, pos = _read_buf(buf, pos)
        return msg_type, sub_type, payload
    except (IndexError, KeyError):
        return None


# ---------------------------------------------------------------------------
# App factory
# ---------------------------------------------------------------------------

def create_app(cfg: Config, db_conn=None) -> FastAPI:
    app = FastAPI(title="boatsched")

    # Run Yjs column migration once on startup
    if db_conn is not None:
        _db.add_yjs_state_column(db_conn)

    # Per-schedule Yjs docs and connected WebSocket sets (app-scoped state)
    docs: dict[int, pycrdt.Doc] = {}
    rooms: dict[int, set] = defaultdict(set)
    room_lock = asyncio.Lock()

    # -----------------------------------------------------------------------
    # Internal helpers
    # -----------------------------------------------------------------------

    async def _get_doc(schedule_id: int) -> pycrdt.Doc:
        """Return (or lazily load from DB) the Yjs doc for a schedule."""
        if schedule_id not in docs:
            state = None
            if db_conn is not None:
                state = await asyncio.to_thread(
                    _db.get_yjs_state, db_conn, schedule_id)
            doc = pycrdt.Doc()
            if state:
                doc.apply_update(state)
            docs[schedule_id] = doc
        return docs[schedule_id]

    # -----------------------------------------------------------------------
    # Static / config
    # -----------------------------------------------------------------------

    @app.get("/", response_class=HTMLResponse)
    @app.get("/index.html", response_class=HTMLResponse)
    async def root():
        return _viewer_html()

    @app.get("/config")
    async def config_endpoint():
        return {
            "boats": cfg.boats,
            "levels": list(cfg.levels),
            "restricted": list(cfg.restricted),
            "diverse": list(cfg.diverse),
            "db": db_conn is not None,
        }

    # -----------------------------------------------------------------------
    # Seasons
    # -----------------------------------------------------------------------

    @app.get("/seasons")
    async def get_seasons():
        if db_conn is None:
            return []
        return await asyncio.to_thread(_db.list_seasons, db_conn)

    @app.post("/seasons")
    async def create_season(request: Request):
        if db_conn is None:
            return JSONResponse({"error": "no database configured"}, status_code=503)
        try:
            body = await request.json()
            year = int(body["year"])
        except Exception as e:
            return JSONResponse({"error": str(e)}, status_code=400)
        try:
            sid = await asyncio.to_thread(_db.create_season, db_conn, year)
            return {"id": sid, "year": year}
        except Exception as e:
            return JSONResponse({"error": str(e)}, status_code=500)

    # -----------------------------------------------------------------------
    # Boats
    # -----------------------------------------------------------------------

    @app.get("/boats")
    async def get_boats():
        if db_conn is None:
            return []
        return await asyncio.to_thread(_db.list_boats, db_conn)

    @app.post("/boat")
    async def rename_boat_endpoint(request: Request):
        if db_conn is None:
            return JSONResponse({"error": "no database"}, status_code=503)
        try:
            body = await request.json()
            boat_id = int(body["id"])
            name = body.get("name") or None
        except Exception as e:
            return JSONResponse({"error": str(e)}, status_code=400)
        try:
            await asyncio.to_thread(_db.rename_boat, db_conn, boat_id, name)
            return {"ok": True}
        except Exception as e:
            return JSONResponse({"error": str(e)}, status_code=500)

    # -----------------------------------------------------------------------
    # Schedules
    # -----------------------------------------------------------------------

    @app.get("/schedules")
    async def get_schedules(season_id: Optional[int] = None):
        if db_conn is None or season_id is None:
            return []
        return await asyncio.to_thread(_db.list_schedules, db_conn, season_id)

    @app.get("/schedule")
    async def get_schedule(id: Optional[int] = None):
        if db_conn is None:
            return JSONResponse({"error": "no database"}, status_code=503)
        if id is None:
            return JSONResponse({"error": "missing id"}, status_code=400)
        try:
            result = await asyncio.to_thread(_db.get_schedule, db_conn, id)
            return {"rows": result}
        except Exception as e:
            return JSONResponse({"error": str(e)}, status_code=500)

    # -----------------------------------------------------------------------
    # Solve
    # -----------------------------------------------------------------------

    @app.post("/solve")
    async def solve_endpoint(request: Request, season_id: Optional[str] = None):
        ctype = request.headers.get("content-type", "")
        raw = await request.body()
        if ctype.startswith("multipart/form-data"):
            raw = _extract_multipart(raw, ctype)
        try:
            kids = parse_roster_bytes(raw)
        except RosterError as e:
            return JSONResponse({"error": str(e)}, status_code=400)
        if db_conn is not None:
            try:
                await asyncio.to_thread(_db.add_or_update_members, db_conn, kids)
            except Exception as e:
                return JSONResponse({"error": f"db error: {e}"}, status_code=500)
        jid = uuid.uuid4().hex[:12]
        with JOBS_LOCK:
            JOBS[jid] = dict(status="running", result=None, error=None, stats=None)
        threading.Thread(
            target=_run_job,
            args=(jid, kids, cfg, db_conn, season_id),
            daemon=True,
        ).start()
        return {"id": jid}

    @app.get("/status")
    async def get_status(id: str = ""):
        with JOBS_LOCK:
            job = JOBS.get(id)
        if not job:
            return JSONResponse({"error": "no such job"}, status_code=404)
        out = {"status": job["status"]}
        if job["status"] == "done":
            out["result"] = job["result"]
            out["stats"] = job["stats"]
            if job.get("schedule_id") is not None:
                out["schedule_id"] = job["schedule_id"]
        elif job["status"] == "error":
            out["error"] = job["error"]
        return out

    # -----------------------------------------------------------------------
    # Swap
    # -----------------------------------------------------------------------

    @app.post("/swap")
    async def swap_endpoint(request: Request):
        if db_conn is None:
            return JSONResponse({"error": "no database"}, status_code=503)
        try:
            body = await request.json()
            schedule_id = int(body["schedule_id"])
            day = str(body["day"])
            name_a = str(body["name_a"])
            name_b = str(body["name_b"])
        except Exception as e:
            return JSONResponse({"error": str(e)}, status_code=400)
        try:
            await asyncio.to_thread(
                _db.swap_assignment, db_conn, schedule_id, day, name_a, name_b)
            return {"ok": True}
        except Exception as e:
            return JSONResponse({"error": str(e)}, status_code=500)

    # -----------------------------------------------------------------------
    # Yjs commit
    # -----------------------------------------------------------------------

    @app.post("/schedule/{schedule_id}/commit")
    async def commit_schedule(schedule_id: int):
        if db_conn is None:
            return JSONResponse({"error": "no database"}, status_code=503)

        doc = await _get_doc(schedule_id)
        sch = doc.get("schedule", type=pycrdt.Map)

        seen: dict = {}   # (name, day) -> boat_id
        conflicts = []
        for key, members in sch.items():
            colon = key.find(":")
            if colon < 0:
                continue
            day, boat = key[:colon], int(key[colon + 1:])
            for name in members:
                k = (name, day)
                if k in seen:
                    conflicts.append(
                        {"name": name, "day": day, "boats": [seen[k], boat]})
                else:
                    seen[k] = boat

        if conflicts:
            return {"ok": False, "conflicts": conflicts}

        assignments = [(name, day, boat) for (name, day), boat in seen.items()]
        try:
            await asyncio.to_thread(
                _db.replace_assignments, db_conn, schedule_id, assignments)
            return {"ok": True}
        except Exception as e:
            return JSONResponse({"error": str(e)}, status_code=500)

    # -----------------------------------------------------------------------
    # WebSocket /collab/{schedule_id} — y-websocket compatible sync protocol
    # -----------------------------------------------------------------------

    @app.websocket("/collab/{schedule_id}")
    async def collab_ws(ws: WebSocket, schedule_id: int):
        if db_conn is None:
            await ws.close(code=1011, reason="no database")
            return

        await ws.accept()
        doc = await _get_doc(schedule_id)

        async with room_lock:
            rooms[schedule_id].add(ws)

        async def _broadcast(data: bytes, sender: WebSocket) -> None:
            async with room_lock:
                others = set(rooms[schedule_id]) - {sender}
            for peer in others:
                try:
                    await peer.send_bytes(data)
                except Exception:
                    pass

        try:
            # Send sync step 1 so the client can catch up
            await ws.send_bytes(_encode_sync_step1(doc.get_state()))

            while True:
                raw = await ws.receive_bytes()
                parsed = _parse_message(raw)
                if parsed is None:
                    continue

                msg_type, sub_type, payload = parsed

                if msg_type == MSG_SYNC:
                    if sub_type == SYNC_STEP1:
                        # Client sent their state vector → send what they're missing
                        missing = doc.get_update(payload)
                        await ws.send_bytes(_encode_sync_step2(missing))
                    elif sub_type in (SYNC_STEP2, SYNC_UPDATE):
                        # Client sent updates → apply, persist, broadcast
                        doc.apply_update(payload)
                        state = doc.get_update()
                        await asyncio.to_thread(
                            _db.set_yjs_state, db_conn, schedule_id, state)
                        await _broadcast(_encode_update_msg(payload), ws)

                elif msg_type == MSG_AWARENESS:
                    await _broadcast(raw, ws)

        except WebSocketDisconnect:
            pass
        finally:
            async with room_lock:
                rooms[schedule_id].discard(ws)

    # -----------------------------------------------------------------------
    # 404 fallthrough
    # -----------------------------------------------------------------------

    @app.api_route("/{path:path}", methods=["GET", "POST", "PUT", "DELETE"])
    async def not_found(path: str):
        return JSONResponse({"error": "not found"}, status_code=404)

    return app


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------

def parse_args(argv=None):
    p = argparse.ArgumentParser(prog="boatsched-server")
    p.add_argument("--port", type=int, default=8000)
    p.add_argument("--host", default="0.0.0.0")
    p.add_argument("--boats", type=int, default=13)
    p.add_argument("--levels", nargs="+", default=["k", "b", "A", "B"])
    p.add_argument("--restricted", nargs="*",
                   default=["Sat", "Sun", "Tue", "Wed"])
    p.add_argument("--diverse", nargs="*", default=["Fri", "Mon"])
    p.add_argument("--adj", type=int, default=1)
    p.add_argument("--stage-seconds", type=float, default=45, dest="stage_seconds")
    p.add_argument("--seconds", type=float, default=60)
    p.add_argument("--workers", type=int, default=8)
    p.add_argument("--db", default=None, metavar="PATH",
                   help="SQLite database file (created if absent)")
    return p.parse_args(argv)


def main(argv=None) -> int:
    a = parse_args(argv)
    cfg = Config(boats=a.boats, levels=a.levels, restricted=a.restricted,
                 diverse=a.diverse, adj=a.adj, stage_seconds=a.stage_seconds,
                 seconds=a.seconds, workers=a.workers)
    db_conn = _db.init_db(a.db) if a.db else None
    app = create_app(cfg, db_conn)
    print(f"boatsched server at http://localhost:{a.port}")
    print(f"  boats={cfg.boats} levels={list(cfg.levels)}")
    print(f"  restricted={list(cfg.restricted)} diverse={list(cfg.diverse)}")
    print("Ctrl-C to stop.")
    uvicorn.run(app, host=a.host, port=a.port)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
