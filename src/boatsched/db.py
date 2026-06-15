"""Database connection and schema initialization."""
import hashlib as _hashlib
import json as _json
import secrets as _secrets
import sqlite3
from pathlib import Path


def get_connection(db_path: str) -> sqlite3.Connection:
    conn = sqlite3.connect(db_path, check_same_thread=False)
    conn.row_factory = sqlite3.Row
    conn.execute("PRAGMA journal_mode=WAL")
    conn.execute("PRAGMA foreign_keys=ON")
    return conn


def init_db(db_path: str) -> sqlite3.Connection:
    conn = get_connection(db_path)
    _create_tables(conn)
    return conn


def _create_tables(conn: sqlite3.Connection) -> None:
    conn.executescript("""
        CREATE TABLE IF NOT EXISTS member (
            id      INTEGER PRIMARY KEY,
            name    TEXT NOT NULL,
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
            id         INTEGER PRIMARY KEY,
            season_id  INTEGER NOT NULL REFERENCES season(id),
            created    TIMESTAMP DEFAULT (datetime('now')),
            start_date DATE,
            config     TEXT NOT NULL
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

        CREATE TABLE IF NOT EXISTS enrollment (
            member_id INTEGER NOT NULL REFERENCES member(id),
            season_id INTEGER NOT NULL REFERENCES season(id),
            level     TEXT NOT NULL CHECK (level IN ('k','b','A','B')),
            PRIMARY KEY (member_id, season_id)
        );

        CREATE TABLE IF NOT EXISTS skill (
            id      INTEGER PRIMARY KEY,
            level   TEXT NOT NULL CHECK (level IN ('k','b','A','B')),
            code    TEXT NOT NULL,
            label   TEXT NOT NULL,
            ordinal INTEGER NOT NULL,
            retired BOOLEAN NOT NULL DEFAULT 0,
            UNIQUE (level, code)
        );

        CREATE TABLE IF NOT EXISTS skill_result (
            member_id INTEGER NOT NULL,
            season_id INTEGER NOT NULL,
            skill_id  INTEGER NOT NULL REFERENCES skill(id),
            status    TEXT NOT NULL,
            note      TEXT,
            PRIMARY KEY (member_id, season_id, skill_id),
            FOREIGN KEY (member_id, season_id)
                REFERENCES enrollment(member_id, season_id)
        );

        CREATE UNIQUE INDEX IF NOT EXISTS ix_unique_member_per_boat_assignment
            ON boat_assignment(schedule_id, member_id, day);

        CREATE UNIQUE INDEX IF NOT EXISTS ix_unique_name ON member(name);

        CREATE INDEX IF NOT EXISTS ix_enrollment_season ON enrollment(season_id);

        CREATE VIEW IF NOT EXISTS v_level_card AS
        SELECT e.member_id, e.season_id, se.year AS season_year,
               m.name AS member_name, e.level,
               sk.id AS skill_id, sk.code AS skill_code, sk.label AS skill_label,
               sk.ordinal, sr.status, sr.note
        FROM enrollment e
        JOIN member m  ON m.id  = e.member_id
        JOIN season se ON se.id = e.season_id
        JOIN skill  sk ON sk.level = e.level AND sk.retired = 0
        LEFT JOIN skill_result sr
               ON sr.member_id = e.member_id
              AND sr.season_id = e.season_id
              AND sr.skill_id  = sk.id;
    """)
    conn.executemany(
        "INSERT OR IGNORE INTO boat (boat_id) VALUES (?)",
        [(i,) for i in range(1, 14)],
    )
    conn.commit()


# ---------------------------------------------------------------------------
# Idempotent migrations (called at server startup for existing databases)
# ---------------------------------------------------------------------------

def add_admin_column(conn: sqlite3.Connection) -> None:
    """Add is_admin BOOLEAN to instructor table (idempotent)."""
    try:
        conn.execute(
            "ALTER TABLE instructor ADD COLUMN is_admin BOOLEAN NOT NULL DEFAULT 0"
        )
        conn.commit()
    except sqlite3.OperationalError:
        pass


def auto_grant_first_admin(conn: sqlite3.Connection) -> None:
    """If no admin exists yet, promote all current instructors to admin.

    Handles existing deployments upgrading to role-based access: instructors
    that were created before admin roles existed had unrestricted access, so
    they all become admins.
    """
    has_admin = conn.execute(
        "SELECT 1 FROM instructor WHERE is_admin=1 LIMIT 1"
    ).fetchone()
    if has_admin:
        return
    conn.execute("UPDATE instructor SET is_admin=1")
    conn.commit()


def make_name_index_nocase(conn: sqlite3.Connection) -> None:
    """Rebuild ix_unique_name as a case-insensitive (NOCASE) unique index.

    Member names were matched case-sensitively (SQLite's default BINARY
    collation), so a roster/import entry like 'van Aarle' would not match a
    stored 'Van Aarle' and was silently created as a duplicate. Making the
    unique index NOCASE also makes the INSERT OR IGNORE / ON CONFLICT(name)
    insert paths match case-insensitively.

    Idempotent: a no-op once the index is already NOCASE. Raises RuntimeError
    if the table already contains names that collide only by case, since
    merging them needs human judgement (which row keeps the enrollments and
    boat assignments).
    """
    row = conn.execute(
        "SELECT sql FROM sqlite_master WHERE type='index' AND name='ix_unique_name'"
    ).fetchone()
    if row is not None and row["sql"] and "NOCASE" in row["sql"].upper():
        return

    dups = conn.execute(
        "SELECT group_concat(name, ' | ') AS names"
        " FROM member GROUP BY name COLLATE NOCASE HAVING COUNT(*) > 1"
    ).fetchall()
    if dups:
        groups = "; ".join(r["names"] for r in dups)
        raise RuntimeError(
            "cannot make member names case-insensitive: existing names collide "
            f"only by case and must be merged manually first: {groups}"
        )

    conn.execute("DROP INDEX IF EXISTS ix_unique_name")
    conn.execute(
        "CREATE UNIQUE INDEX ix_unique_name ON member(name COLLATE NOCASE)"
    )
    conn.commit()


def add_yjs_state_column(conn: sqlite3.Connection) -> None:
    """Add yjs_state BLOB to schedule table (idempotent)."""
    try:
        conn.execute("ALTER TABLE schedule ADD COLUMN yjs_state BLOB")
        conn.commit()
    except sqlite3.OperationalError:
        pass


def add_start_date_column(conn: sqlite3.Connection) -> None:
    """Add start_date DATE to schedule table (idempotent)."""
    try:
        conn.execute("ALTER TABLE schedule ADD COLUMN start_date DATE")
        conn.commit()
    except sqlite3.OperationalError:
        pass


def add_enrollment_tables(conn: sqlite3.Connection) -> None:
    """Create enrollment, skill, skill_result, index, and view (idempotent)."""
    conn.executescript("""
        CREATE TABLE IF NOT EXISTS enrollment (
            member_id INTEGER NOT NULL REFERENCES member(id),
            season_id INTEGER NOT NULL REFERENCES season(id),
            level     TEXT NOT NULL CHECK (level IN ('k','b','A','B')),
            PRIMARY KEY (member_id, season_id)
        );

        CREATE TABLE IF NOT EXISTS skill (
            id      INTEGER PRIMARY KEY,
            level   TEXT NOT NULL CHECK (level IN ('k','b','A','B')),
            code    TEXT NOT NULL,
            label   TEXT NOT NULL,
            ordinal INTEGER NOT NULL,
            retired BOOLEAN NOT NULL DEFAULT 0,
            UNIQUE (level, code)
        );

        CREATE TABLE IF NOT EXISTS skill_result (
            member_id INTEGER NOT NULL,
            season_id INTEGER NOT NULL,
            skill_id  INTEGER NOT NULL REFERENCES skill(id),
            status    TEXT NOT NULL,
            note      TEXT,
            PRIMARY KEY (member_id, season_id, skill_id),
            FOREIGN KEY (member_id, season_id)
                REFERENCES enrollment(member_id, season_id)
        );

        CREATE INDEX IF NOT EXISTS ix_enrollment_season ON enrollment(season_id);

        CREATE VIEW IF NOT EXISTS v_level_card AS
        SELECT e.member_id, e.season_id, se.year AS season_year,
               m.name AS member_name, e.level,
               sk.id AS skill_id, sk.code AS skill_code, sk.label AS skill_label,
               sk.ordinal, sr.status, sr.note
        FROM enrollment e
        JOIN member m  ON m.id  = e.member_id
        JOIN season se ON se.id = e.season_id
        JOIN skill  sk ON sk.level = e.level AND sk.retired = 0
        LEFT JOIN skill_result sr
               ON sr.member_id = e.member_id
              AND sr.season_id = e.season_id
              AND sr.skill_id  = sk.id;
    """)
    conn.commit()


def drop_member_level_column(conn: sqlite3.Connection) -> None:
    """Remove member.level — level is now per-season in enrollment. Idempotent."""
    cols = {r["name"] for r in conn.execute("PRAGMA table_info(member)")}
    if "level" not in cols:
        return
    ver = tuple(
        int(x) for x in
        conn.execute("SELECT sqlite_version()").fetchone()[0].split(".")
    )
    if ver >= (3, 35, 0):
        conn.execute("ALTER TABLE member DROP COLUMN level")
        conn.commit()
    else:
        # SQLite < 3.35 cannot drop columns; the unused column is harmless
        # since no code reads it anymore.
        pass


def drop_legacy_camp_tables(conn: sqlite3.Connection) -> None:
    """Drop progress_record and day_allocation (no data to preserve). Idempotent."""
    conn.executescript("""
        DROP TABLE IF EXISTS progress_record;
        DROP TABLE IF EXISTS day_allocation;
    """)
    conn.commit()


# ---------------------------------------------------------------------------
# Member / enrollment helpers
# ---------------------------------------------------------------------------

def add_or_update_members(conn: sqlite3.Connection, members: list,
                          season_id=None) -> None:
    """Upsert members from a roster; if season_id given, also upsert enrollments.

    members: list of (name, level) tuples.
    season_id: if provided, creates/updates an enrollment row for each member
               in that season with the corresponding level.

    Level 'k' entries are treated as new participants and are inserted into
    the member table automatically (INSERT OR IGNORE).  All other levels are
    treated as returning participants who must already exist in the member
    table; a missing name raises RuntimeError so a silently shorter roster
    is never handed to the solver.
    """
    # Auto-create members for k-level (new participants this season).
    conn.executemany(
        "INSERT INTO member (name) VALUES (?) ON CONFLICT(name) DO NOTHING",
        [(name,) for name, level in members if level == "k"],
    )
    conn.commit()

    # Resolve member IDs; non-k members must already be in the table.
    id_map: dict[str, int] = {}
    missing = []
    for name, level in members:
        row = conn.execute(
            "SELECT id FROM member WHERE name = ? COLLATE NOCASE", (name,)
        ).fetchone()
        if row is None:
            missing.append(f"'{name}' (level '{level}')")
        else:
            id_map[name] = row["id"]
    if missing:
        raise RuntimeError(
            "returning members not found in member table: "
            + ", ".join(missing)
            + "; only 'k'-level entries are auto-created"
        )

    if season_id is not None:
        for name, level in members:
            conn.execute(
                "INSERT INTO enrollment (member_id, season_id, level)"
                " VALUES (?, ?, ?)"
                " ON CONFLICT(member_id, season_id) DO UPDATE SET level = excluded.level",
                (id_map[name], season_id, level),
            )
        conn.commit()


def seed_members(conn: sqlite3.Connection, names: list) -> dict:
    """INSERT OR IGNORE a list of names into the member table.

    Unlike add_or_update_members this is a one-shot initialization: every name
    is treated as potentially new — no level semantics, no enrollment.
    Returns {"created": int, "existing": int, "total": int}.
    """
    clean = [n.strip() for n in names if n and str(n).strip()]
    if not clean:
        total = conn.execute("SELECT COUNT(*) FROM member").fetchone()[0]
        return {"created": 0, "existing": 0, "total": total}
    before = conn.execute("SELECT COUNT(*) FROM member").fetchone()[0]
    conn.executemany(
        "INSERT OR IGNORE INTO member (name) VALUES (?)", [(n,) for n in clean]
    )
    conn.commit()
    after = conn.execute("SELECT COUNT(*) FROM member").fetchone()[0]
    created = after - before
    return {"created": created, "existing": len(clean) - created, "total": after}


def list_members(conn: sqlite3.Connection) -> list:
    """Return all active members ordered by name."""
    return [{"id": r["id"], "name": r["name"]}
            for r in conn.execute(
                "SELECT id, name FROM member WHERE active=1 ORDER BY name"
            )]


def delete_member(conn: sqlite3.Connection, member_id: int) -> None:
    """Soft-delete a member (sets active=0). Raises ValueError if they are an instructor."""
    if conn.execute(
        "SELECT 1 FROM instructor WHERE member_id=?", (member_id,)
    ).fetchone():
        raise ValueError("lid is een instructeur — verwijder eerst de instructeursrol")
    conn.execute("UPDATE member SET active=0 WHERE id=?", (member_id,))
    conn.commit()


def rename_member(conn: sqlite3.Connection, member_id: int, name: str) -> None:
    """Rename a member. Raises ValueError on empty name or name collision."""
    name = name.strip()
    if not name:
        raise ValueError("name must not be empty")
    try:
        conn.execute("UPDATE member SET name=? WHERE id=?", (name, member_id))
        conn.commit()
    except sqlite3.IntegrityError:
        raise ValueError(f"naam '{name}' bestaat al")


def add_enrollment(conn: sqlite3.Connection, member_id: int,
                   season_id: int, level: str) -> None:
    """Upsert a single enrollment row."""
    conn.execute(
        "INSERT INTO enrollment (member_id, season_id, level) VALUES (?, ?, ?)"
        " ON CONFLICT(member_id, season_id) DO UPDATE SET level = excluded.level",
        (member_id, season_id, level),
    )
    conn.commit()


def get_season_roster(conn: sqlite3.Connection, season_id: int) -> list:
    """Return [(name, level)] for active members enrolled in season_id."""
    rows = conn.execute(
        "SELECT m.name, e.level"
        " FROM enrollment e"
        " JOIN member m ON m.id = e.member_id"
        " WHERE e.season_id = ? AND m.active = 1"
        " ORDER BY m.name",
        (season_id,),
    ).fetchall()
    return [(r["name"], r["level"]) for r in rows]


def set_enrollment_level(conn: sqlite3.Connection, member_id: int,
                         season_id: int, level: str) -> bool:
    """Update the level of an existing enrollment row.

    Unlike add_enrollment (which upserts), this only touches a row that already
    exists — it never creates one — so callers can distinguish "no such student
    in this enrollment" (returns False) from a real update (returns True). The
    level is constrained to ('k','b','A','B') by the table's CHECK; an invalid
    value raises sqlite3.IntegrityError and should be validated by the caller.
    """
    cur = conn.execute(
        "UPDATE enrollment SET level = ? WHERE member_id = ? AND season_id = ?",
        (level, member_id, season_id),
    )
    conn.commit()
    return cur.rowcount > 0


def list_enrolled_students(conn: sqlite3.Connection, season_id: int) -> list:
    """Return [(name, level)] for non-instructor members enrolled in season_id.

    "Students" are members without an instructor row. Used by the CSV export.
    """
    rows = conn.execute(
        "SELECT m.name, e.level"
        " FROM enrollment e"
        " JOIN member m ON m.id = e.member_id"
        " WHERE e.season_id = ?"
        "   AND m.id NOT IN (SELECT member_id FROM instructor)"
        " ORDER BY m.name",
        (season_id,),
    ).fetchall()
    return [(r["name"], r["level"]) for r in rows]


def is_student(conn: sqlite3.Connection, member_id: int, season_id: int) -> bool:
    """True when member_id is enrolled in season_id and is not an instructor."""
    row = conn.execute(
        "SELECT 1 FROM enrollment e"
        " WHERE e.member_id = ? AND e.season_id = ?"
        "   AND e.member_id NOT IN (SELECT member_id FROM instructor)",
        (member_id, season_id),
    ).fetchone()
    return row is not None


# ---------------------------------------------------------------------------
# Season / schedule helpers
# ---------------------------------------------------------------------------

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
            "SELECT id FROM member WHERE name = ? COLLATE NOCASE", (row["name"],)
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
        "SELECT id, created, start_date FROM schedule WHERE season_id=? ORDER BY id DESC",
        (season_id,),
    )]


def get_schedule(conn: sqlite3.Connection, schedule_id: int) -> list:
    """Return schedule rows; raises RuntimeError if any member lacks enrollment."""
    rows = conn.execute(
        "SELECT ba.day, ba.boat_id AS boat, m.name, e.level"
        " FROM boat_assignment ba"
        " JOIN member m ON m.id = ba.member_id"
        " JOIN schedule s ON s.id = ba.schedule_id"
        " LEFT JOIN enrollment e"
        "   ON e.member_id = m.id AND e.season_id = s.season_id"
        " WHERE ba.schedule_id = ?"
        " ORDER BY ba.day, ba.boat_id, m.name",
        (schedule_id,),
    ).fetchall()

    missing = [r["name"] for r in rows if r["level"] is None]
    if missing:
        season_row = conn.execute(
            "SELECT season_id FROM schedule WHERE id=?", (schedule_id,)
        ).fetchone()
        season_id = season_row["season_id"] if season_row else "unknown"
        names = ", ".join(sorted(set(missing)))
        raise RuntimeError(
            f"members not enrolled in season {season_id}: {names}"
        )

    return [dict(r) for r in rows]


def swap_assignment(conn: sqlite3.Connection, schedule_id: int,
                    day: str, name_a: str, name_b: str) -> None:
    def _row(name):
        return conn.execute(
            "SELECT ba.id, ba.boat_id FROM boat_assignment ba"
            " JOIN member m ON m.id = ba.member_id"
            " WHERE ba.schedule_id=? AND ba.day=? AND m.name=? COLLATE NOCASE",
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


# ---------------------------------------------------------------------------
# Boat helpers
# ---------------------------------------------------------------------------

def list_boats(conn: sqlite3.Connection) -> list:
    return [dict(r) for r in conn.execute(
        "SELECT boat_id AS id, name FROM boat ORDER BY boat_id"
    )]


def rename_boat(conn: sqlite3.Connection, boat_id: int, name) -> None:
    conn.execute("UPDATE boat SET name=? WHERE boat_id=?", (name or None, boat_id))
    conn.commit()


# ---------------------------------------------------------------------------
# Instructor / auth helpers
# ---------------------------------------------------------------------------

def set_instructor_password(conn: sqlite3.Connection, member_id: int, password: str) -> None:
    salt = _secrets.token_bytes(32)
    dk = _hashlib.scrypt(password.encode("utf-8"), salt=salt, n=2**14, r=8, p=1)
    conn.execute(
        "UPDATE instructor SET password_hash=?, password_salt=? WHERE member_id=?",
        (dk, salt, member_id),
    )
    conn.commit()


def verify_instructor_login(conn: sqlite3.Connection, name: str, password: str):
    row = conn.execute(
        "SELECT i.member_id, i.password_hash, i.password_salt"
        " FROM instructor i JOIN member m ON m.id = i.member_id"
        " WHERE m.name=? COLLATE NOCASE"
        " AND i.password_hash IS NOT NULL AND i.password_salt IS NOT NULL",
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
    return [dict(r) for r in conn.execute(
        "SELECT i.member_id AS id, m.name, i.is_admin,"
        " (i.password_hash IS NOT NULL AND i.password_salt IS NOT NULL) AS has_password"
        " FROM instructor i JOIN member m ON m.id = i.member_id"
        " ORDER BY m.name"
    )]


def set_admin(conn: sqlite3.Connection, member_id: int, is_admin: bool) -> None:
    """Set the is_admin flag for an instructor."""
    conn.execute(
        "UPDATE instructor SET is_admin=? WHERE member_id=?",
        (1 if is_admin else 0, member_id),
    )
    conn.commit()


def ensure_admin(conn: sqlite3.Connection, name: str, password: str) -> bool:
    """Seed a bootstrap admin instructor (idempotent).

    Ensures `name` exists as a member, is an admin instructor, and has a
    password. The password is set only if the instructor does not already
    have one, so a UI-changed password survives restarts. Returns True if a
    password was set (first-time seed), False if the admin already existed.
    """
    name = (name or "").strip()
    if not name or not password:
        raise ValueError("admin name and password are required")
    row = conn.execute(
        "SELECT id FROM member WHERE name=? COLLATE NOCASE", (name,)
    ).fetchone()
    member_id = row["id"] if row else conn.execute(
        "INSERT INTO member (name) VALUES (?)", (name,)
    ).lastrowid
    inst = conn.execute(
        "SELECT password_hash FROM instructor WHERE member_id=?", (member_id,)
    ).fetchone()
    if inst is None:
        conn.execute(
            "INSERT INTO instructor (member_id, is_admin) VALUES (?, 1)",
            (member_id,),
        )
    else:
        conn.execute(
            "UPDATE instructor SET is_admin=1 WHERE member_id=?", (member_id,)
        )
    conn.commit()
    if inst is not None and inst["password_hash"] is not None:
        return False  # keep existing (possibly UI-changed) password
    set_instructor_password(conn, member_id, password)
    return True


# ---------------------------------------------------------------------------
# Yjs / schedule state helpers
# ---------------------------------------------------------------------------

def get_yjs_state(conn: sqlite3.Connection, schedule_id: int):
    row = conn.execute(
        "SELECT yjs_state FROM schedule WHERE id=?", (schedule_id,)
    ).fetchone()
    if row is None or row["yjs_state"] is None:
        return None
    return bytes(row["yjs_state"])


def set_yjs_state(conn: sqlite3.Connection, schedule_id: int, state: bytes) -> None:
    conn.execute("UPDATE schedule SET yjs_state=? WHERE id=?", (state, schedule_id))
    conn.commit()


def get_schedule_start_date(conn: sqlite3.Connection, schedule_id: int):
    row = conn.execute(
        "SELECT start_date FROM schedule WHERE id=?", (schedule_id,)
    ).fetchone()
    return row["start_date"] if row else None


def set_schedule_start_date(
    conn: sqlite3.Connection, schedule_id: int, date_str
) -> None:
    conn.execute(
        "UPDATE schedule SET start_date=? WHERE id=?", (date_str, schedule_id)
    )
    conn.commit()


def replace_assignments(conn: sqlite3.Connection, schedule_id: int,
                        assignments: list) -> None:
    """Replace all boat_assignments for a schedule from a Yjs commit.

    assignments: list of (name, day, boat_id) tuples.
    """
    conn.execute("DELETE FROM boat_assignment WHERE schedule_id=?", (schedule_id,))
    rows = []
    for name, day, boat_id in assignments:
        member = conn.execute(
            "SELECT id FROM member WHERE name=? COLLATE NOCASE", (name,)
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
