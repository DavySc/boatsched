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
from datetime import date as _date
import importlib.resources as resources
import json
import secrets
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

    # Run DB column migrations once on startup
    if db_conn is not None:
        _db.add_yjs_state_column(db_conn)
        _db.add_start_date_column(db_conn)
        _db.add_camp_tables(db_conn)

    # Per-schedule Yjs docs and connected WebSocket sets (app-scoped state)
    docs: dict[int, pycrdt.Doc] = {}
    rooms: dict[int, set] = defaultdict(set)
    room_lock = asyncio.Lock()

    # In-memory session store: token → member_id
    sessions: dict[str, int] = {}

    def _is_open() -> bool:
        """True when no instructor has a password (bootstrap mode — auth not enforced)."""
        if db_conn is None:
            return True
        row = db_conn.execute(
            "SELECT 1 FROM instructor WHERE password_hash IS NOT NULL LIMIT 1"
        ).fetchone()
        return row is None

    def _auth_member(token: Optional[str]) -> Optional[int]:
        """Return member_id for a valid session token, 0 in open/no-DB mode, else None."""
        if token and token in sessions:
            return sessions[token]
        if _is_open():
            return 0  # bootstrap: no passwords set yet
        return None

    def _is_locked(schedule_id: int) -> bool:
        """True when the schedule's start_date is today or in the past."""
        if db_conn is None:
            return False
        start = _db.get_schedule_start_date(db_conn, schedule_id)
        if not start:
            return False
        try:
            return _date.fromisoformat(start) <= _date.today()
        except ValueError:
            return False

    @app.middleware("http")
    async def _auth_middleware(request: Request, call_next):
        """Require auth for all paths except the HTML root and auth endpoints."""
        public = {"/", "/index.html", "/api/auth/login", "/api/auth/logout"}
        path = request.url.path
        if path not in public:
            token = request.cookies.get("bsession")
            member_id = _auth_member(token)
            if member_id is None:
                return JSONResponse({"error": "not authenticated"}, status_code=401)
        return await call_next(request)

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
        return HTMLResponse(_viewer_html())

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
    # Auth — login / logout / session info
    # -----------------------------------------------------------------------

    @app.post("/logout")
    async def do_logout(request: Request):
        token = request.cookies.get("bsession")
        if token:
            sessions.pop(token, None)
        resp = JSONResponse({"ok": True})
        resp.delete_cookie("bsession")
        return resp

    @app.get("/me")
    async def me(request: Request):
        token = request.cookies.get("bsession")
        member_id = _auth_member(token)
        if member_id is None:
            return JSONResponse({"error": "not authenticated"}, status_code=401)
        if member_id == 0:
            return {"member_id": 0, "name": None}
        row = await asyncio.to_thread(lambda: db_conn.execute(
            "SELECT m.name FROM member m JOIN instructor i ON i.member_id = m.id"
            " WHERE m.id=?", (member_id,)
        ).fetchone())
        return {"member_id": member_id, "name": row["name"] if row else None}

    # -----------------------------------------------------------------------
    # Instructor password management
    # -----------------------------------------------------------------------

    @app.get("/instructors")
    async def get_instructors():
        if db_conn is None:
            return []
        return await asyncio.to_thread(_db.list_instructors, db_conn)

    @app.post("/instructor")
    async def create_instructor(request: Request):
        if db_conn is None:
            return JSONResponse({"error": "no database"}, status_code=503)
        try:
            body = await request.json()
            name = str(body["name"]).strip()
            password = str(body["password"])
        except Exception as e:
            return JSONResponse({"error": str(e)}, status_code=400)
        if not name or not password:
            return JSONResponse({"error": "name and password are required"}, status_code=400)

        def _create():
            row = db_conn.execute(
                "SELECT id FROM member WHERE name=?", (name,)
            ).fetchone()
            member_id = row["id"] if row else db_conn.execute(
                "INSERT INTO member (name, level) VALUES (?, NULL)", (name,)
            ).lastrowid
            already = db_conn.execute(
                "SELECT 1 FROM instructor WHERE member_id=?", (member_id,)
            ).fetchone()
            if already:
                return None
            db_conn.execute(
                "INSERT INTO instructor (member_id) VALUES (?)", (member_id,)
            )
            db_conn.commit()
            return member_id

        member_id = await asyncio.to_thread(_create)
        if member_id is None:
            return JSONResponse({"error": "already an instructor"}, status_code=409)
        await asyncio.to_thread(_db.set_instructor_password, db_conn, member_id, password)
        return {"ok": True}

    @app.post("/set-password")
    async def set_password(request: Request):
        if db_conn is None:
            return JSONResponse({"error": "no database"}, status_code=503)
        try:
            body = await request.json()
            name = str(body["name"]).strip()
            password = str(body["password"])
        except Exception as e:
            return JSONResponse({"error": str(e)}, status_code=400)
        if not password:
            return JSONResponse({"error": "password must not be empty"}, status_code=400)
        row = await asyncio.to_thread(lambda: db_conn.execute(
            "SELECT i.member_id FROM instructor i JOIN member m ON m.id = i.member_id"
            " WHERE m.name=?", (name,)
        ).fetchone())
        if row is None:
            return JSONResponse({"error": "not an instructor"}, status_code=404)
        # In auth mode, only authenticated instructors may change passwords
        if not _is_open():
            token = request.cookies.get("bsession")
            if _auth_member(token) is None:
                return JSONResponse({"error": "not authenticated"}, status_code=401)
        await asyncio.to_thread(
            _db.set_instructor_password, db_conn, row["member_id"], password)
        return {"ok": True}

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
            start_date = await asyncio.to_thread(
                _db.get_schedule_start_date, db_conn, id)
            return {"rows": result, "start_date": start_date}
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

    @app.post("/schedule/{schedule_id}/start-date")
    async def set_start_date(schedule_id: int, request: Request):
        if db_conn is None:
            return JSONResponse({"error": "no database"}, status_code=503)
        body = await request.json()
        date_str = body.get("start_date") or None
        try:
            await asyncio.to_thread(
                _db.set_schedule_start_date, db_conn, schedule_id, date_str)
            return {"ok": True}
        except Exception as e:
            return JSONResponse({"error": str(e)}, status_code=500)

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
        if _is_locked(schedule_id):
            return JSONResponse({"error": "schedule is locked"}, status_code=403)
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
        if _is_locked(schedule_id):
            return JSONResponse({"error": "schedule is locked"}, status_code=403)

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
        token = ws.cookies.get("bsession")
        if _auth_member(token) is None:
            await ws.close(code=1008, reason="not authenticated")
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
                        if not _is_locked(schedule_id):
                            # Apply, persist, and broadcast only when not locked
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
    # JSON auth endpoints
    # -----------------------------------------------------------------------

    @app.post("/api/auth/login")
    async def api_login(request: Request):
        try:
            body = await request.json()
            name = str(body.get("name", "")).strip()
            password = str(body.get("password", ""))
        except Exception:
            return JSONResponse({"error": "invalid request"}, status_code=400)
        if db_conn is None or _is_open():
            # No auth configured — accept any credentials
            token = secrets.token_urlsafe(32)
            sessions[token] = 0
            resp = JSONResponse({"ok": True, "name": name or "instructor"})
            resp.set_cookie("bsession", token, httponly=True, samesite="lax")
            return resp
        member_id = await asyncio.to_thread(
            _db.verify_instructor_login, db_conn, name, password)
        if member_id is None:
            return JSONResponse({"error": "Onjuiste naam of wachtwoord."}, status_code=401)
        token = secrets.token_urlsafe(32)
        sessions[token] = member_id
        resp = JSONResponse({"ok": True, "name": name})
        resp.set_cookie("bsession", token, httponly=True, samesite="lax")
        return resp

    @app.post("/api/auth/logout")
    async def api_logout(request: Request):
        token = request.cookies.get("bsession")
        if token:
            sessions.pop(token, None)
        resp = JSONResponse({"ok": True})
        resp.delete_cookie("bsession")
        return resp

    @app.get("/api/auth/me")
    async def api_me(request: Request):
        token = request.cookies.get("bsession")
        member_id = _auth_member(token)
        if member_id is None:
            return JSONResponse({"error": "not authenticated"}, status_code=401)
        if member_id == 0 or db_conn is None:
            return {"member_id": 0, "name": "instructor"}
        row = await asyncio.to_thread(lambda: db_conn.execute(
            "SELECT m.name FROM member m JOIN instructor i ON i.member_id = m.id"
            " WHERE m.id=?", (member_id,)
        ).fetchone())
        return {"member_id": member_id, "name": row["name"] if row else None}

    # -----------------------------------------------------------------------
    # API: health
    # -----------------------------------------------------------------------

    @app.get("/api/health")
    async def api_health():
        return {"status": "healthy", "timestamp": __import__("datetime").datetime.utcnow().isoformat(), "version": "1.0"}

    # -----------------------------------------------------------------------
    # API: students (camp members for the evaluation tracker)
    # -----------------------------------------------------------------------

    @app.get("/api/students")
    async def api_get_students():
        if db_conn is None:
            return {"students": []}
        students = await asyncio.to_thread(_db.list_students, db_conn)
        return {"students": students}

    @app.post("/api/students")
    async def api_save_students(request: Request):
        if db_conn is None:
            return JSONResponse({"error": "no database"}, status_code=503)
        try:
            body = await request.json()
            students = body.get("students", [])
            if not isinstance(students, list):
                return JSONResponse({"error": "students must be an array"}, status_code=400)
        except Exception as e:
            return JSONResponse({"error": str(e)}, status_code=400)
        result = await asyncio.to_thread(_db.save_students, db_conn, students)
        return {"students": result}

    @app.put("/api/students/{student_id}")
    async def api_update_student(student_id: str, request: Request):
        if db_conn is None:
            return JSONResponse({"error": "no database"}, status_code=503)
        try:
            data = await request.json()
        except Exception as e:
            return JSONResponse({"error": str(e)}, status_code=400)
        if not student_id.isdigit():
            return JSONResponse({"error": "invalid id"}, status_code=400)
        student = await asyncio.to_thread(
            _db.update_student, db_conn, int(student_id), data)
        if student is None:
            return JSONResponse({"error": "not found"}, status_code=404)
        return {"student": student}

    @app.delete("/api/students/{student_id}")
    async def api_delete_student(student_id: str):
        if db_conn is None:
            return JSONResponse({"error": "no database"}, status_code=503)
        if not student_id.isdigit():
            return JSONResponse({"error": "invalid id"}, status_code=400)
        await asyncio.to_thread(_db.delete_student, db_conn, int(student_id))
        return {"ok": True}

    # -----------------------------------------------------------------------
    # API: progress records
    # -----------------------------------------------------------------------

    @app.get("/api/progress")
    async def api_get_all_progress():
        if db_conn is None:
            return {"progressData": []}
        data = await asyncio.to_thread(_db.get_all_progress, db_conn)
        return {"progressData": data}

    @app.get("/api/progress/{student_id}")
    async def api_get_progress(student_id: str):
        if db_conn is None:
            return {"progress": None}
        data = await asyncio.to_thread(_db.get_progress, db_conn, student_id)
        return {"progress": data}

    @app.post("/api/progress")
    async def api_save_progress(request: Request):
        if db_conn is None:
            return JSONResponse({"error": "no database"}, status_code=503)
        try:
            progress = await request.json()
            if not progress.get("studentId"):
                return JSONResponse({"error": "studentId required"}, status_code=400)
        except Exception as e:
            return JSONResponse({"error": str(e)}, status_code=400)
        result = await asyncio.to_thread(_db.save_progress, db_conn, progress)
        return {"progress": result}

    @app.delete("/api/progress/{student_id}")
    async def api_delete_progress(student_id: str):
        if db_conn is None:
            return JSONResponse({"error": "no database"}, status_code=503)
        await asyncio.to_thread(_db.delete_progress, db_conn, student_id)
        return {"ok": True}

    # -----------------------------------------------------------------------
    # API: day allocations (manual boat tables from the React UI)
    # -----------------------------------------------------------------------

    @app.get("/api/boat-allocations")
    async def api_get_all_allocations():
        if db_conn is None:
            return {"boatAllocations": []}
        data = await asyncio.to_thread(_db.get_all_day_allocations, db_conn)
        return {"boatAllocations": data}

    @app.get("/api/boat-allocations/{day}")
    async def api_get_allocation(day: str):
        if db_conn is None:
            return {"allocation": None}
        decoded = __import__("urllib.parse", fromlist=["unquote"]).unquote(day)
        data = await asyncio.to_thread(_db.get_day_allocation, db_conn, decoded)
        return {"allocation": data}

    @app.post("/api/boat-allocations")
    async def api_save_allocation(request: Request):
        if db_conn is None:
            return JSONResponse({"error": "no database"}, status_code=503)
        try:
            allocation = await request.json()
            if not allocation.get("day"):
                return JSONResponse({"error": "day required"}, status_code=400)
        except Exception as e:
            return JSONResponse({"error": str(e)}, status_code=400)
        result = await asyncio.to_thread(_db.save_day_allocation, db_conn, allocation)
        return {"allocation": result}

    @app.delete("/api/boat-allocations/{day}")
    async def api_delete_allocation(day: str):
        if db_conn is None:
            return JSONResponse({"error": "no database"}, status_code=503)
        decoded = __import__("urllib.parse", fromlist=["unquote"]).unquote(day)
        await asyncio.to_thread(_db.delete_day_allocation, db_conn, decoded)
        return {"ok": True}

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
