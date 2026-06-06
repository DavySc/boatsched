"""Local web server: upload a roster, solve in the background, view results.

Stdlib only. Serves an embedded viewer and a small JSON API:
  GET  /            -> the viewer page
  GET  /config      -> {boats, levels, restricted, diverse}
  POST /solve       -> {id}            (accepts multipart or raw CSV body)
  GET  /status?id=  -> {status, [result, stats] | [error]}
"""
from __future__ import annotations
import argparse
import importlib.resources as resources
import json
import sys
import threading
import traceback
import uuid
from dataclasses import asdict
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import urlparse, parse_qs

from .solver import Config, solve
from .roster import parse_roster_bytes, RosterError
from . import db as _db

JOBS: dict = {}
JOBS_LOCK = threading.Lock()


def _viewer_html() -> str:
    return resources.files("boatsched").joinpath("viewer.html").read_text(
        encoding="utf-8")


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
            except Exception as exc:
                print(f"[db] save_schedule failed: {exc}", file=sys.stderr)
        with JOBS_LOCK:
            JOBS[job_id].update(status="done", result=result, stats=stats,
                                schedule_id=schedule_id)
    except BaseException as e:   # catch SystemExit too, so no silent hangs
        with JOBS_LOCK:
            JOBS[job_id].update(status="error",
                                error=str(e) or e.__class__.__name__,
                                trace=traceback.format_exc())


def _extract_multipart(raw: bytes, ctype: str) -> bytes:
    boundary = ctype.split("boundary=")[-1].strip().strip('"').encode()
    sep = b"--" + boundary
    for part in raw.split(sep):
        head = part.split(b"\r\n\r\n", 1)[0]
        if b"filename=" in head:
            body = part.split(b"\r\n\r\n", 1)[1]
            return body.rsplit(b"\r\n", 1)[0]
    return raw


def make_handler(cfg: Config, db_conn=None):
    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *a):
            pass

        def _send(self, code, body, ctype="application/json"):
            if isinstance(body, (dict, list)):
                body = json.dumps(body).encode()
            elif isinstance(body, str):
                body = body.encode()
            self.send_response(code)
            self.send_header("Content-Type", ctype)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def do_GET(self):
            path = urlparse(self.path).path
            if path in ("/", "/index.html"):
                self._send(200, _viewer_html(), "text/html; charset=utf-8")
            elif path == "/config":
                self._send(200, {"boats": cfg.boats,
                                 "levels": list(cfg.levels),
                                 "restricted": list(cfg.restricted),
                                 "diverse": list(cfg.diverse),
                                 "db": db_conn is not None})
            elif path == "/seasons":
                seasons = _db.list_seasons(db_conn) if db_conn is not None else []
                self._send(200, seasons)
            elif path == "/schedules":
                q = parse_qs(urlparse(self.path).query)
                sid = (q.get("season_id") or [None])[0]
                if db_conn is None or sid is None:
                    self._send(200, []); return
                try:
                    self._send(200, _db.list_schedules(db_conn, int(sid)))
                except Exception as e:
                    self._send(500, {"error": str(e)})
            elif path == "/schedule":
                if db_conn is None:
                    self._send(503, {"error": "no database"}); return
                q = parse_qs(urlparse(self.path).query)
                sid = (q.get("id") or [None])[0]
                if sid is None:
                    self._send(400, {"error": "missing id"}); return
                try:
                    self._send(200, {"rows": _db.get_schedule(db_conn, int(sid))})
                except Exception as e:
                    self._send(500, {"error": str(e)})
            elif path == "/status":
                q = parse_qs(urlparse(self.path).query)
                jid = (q.get("id") or [""])[0]
                with JOBS_LOCK:
                    job = JOBS.get(jid)
                if not job:
                    self._send(404, {"error": "no such job"}); return
                out = {"status": job["status"]}
                if job["status"] == "done":
                    out["result"] = job["result"]; out["stats"] = job["stats"]
                    if job.get("schedule_id") is not None:
                        out["schedule_id"] = job["schedule_id"]
                elif job["status"] == "error":
                    out["error"] = job["error"]
                self._send(200, out)
            else:
                self._send(404, {"error": "not found"})

        def do_POST(self):
            path = urlparse(self.path).path
            if path == "/swap":
                if db_conn is None:
                    self._send(503, {"error": "no database"}); return
                length = int(self.headers.get("Content-Length", 0))
                try:
                    body = json.loads(self.rfile.read(length))
                    schedule_id = int(body["schedule_id"])
                    day = str(body["day"])
                    name_a = str(body["name_a"])
                    name_b = str(body["name_b"])
                except (ValueError, KeyError, json.JSONDecodeError) as e:
                    self._send(400, {"error": str(e)}); return
                try:
                    _db.swap_assignment(db_conn, schedule_id, day, name_a, name_b)
                    self._send(200, {"ok": True})
                except Exception as e:
                    self._send(500, {"error": str(e)})
                return
            if path == "/seasons":
                if db_conn is None:
                    self._send(503, {"error": "no database configured"}); return
                length = int(self.headers.get("Content-Length", 0))
                try:
                    body = json.loads(self.rfile.read(length))
                    year = int(body["year"])
                except (ValueError, KeyError, json.JSONDecodeError) as e:
                    self._send(400, {"error": str(e)}); return
                try:
                    sid = _db.create_season(db_conn, year)
                    self._send(200, {"id": sid, "year": year})
                except Exception as e:
                    self._send(500, {"error": str(e)})
                return
            if path != "/solve":
                self._send(404, {"error": "not found"}); return
            q = parse_qs(urlparse(self.path).query)
            season_id = (q.get("season_id") or [None])[0]
            length = int(self.headers.get("Content-Length", 0))
            raw = self.rfile.read(length)
            ctype = self.headers.get("Content-Type", "")
            if ctype.startswith("multipart/form-data"):
                raw = _extract_multipart(raw, ctype)
            try:
                kids = parse_roster_bytes(raw)
            except RosterError as e:
                self._send(400, {"error": str(e)}); return
            if db_conn is not None:
                try:
                    _db.add_or_update_members(db_conn, kids)
                except Exception as e:
                    self._send(500, {"error": f"db error: {e}"}); return
            jid = uuid.uuid4().hex[:12]
            with JOBS_LOCK:
                JOBS[jid] = dict(status="running", result=None,
                                 error=None, stats=None)
            threading.Thread(target=_run_job,
                             args=(jid, kids, cfg, db_conn, season_id),
                             daemon=True).start()
            self._send(200, {"id": jid})

    return Handler


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
    srv = ThreadingHTTPServer((a.host, a.port), make_handler(cfg, db_conn))
    print(f"boatsched server at http://localhost:{a.port}")
    print(f"  boats={cfg.boats} levels={list(cfg.levels)}")
    print(f"  restricted={list(cfg.restricted)} diverse={list(cfg.diverse)}")
    print("Ctrl-C to stop.")
    try:
        srv.serve_forever()
    except KeyboardInterrupt:
        print("\nstopped.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
