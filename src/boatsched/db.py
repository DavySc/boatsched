"""Database connection and schema initialization."""
import hashlib as _hashlib
import secrets as _secrets
import sqlite3
from pathlib import Path


def get_connection(db_path: str) -> sqlite3.Connection:
    conn = sqlite3.connect(db_path, check_same_thread=False)
    conn.row_factory = sqlite3.Row      # rows behave like dicts
    conn.execute("PRAGMA journal_mode=WAL")
    conn.execute("PRAGMA foreign_keys=ON")
    return conn


def init_db(db_path: str) -> sqlite3.Connection:
    conn = get_connection(db_path)
    _create_tables(conn)
    return conn


def add_or_update_members(conn: sqlite3.Connection, members: list) -> None:
    """Upsert (name, level) pairs into the member table, then verify each row."""
    conn.executemany(
        "INSERT INTO member (name, level) VALUES (?, ?)"
        " ON CONFLICT(name) DO UPDATE SET level = excluded.level"
        " WHERE name = excluded.name",
        members,
    )
    conn.commit()

    for name, level in members:
        row = conn.execute(
            "SELECT level FROM member WHERE name = ?", (name,)
        ).fetchone()
        if row is None:
            raise RuntimeError(f"member '{name}' not found after upsert")
        if row["level"] != level:
            raise RuntimeError(
                f"member '{name}': expected level '{level}', got '{row['level']}' after upsert"
            )


def create_season(conn: sqlite3.Connection, year: int) -> int:
    cur = conn.execute("INSERT INTO season (year) VALUES (?)", (year,))
    conn.commit()
    return cur.lastrowid


def list_seasons(conn: sqlite3.Connection) -> list:
    return [dict(row) for row in
            conn.execute("SELECT id, year FROM season ORDER BY year DESC")]


def save_schedule(conn: sqlite3.Connection, season_id: int,
                  config_json: str, result: list) -> int:
    cur = conn.execute(
        "INSERT INTO schedule (season_id, config) VALUES (?, ?)",
        (season_id, config_json),
    )
    schedule_id = cur.lastrowid
    assignments = []
    for row in result:
        member = conn.execute(
            "SELECT id FROM member WHERE name = ?", (row["name"],)
        ).fetchone()
        if member is None:
            raise RuntimeError(f"member '{row['name']}' not found in DB")
        assignments.append((schedule_id, member["id"], row["boat"], row["day"]))
    conn.executemany(
        "INSERT INTO boat_assignment (schedule_id, member_id, boat_id, day)"
        " VALUES (?, ?, ?, ?)",
        assignments,
    )
    conn.commit()
    return schedule_id


def list_schedules(conn: sqlite3.Connection, season_id: int) -> list:
    return [dict(r) for r in conn.execute(
        "SELECT id, created FROM schedule WHERE season_id=? ORDER BY id DESC",
        (season_id,),
    )]


def get_schedule(conn: sqlite3.Connection, schedule_id: int) -> list:
    rows = conn.execute(
        "SELECT ba.day, ba.boat_id AS boat, m.name, m.level"
        " FROM boat_assignment ba"
        " JOIN member m ON m.id = ba.member_id"
        " WHERE ba.schedule_id = ?"
        " ORDER BY ba.day, ba.boat_id, m.name",
        (schedule_id,),
    ).fetchall()
    return [dict(r) for r in rows]


def swap_assignment(conn: sqlite3.Connection, schedule_id: int,
                    day: str, name_a: str, name_b: str) -> None:
    def _row(name):
        return conn.execute(
            "SELECT ba.id, ba.boat_id FROM boat_assignment ba"
            " JOIN member m ON m.id = ba.member_id"
            " WHERE ba.schedule_id=? AND ba.day=? AND m.name=?",
            (schedule_id, day, name),
        ).fetchone()

    row_a, row_b = _row(name_a), _row(name_b)
    if row_a is None or row_b is None:
        raise RuntimeError("member not found in schedule for that day")
    if row_a["boat_id"] == row_b["boat_id"]:
        return
    conn.execute("UPDATE boat_assignment SET boat_id=? WHERE id=?",
                 (row_b["boat_id"], row_a["id"]))
    conn.execute("UPDATE boat_assignment SET boat_id=? WHERE id=?",
                 (row_a["boat_id"], row_b["id"]))
    conn.commit()


def list_boats(conn: sqlite3.Connection) -> list:
    return [dict(r) for r in conn.execute(
        "SELECT boat_id AS id, name FROM boat ORDER BY boat_id"
    )]


def rename_boat(conn: sqlite3.Connection, boat_id: int, name) -> None:
    conn.execute("UPDATE boat SET name=? WHERE boat_id=?", (name or None, boat_id))
    conn.commit()


def set_instructor_password(conn: sqlite3.Connection, member_id: int, password: str) -> None:
    """Hash and store a scrypt password for an instructor (binary BLOB columns)."""
    salt = _secrets.token_bytes(32)
    dk = _hashlib.scrypt(password.encode("utf-8"), salt=salt, n=2**14, r=8, p=1)
    conn.execute(
        "UPDATE instructor SET password_hash=?, password_salt=? WHERE member_id=?",
        (dk, salt, member_id),
    )
    conn.commit()


def verify_instructor_login(conn: sqlite3.Connection, name: str, password: str):
    """Return member_id if (name, password) matches an instructor, else None."""
    row = conn.execute(
        "SELECT i.member_id, i.password_hash, i.password_salt"
        " FROM instructor i JOIN member m ON m.id = i.member_id"
        " WHERE m.name=? AND i.password_hash IS NOT NULL AND i.password_salt IS NOT NULL",
        (name,),
    ).fetchone()
    if row is None:
        return None
    salt = bytes(row["password_salt"])
    expected = bytes(row["password_hash"])
    dk = _hashlib.scrypt(password.encode("utf-8"), salt=salt, n=2**14, r=8, p=1)
    if _secrets.compare_digest(dk, expected):
        return row["member_id"]
    return None


def list_instructors(conn: sqlite3.Connection) -> list:
    """List instructors with name and whether they have a password."""
    return [dict(r) for r in conn.execute(
        "SELECT i.member_id AS id, m.name,"
        " (i.password_hash IS NOT NULL AND i.password_salt IS NOT NULL) AS has_password"
        " FROM instructor i JOIN member m ON m.id = i.member_id"
        " ORDER BY m.name"
    )]


def add_yjs_state_column(conn: sqlite3.Connection) -> None:
    """Add yjs_state BLOB to schedule table (idempotent)."""
    try:
        conn.execute("ALTER TABLE schedule ADD COLUMN yjs_state BLOB")
        conn.commit()
    except sqlite3.OperationalError:
        pass  # column already exists


def get_yjs_state(conn: sqlite3.Connection, schedule_id: int):
    """Return persisted Yjs binary state for a schedule, or None."""
    row = conn.execute(
        "SELECT yjs_state FROM schedule WHERE id=?", (schedule_id,)
    ).fetchone()
    if row is None or row["yjs_state"] is None:
        return None
    return bytes(row["yjs_state"])


def set_yjs_state(conn: sqlite3.Connection, schedule_id: int, state: bytes) -> None:
    """Persist Yjs binary state for a schedule."""
    conn.execute("UPDATE schedule SET yjs_state=? WHERE id=?", (state, schedule_id))
    conn.commit()


def replace_assignments(conn: sqlite3.Connection, schedule_id: int,
                        assignments: list) -> None:
    """Replace all boat_assignments for a schedule from Yjs commit.

    assignments: list of (name, day, boat_id) tuples.
    Raises RuntimeError if a member name is not found in the member table.
    """
    conn.execute("DELETE FROM boat_assignment WHERE schedule_id=?", (schedule_id,))
    rows = []
    for name, day, boat_id in assignments:
        member = conn.execute(
            "SELECT id FROM member WHERE name=?", (name,)
        ).fetchone()
        if member is None:
            conn.rollback()
            raise RuntimeError(f"member '{name}' not found")
        rows.append((schedule_id, member["id"], boat_id, day))
    conn.executemany(
        "INSERT INTO boat_assignment (schedule_id, member_id, boat_id, day)"
        " VALUES (?, ?, ?, ?)",
        rows,
    )
    conn.commit()


def _create_tables(conn: sqlite3.Connection) -> None:
    conn.executescript("""
        CREATE TABLE IF NOT EXISTS member (
            id      INTEGER PRIMARY KEY,
            name    TEXT NOT NULL,
            level   TEXT CHECK(level IN ('k','b','A','B')),
            active  BOOLEAN NOT NULL DEFAULT 1,
            joined  DATE NOT NULL DEFAULT (date('now'))
        );

        CREATE TABLE IF NOT EXISTS instructor (
            instructor_id INTEGER PRIMARY KEY,
            member_id     INTEGER NOT NULL UNIQUE REFERENCES member(id),
            password_hash BLOB,
            password_salt BLOB
        );

        CREATE TABLE IF NOT EXISTS season (
            id   INTEGER PRIMARY KEY,
            year INTEGER NOT NULL UNIQUE
        );

        CREATE TABLE IF NOT EXISTS schedule (
            id        INTEGER PRIMARY KEY,
            season_id INTEGER NOT NULL REFERENCES season(id),
            created   TIMESTAMP DEFAULT (datetime('now')),
            config    TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS boat (
            boat_id INTEGER PRIMARY KEY,
            name    TEXT,
            notes   TEXT
        );

        CREATE TABLE IF NOT EXISTS boat_assignment (
            id          INTEGER PRIMARY KEY,
            schedule_id INTEGER NOT NULL REFERENCES schedule(id),
            member_id   INTEGER NOT NULL REFERENCES member(id),
            boat_id     INTEGER NOT NULL REFERENCES boat(boat_id),
            day         TEXT NOT NULL
        );

        CREATE UNIQUE INDEX IF NOT EXISTS ix_unique_member_per_boat_assignment
            ON boat_assignment(schedule_id, member_id, day);

        CREATE UNIQUE INDEX IF NOT EXISTS ix_unique_name ON member(name);
    """)
    conn.executemany(
        "INSERT OR IGNORE INTO boat (boat_id) VALUES (?)",
        [(i,) for i in range(1, 14)],
    )
    conn.commit()
