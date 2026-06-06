"""Database connection and schema initialization."""
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
        assignments.append((schedule_id, member["id"], row["day"], row["boat"]))
    conn.executemany(
        "INSERT INTO boat_assignment (schedule_id, member_id, day, boat)"
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
        "SELECT ba.day, ba.boat, m.name, m.level"
        " FROM boat_assignment ba"
        " JOIN member m ON m.id = ba.member_id"
        " WHERE ba.schedule_id = ?"
        " ORDER BY ba.day, ba.boat, m.name",
        (schedule_id,),
    ).fetchall()
    return [dict(r) for r in rows]


def swap_assignment(conn: sqlite3.Connection, schedule_id: int,
                    day: str, name_a: str, name_b: str) -> None:
    def _row(name):
        return conn.execute(
            "SELECT ba.id, ba.boat FROM boat_assignment ba"
            " JOIN member m ON m.id = ba.member_id"
            " WHERE ba.schedule_id=? AND ba.day=? AND m.name=?",
            (schedule_id, day, name),
        ).fetchone()

    row_a, row_b = _row(name_a), _row(name_b)
    if row_a is None or row_b is None:
        raise RuntimeError("member not found in schedule for that day")
    if row_a["boat"] == row_b["boat"]:
        return
    conn.execute("UPDATE boat_assignment SET boat=? WHERE id=?",
                 (row_b["boat"], row_a["id"]))
    conn.execute("UPDATE boat_assignment SET boat=? WHERE id=?",
                 (row_a["boat"], row_b["id"]))
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
            member_id     INTEGER NOT NULL UNIQUE REFERENCES member(id),
            instructor_id            INTEGER PRIMARY KEY
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

        CREATE TABLE IF NOT EXISTS boat_assignment (
            id          INTEGER PRIMARY KEY,
            schedule_id INTEGER NOT NULL REFERENCES schedule(id),
            member_id   INTEGER NOT NULL REFERENCES member(id),
            day         TEXT NOT NULL,
            boat        INTEGER NOT NULL
        );

        CREATE UNIQUE INDEX IF NOT EXISTS ix_unique_member_per_boat_assignment on boat_assignment(schedule_id,member_id,day);

        CREATE UNIQUE INDEX IF NOT EXISTS ix_unique_name on member(name);
    """)
    conn.commit()
