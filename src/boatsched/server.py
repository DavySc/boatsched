"""Web server for boatsched — FastAPI/uvicorn."""
from __future__ import annotations

import argparse
import asyncio
import importlib.resources as resources
import json
import os
import secrets
import sys
import threading
import time as _time
import traceback
import uuid
from collections import defaultdict
from dataclasses import asdict
from typing import Optional

import pycrdt
import uvicorn
from fastapi import FastAPI, Request, WebSocket, WebSocketDisconnect
from fastapi.responses import HTMLResponse, JSONResponse, Response

from .solver import Config, solve
from .roster import parse_roster_bytes, RosterError
from . import db as _db

JOBS: dict = {}
JOBS_LOCK = threading.Lock()

# Student levels are a fixed enum, enforced by the enrollment.level CHECK
# constraint in db._create_tables() and tied to the solver's level ordering.
# The level-editing API validates against this set rather than accepting free
# text, which the DB (and solver) would reject.
VALID_LEVELS = ("k", "b", "A", "B")
JOB_TTL = 3600          # seconds a finished job is retained for polling
JOB_MAX = 200           # hard cap on retained jobs (bounds memory)


def _prune_jobs() -> None:
    """Evict expired/oldest jobs. Caller must hold JOBS_LOCK."""
    now = _time.time()
    for jid in [j for j, v in JOBS.items()
                if v.get("status") != "running"
                and now - v.get("created", now) > JOB_TTL]:
        JOBS.pop(jid, None)
    if len(JOBS) > JOB_MAX:
        for jid, _v in sorted(JOBS.items(),
                              key=lambda kv: kv[1].get("created", 0))[:len(JOBS) - JOB_MAX]:
            JOBS.pop(jid, None)


def _viewer_html() -> str:
    return resources.files("boatsched").joinpath("viewer.html").read_text(
        encoding="utf-8")


def _server_error(exc: Exception, context: str = "") -> JSONResponse:
    """Log the real exception server-side, return a generic 500 to the client.

    Avoids leaking DB messages, file paths, or stack details to attackers.
    """
    print(f"[error] {context}: {exc!r}", file=sys.stderr)
    return JSONResponse({"error": "internal server error"}, status_code=500)


def _parse_names_csv(text: str) -> list:
    """Extract names from a CSV string or a plain name-per-line list.

    Supports the roster CSV format (name,level columns) and any CSV with
    a 'name' column. Falls back to treating each non-blank line as a name.
    """
    import csv
    from io import StringIO
    lines = [l for l in text.splitlines() if l.strip()]
    if not lines:
        return []
    try:
        reader = csv.DictReader(StringIO(text))
        if reader.fieldnames:
            norm = {f.strip().lower(): f for f in reader.fieldnames}
            if "name" in norm:
                key = norm["name"]
                return [r[key].strip() for r in reader if r.get(key, "").strip()]
    except Exception:
        pass
    # No 'name' header: treat each line's first token as a name
    result = []
    for line in lines:
        name = line.strip().split(",")[0].strip().strip('"')
        if name and name.lower() not in ("name", "naam"):
            result.append(name)
    return result


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

def create_app(
    cfg: Config,
    db_conn=None,
    *,
    require_auth: bool = False,
    secure_cookies: bool = False,
    session_ttl: int = 12 * 3600,
    login_max_failures: int = 20,
    login_window: int = 300,
    max_upload_bytes: int = 5 * 1024 * 1024,
    max_ws_message_bytes: int = 1024 * 1024,
    min_password_len: int = 1,
) -> FastAPI:
    """Build the FastAPI app.

    Security knobs (all default to the permissive/dev behaviour so the test
    suite is unaffected; ``main()`` enables the strict values in production):

    - ``require_auth``      — when True, bootstrap/open mode is disabled and a
                              valid session is required for every protected
                              route, even if no instructor has a password yet.
    - ``secure_cookies``    — set the ``Secure`` flag on the session cookie
                              (HTTPS-only). Enable behind a TLS terminator.
    - ``session_ttl``       — seconds before an issued session expires.
    - ``login_max_failures``/``login_window`` — per-IP failed-login throttle.
    - ``max_upload_bytes``  — reject bodies larger than this on upload routes.
    - ``max_ws_message_bytes`` — drop oversized collab frames before applying.
    """
    app = FastAPI(title="boatsched")

    # Run DB migrations once on startup
    if db_conn is not None:
        _db.add_yjs_state_column(db_conn)
        _db.add_start_date_column(db_conn)
        _db.add_enrollment_tables(db_conn)
        _db.drop_member_level_column(db_conn)
        _db.drop_legacy_camp_tables(db_conn)
        _db.add_admin_column(db_conn)
        _db.auto_grant_first_admin(db_conn)
        _db.make_name_index_nocase(db_conn)

    # Per-schedule Yjs docs and connected WebSocket sets (app-scoped state)
    docs: dict[int, pycrdt.Doc] = {}
    rooms: dict[int, set] = defaultdict(set)
    room_lock = asyncio.Lock()

    # In-memory session store: token → (member_id, expiry_epoch)
    sessions: dict[str, tuple] = {}
    # Per-IP failed-login tracking: ip → [timestamps]
    login_failures: dict[str, list] = {}

    def _client_ip(request: Request) -> str:
        """Best-effort client IP. Trusts X-Real-IP only (set by our nginx)."""
        return (request.headers.get("x-real-ip")
                or (request.client.host if request.client else "unknown"))

    def _record_login_failure(ip: str) -> None:
        now = _time.time()
        bucket = [t for t in login_failures.get(ip, []) if now - t < login_window]
        bucket.append(now)
        login_failures[ip] = bucket

    def _login_blocked(ip: str) -> bool:
        now = _time.time()
        bucket = [t for t in login_failures.get(ip, []) if now - t < login_window]
        login_failures[ip] = bucket
        return len(bucket) >= login_max_failures

    def _is_open() -> bool:
        """True only in dev: no DB, or no instructor has a password yet.

        When ``require_auth`` is set (production), open/bootstrap mode is
        disabled entirely — there is never anonymous access.
        """
        if require_auth:
            return False
        if db_conn is None:
            return True
        row = db_conn.execute(
            "SELECT 1 FROM instructor WHERE password_hash IS NOT NULL LIMIT 1"
        ).fetchone()
        return row is None

    def _member_is_admin(mid: Optional[int]) -> bool:
        """True when member_id is an admin instructor, or when in open/no-DB mode."""
        if _is_open():
            return True
        if not mid:
            return False
        row = db_conn.execute(
            "SELECT is_admin FROM instructor WHERE member_id=?", (mid,)
        ).fetchone()
        return bool(row and row["is_admin"])

    def _current_is_admin(request: Request) -> bool:
        """True when the request comes from an admin, or when in open/no-DB mode."""
        return _member_is_admin(_auth_member(request.cookies.get("bsession")))

    def _auth_member(token: Optional[str]) -> Optional[int]:
        """Return member_id for a valid, unexpired session, 0 in open mode, else None."""
        if token and token in sessions:
            member_id, expiry = sessions[token]
            if _time.time() < expiry:
                return member_id
            sessions.pop(token, None)  # expired — evict
        if _is_open():
            return 0  # bootstrap: no passwords set yet
        return None

    def _new_session(member_id: int) -> str:
        token = secrets.token_urlsafe(32)
        sessions[token] = (member_id, _time.time() + session_ttl)
        return token

    def _set_session_cookie(resp, token: str) -> None:
        resp.set_cookie(
            "bsession", token, httponly=True, samesite="lax",
            secure=secure_cookies, max_age=session_ttl, path="/",
        )

    _PUBLIC_PATHS = {
        "/", "/index.html",
        "/api/auth/login", "/api/auth/logout",
        "/api/health",  # unauthenticated so uptime monitors can probe it
    }

    @app.middleware("http")
    async def _auth_middleware(request: Request, call_next):
        """Require auth for all paths except the HTML root and auth endpoints."""
        path = request.url.path
        if path not in _PUBLIC_PATHS:
            token = request.cookies.get("bsession")
            member_id = _auth_member(token)
            if member_id is None:
                return JSONResponse({"error": "not authenticated"}, status_code=401)
        response = await call_next(request)
        # Defence-in-depth headers (nginx sets these too, but be robust if
        # the app is ever exposed without the proxy).
        response.headers.setdefault("X-Content-Type-Options", "nosniff")
        response.headers.setdefault("X-Frame-Options", "DENY")
        response.headers.setdefault("Referrer-Policy", "no-referrer")
        return response

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

    # -----------------------------------------------------------------------
    # Instructor password management
    # -----------------------------------------------------------------------

    def _weak_password(password: str):
        """Return an error response if the password is too weak, else None."""
        if len(password) < min_password_len:
            return JSONResponse(
                {"error": f"wachtwoord moet minstens {min_password_len} tekens zijn"},
                status_code=400,
            )
        return None

    @app.get("/instructors")
    async def get_instructors():
        if db_conn is None:
            return []
        return await asyncio.to_thread(_db.list_instructors, db_conn)

    @app.post("/instructor")
    async def create_instructor(request: Request):
        if db_conn is None:
            return JSONResponse({"error": "no database"}, status_code=503)
        if not _current_is_admin(request):
            return JSONResponse({"error": "admin required"}, status_code=403)
        try:
            body = await request.json()
            name = str(body["name"]).strip()
            password = str(body["password"])
        except Exception as e:
            return JSONResponse({"error": str(e)}, status_code=400)
        if not name or not password:
            return JSONResponse({"error": "name and password are required"}, status_code=400)
        weak = _weak_password(password)
        if weak is not None:
            return weak

        def _create():
            row = db_conn.execute(
                "SELECT id FROM member WHERE name=? COLLATE NOCASE", (name,)
            ).fetchone()
            member_id = row["id"] if row else db_conn.execute(
                "INSERT INTO member (name) VALUES (?)", (name,)
            ).lastrowid
            already = db_conn.execute(
                "SELECT 1 FROM instructor WHERE member_id=?", (member_id,)
            ).fetchone()
            if already:
                return None
            # First instructor auto-gets admin; subsequent ones do not.
            no_admins = db_conn.execute(
                "SELECT 1 FROM instructor WHERE is_admin=1 LIMIT 1"
            ).fetchone() is None
            db_conn.execute(
                "INSERT INTO instructor (member_id, is_admin) VALUES (?, ?)",
                (member_id, 1 if no_admins else 0),
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
        weak = _weak_password(password)
        if weak is not None:
            return weak
        row = await asyncio.to_thread(lambda: db_conn.execute(
            "SELECT i.member_id FROM instructor i JOIN member m ON m.id = i.member_id"
            " WHERE m.name=? COLLATE NOCASE", (name,)
        ).fetchone())
        if row is None:
            return JSONResponse({"error": "not an instructor"}, status_code=404)
        if not _is_open():
            token = request.cookies.get("bsession")
            caller = _auth_member(token)
            if caller is None:
                return JSONResponse({"error": "not authenticated"}, status_code=401)
            # Non-admins may only change their own password.
            if row["member_id"] != caller and not _current_is_admin(request):
                return JSONResponse({"error": "admin required to change another instructor's password"}, status_code=403)
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
        if not _current_is_admin(request):
            return JSONResponse({"error": "admin required"}, status_code=403)
        try:
            body = await request.json()
            year = int(body["year"])
        except Exception as e:
            return JSONResponse({"error": str(e)}, status_code=400)
        try:
            sid = await asyncio.to_thread(_db.create_season, db_conn, year)
            return {"id": sid, "year": year}
        except Exception as e:
            return _server_error(e)

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
            return _server_error(e)

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
            return _server_error(e)

    # -----------------------------------------------------------------------
    # Solve
    # -----------------------------------------------------------------------

    def _too_large(request: Request):
        """Reject uploads whose declared size exceeds the cap (cheap guard)."""
        cl = request.headers.get("content-length")
        if cl is not None:
            try:
                if int(cl) > max_upload_bytes:
                    return JSONResponse({"error": "payload too large"}, status_code=413)
            except ValueError:
                return JSONResponse({"error": "invalid content-length"}, status_code=400)
        return None

    @app.post("/solve")
    async def solve_endpoint(request: Request, season_id: Optional[str] = None):
        if db_conn is not None and not _current_is_admin(request):
            return JSONResponse({"error": "admin required"}, status_code=403)
        oversized = _too_large(request)
        if oversized is not None:
            return oversized
        ctype = request.headers.get("content-type", "")
        raw = await request.body()
        if len(raw) > max_upload_bytes:
            return JSONResponse({"error": "payload too large"}, status_code=413)
        if ctype.startswith("multipart/form-data"):
            raw = _extract_multipart(raw, ctype)
        try:
            kids = parse_roster_bytes(raw)
        except RosterError as e:
            return JSONResponse({"error": str(e)}, status_code=400)
        if db_conn is not None:
            try:
                sid = int(season_id) if season_id else None
                await asyncio.to_thread(_db.add_or_update_members, db_conn, kids, sid)
            except RuntimeError as e:
                # e.g. returning members missing from the roster — actionable
                # feedback for the (already authenticated admin) uploader.
                return JSONResponse({"error": str(e)}, status_code=400)
            except Exception as e:
                return _server_error(e, "solve/add_members")
        jid = uuid.uuid4().hex[:12]
        with JOBS_LOCK:
            _prune_jobs()
            JOBS[jid] = dict(status="running", result=None, error=None,
                             stats=None, created=_time.time())
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
            return _server_error(e)

    @app.post("/swap")
    async def swap_endpoint(request: Request):
        if db_conn is None:
            return JSONResponse({"error": "no database"}, status_code=503)
        # Boat assignments are admin-only at all times (before and after the
        # start date). Admins are not blocked by the start-date lock; non-admins
        # are always rejected. This replaces the former start-date lock gate.
        if not _current_is_admin(request):
            return JSONResponse(
                {"detail": "Only admins may edit boat assignments"},
                status_code=403)
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
            return _server_error(e)

    # -----------------------------------------------------------------------
    # Yjs commit
    # -----------------------------------------------------------------------

    @app.post("/schedule/{schedule_id}/commit")
    async def commit_schedule(schedule_id: int, request: Request):
        if db_conn is None:
            return JSONResponse({"error": "no database"}, status_code=503)
        # Admin-only at all times; admins are not blocked by the start-date lock.
        if not _current_is_admin(request):
            return JSONResponse(
                {"detail": "Only admins may edit boat assignments"},
                status_code=403)

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
            return _server_error(e)

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
        # Boat assignments are admin-only. Non-admins may still connect to
        # observe live edits (they receive sync/updates), but any update frame
        # they send is dropped — the WS analogue of the /swap and /commit 403.
        is_admin = _member_is_admin(_auth_member(token))

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
                # Drop oversized frames before parsing/applying so a malicious
                # client cannot bloat the persisted Y.Doc or fan it out to peers.
                # Legitimate Yjs updates are a few KB; the cap is very generous.
                if len(raw) > max_ws_message_bytes:
                    continue
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
                        if is_admin:
                            # Apply, persist, and broadcast only for admins;
                            # non-admin update frames are silently dropped.
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
                # Free the in-memory doc/room when the last editor leaves;
                # state is already persisted, so it reloads lazily on demand.
                if not rooms[schedule_id]:
                    rooms.pop(schedule_id, None)
                    docs.pop(schedule_id, None)

    # -----------------------------------------------------------------------
    # JSON auth endpoints
    # -----------------------------------------------------------------------

    @app.post("/api/auth/login")
    async def api_login(request: Request):
        ip = _client_ip(request)
        if _login_blocked(ip):
            return JSONResponse(
                {"error": "te veel pogingen, probeer later opnieuw"},
                status_code=429,
            )
        try:
            body = await request.json()
            name = str(body.get("name", "")).strip()
            password = str(body.get("password", ""))
        except Exception:
            return JSONResponse({"error": "invalid request"}, status_code=400)
        if db_conn is None or _is_open():
            # No auth configured (dev/bootstrap) — accept any credentials
            token = _new_session(0)
            resp = JSONResponse({"ok": True, "name": name or "instructor"})
            _set_session_cookie(resp, token)
            return resp
        member_id = await asyncio.to_thread(
            _db.verify_instructor_login, db_conn, name, password)
        if member_id is None:
            _record_login_failure(ip)
            return JSONResponse({"error": "Onjuiste naam of wachtwoord."}, status_code=401)
        login_failures.pop(ip, None)  # successful login clears the counter
        token = _new_session(member_id)
        resp = JSONResponse({"ok": True, "name": name})
        _set_session_cookie(resp, token)
        return resp

    @app.post("/api/auth/logout")
    async def api_logout(request: Request):
        token = request.cookies.get("bsession")
        if token:
            sessions.pop(token, None)
        resp = JSONResponse({"ok": True})
        resp.delete_cookie("bsession", path="/")
        return resp

    @app.get("/api/auth/me")
    async def api_me(request: Request):
        token = request.cookies.get("bsession")
        member_id = _auth_member(token)
        if member_id is None:
            return JSONResponse({"error": "not authenticated"}, status_code=401)
        if member_id == 0 or db_conn is None:
            return {"member_id": 0, "name": "instructor", "is_admin": True}
        row = await asyncio.to_thread(lambda: db_conn.execute(
            "SELECT m.name, i.is_admin"
            " FROM member m JOIN instructor i ON i.member_id = m.id"
            " WHERE m.id=?", (member_id,)
        ).fetchone())
        is_admin = _is_open() or bool(row and row["is_admin"])
        return {"member_id": member_id,
                "name": row["name"] if row else None,
                "is_admin": is_admin}

    # -----------------------------------------------------------------------
    # API: members — list and bulk-seed
    # -----------------------------------------------------------------------

    @app.get("/api/members")
    async def list_members_api():
        if db_conn is None:
            return JSONResponse({"error": "no database"}, status_code=503)
        return await asyncio.to_thread(_db.list_members, db_conn)

    @app.get("/api/members/progression")
    async def members_progression():
        if db_conn is None:
            return JSONResponse({"error": "no database"}, status_code=503)
        def _query(conn):
            instructor_ids = {r[0] for r in conn.execute("SELECT member_id FROM instructor")}
            seasons = [dict(r) for r in conn.execute("SELECT id, year FROM season ORDER BY year")]
            enrollments = conn.execute(
                "SELECT member_id, season_id, level FROM enrollment"
            ).fetchall()
            by_member = {}
            for e in enrollments:
                by_member.setdefault(e["member_id"], {})[e["season_id"]] = e["level"]
            members = [
                dict(r) for r in conn.execute(
                    "SELECT id, name FROM member WHERE active=1 ORDER BY name"
                )
                if r["id"] not in instructor_ids
            ]
            return {
                "seasons": seasons,
                "members": [
                    {"id": m["id"], "name": m["name"],
                     "levels": {str(sid): lv for sid, lv in by_member.get(m["id"], {}).items()}}
                    for m in members
                ],
            }
        return await asyncio.to_thread(_query, db_conn)

    # -----------------------------------------------------------------------
    # Student level editing & export (admin-only, lock-independent)
    #
    # NOTE: enrollments are keyed by (member_id, season_id) — there is no
    # separate enrollment_id column — so the {enrollment_id} path segment is the
    # season id. Levels are always admin-editable regardless of schedule lock.
    # -----------------------------------------------------------------------

    @app.patch("/enrollment/{enrollment_id}/member/{member_id}/level")
    async def set_student_level(enrollment_id: int, member_id: int,
                                request: Request):
        if db_conn is None:
            return JSONResponse({"error": "no database"}, status_code=503)
        if not _current_is_admin(request):
            return JSONResponse(
                {"detail": "Only admins may edit student levels"},
                status_code=403)
        try:
            body = await request.json()
            level = str(body["level"]).strip()
        except Exception as e:
            return JSONResponse({"error": str(e)}, status_code=400)
        if level not in VALID_LEVELS:
            return JSONResponse(
                {"detail": f"level must be one of {list(VALID_LEVELS)}"},
                status_code=400)

        def _update(conn):
            # 404 unless the member is an enrolled, non-instructor student.
            if not _db.is_student(conn, member_id, enrollment_id):
                return False
            return _db.set_enrollment_level(conn, member_id, enrollment_id, level)

        updated = await asyncio.to_thread(_update, db_conn)
        if not updated:
            return JSONResponse(
                {"detail": "student not found in this enrollment"},
                status_code=404)
        return {"ok": True, "level": level}

    @app.get("/enrollment/{enrollment_id}/students/export")
    async def export_students(enrollment_id: int, request: Request):
        if db_conn is None:
            return JSONResponse({"error": "no database"}, status_code=503)
        if not _current_is_admin(request):
            return JSONResponse(
                {"detail": "Only admins may export students"},
                status_code=403)
        students = await asyncio.to_thread(
            _db.list_enrolled_students, db_conn, enrollment_id)
        import csv
        from io import StringIO
        buf = StringIO()
        writer = csv.writer(buf)
        # TODO: add further export columns
        writer.writerow(["name", "level"])
        for name, level in students:
            writer.writerow([name, level])
        return Response(
            content=buf.getvalue(),
            media_type="text/csv",
            headers={"Content-Disposition":
                     f'attachment; filename="students_season_{enrollment_id}.csv"'},
        )

    @app.post("/api/instructors/{member_id}/set-admin")
    async def set_instructor_admin(member_id: int, request: Request):
        if db_conn is None:
            return JSONResponse({"error": "no database"}, status_code=503)
        if not _current_is_admin(request):
            return JSONResponse({"error": "admin required"}, status_code=403)
        try:
            body = await request.json()
            is_admin = bool(body["is_admin"])
        except Exception as e:
            return JSONResponse({"error": str(e)}, status_code=400)
        # Prevent an admin from removing their own admin status (lockout protection).
        token = request.cookies.get("bsession")
        if _auth_member(token) == member_id and not is_admin:
            return JSONResponse({"error": "cannot remove own admin status"}, status_code=400)
        row = db_conn.execute(
            "SELECT 1 FROM instructor WHERE member_id=?", (member_id,)
        ).fetchone()
        if row is None:
            return JSONResponse({"error": "not an instructor"}, status_code=404)
        await asyncio.to_thread(_db.set_admin, db_conn, member_id, is_admin)
        return {"ok": True}

    @app.delete("/api/members/{member_id}")
    async def delete_member_api(member_id: int, request: Request):
        if db_conn is None:
            return JSONResponse({"error": "no database"}, status_code=503)
        if not _current_is_admin(request):
            return JSONResponse({"error": "admin required"}, status_code=403)
        try:
            await asyncio.to_thread(_db.delete_member, db_conn, member_id)
            return {"ok": True}
        except ValueError as e:
            return JSONResponse({"error": str(e)}, status_code=409)
        except Exception as e:
            return _server_error(e)

    @app.put("/api/members/{member_id}")
    async def rename_member_api(member_id: int, request: Request):
        if db_conn is None:
            return JSONResponse({"error": "no database"}, status_code=503)
        if not _current_is_admin(request):
            return JSONResponse({"error": "admin required"}, status_code=403)
        try:
            body = await request.json()
            name = str(body["name"]).strip()
        except Exception as e:
            return JSONResponse({"error": str(e)}, status_code=400)
        if not name:
            return JSONResponse({"error": "name must not be empty"}, status_code=400)
        try:
            await asyncio.to_thread(_db.rename_member, db_conn, member_id, name)
            return {"ok": True}
        except ValueError as e:
            return JSONResponse({"error": str(e)}, status_code=409)
        except Exception as e:
            return _server_error(e)

    @app.post("/api/members/import")
    async def import_members_api(request: Request):
        if db_conn is None:
            return JSONResponse({"error": "no database"}, status_code=503)
        if not _current_is_admin(request):
            return JSONResponse({"error": "admin required"}, status_code=403)
        oversized = _too_large(request)
        if oversized is not None:
            return oversized
        raw = await request.body()
        if len(raw) > max_upload_bytes:
            return JSONResponse({"error": "payload too large"}, status_code=413)
        ctype = request.headers.get("content-type", "")
        if "multipart" in ctype:
            raw = _extract_multipart(raw, ctype)
        names = _parse_names_csv(raw.decode("utf-8", "replace"))
        if not names:
            return JSONResponse({"error": "no names found in upload"}, status_code=400)
        result = await asyncio.to_thread(_db.seed_members, db_conn, names)
        return result

    # -----------------------------------------------------------------------
    # API: health
    # -----------------------------------------------------------------------

    @app.get("/api/health")
    async def api_health():
        from datetime import datetime, timezone
        return {"status": "healthy",
                "timestamp": datetime.now(timezone.utc).isoformat(),
                "version": "1.0"}

    return app


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------

def _env_bool(name: str, default: bool) -> bool:
    val = os.environ.get(name)
    if val is None:
        return default
    return val.strip().lower() in ("1", "true", "yes", "on")


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

    # Production hardening is on by default whenever a database is configured
    # (i.e. a real deployment). It can be relaxed via env vars for local runs.
    production = db_conn is not None
    require_auth = _env_bool("BOATSCHED_REQUIRE_AUTH", production)
    secure_cookies = _env_bool("BOATSCHED_SECURE_COOKIES", production)
    min_password_len = int(os.environ.get("BOATSCHED_MIN_PASSWORD_LEN",
                                          "8" if production else "1"))
    login_max_failures = int(os.environ.get("BOATSCHED_LOGIN_MAX_FAIL", "10"))

    # Seed/refresh the bootstrap admin from env so there is never an anonymous
    # access window. Migrations have already run inside create_app() below, so
    # do the seeding after the app is built.
    app = create_app(
        cfg, db_conn,
        require_auth=require_auth,
        secure_cookies=secure_cookies,
        min_password_len=min_password_len,
        login_max_failures=login_max_failures,
    )

    if db_conn is not None:
        admin_name = os.environ.get("BOATSCHED_ADMIN")
        admin_pw = os.environ.get("BOATSCHED_ADMIN_PW")
        if admin_name and admin_pw:
            try:
                seeded = _db.ensure_admin(db_conn, admin_name, admin_pw)
                print(f"  bootstrap admin '{admin_name}' "
                      f"{'seeded' if seeded else 'present'}")
            except Exception as exc:  # pragma: no cover - defensive
                print(f"[error] admin bootstrap failed: {exc!r}", file=sys.stderr)
                return 1
        elif require_auth:
            has_login = db_conn.execute(
                "SELECT 1 FROM instructor WHERE password_hash IS NOT NULL LIMIT 1"
            ).fetchone()
            if not has_login:
                print("[warn] require_auth is on but no admin exists and "
                      "BOATSCHED_ADMIN/BOATSCHED_ADMIN_PW are unset — nobody "
                      "can log in. Set them and restart.", file=sys.stderr)

    print(f"boatsched server at http://localhost:{a.port}")
    print(f"  boats={cfg.boats} levels={list(cfg.levels)}")
    print(f"  restricted={list(cfg.restricted)} diverse={list(cfg.diverse)}")
    print(f"  require_auth={require_auth} secure_cookies={secure_cookies} "
          f"min_password_len={min_password_len}")
    print("Ctrl-C to stop.")
    uvicorn.run(app, host=a.host, port=a.port)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
