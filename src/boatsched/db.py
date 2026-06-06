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
