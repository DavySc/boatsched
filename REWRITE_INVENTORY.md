# REWRITE_INVENTORY.md

Inventory of the existing Python application (`src/boatsched/`) as the parity
baseline for the Rust (`axum` + `sqlx`/`rusqlite` + OR-Tools FFI + `yrs`)
rewrite on branch `rust-rewrite`. Nothing has been translated yet. Open
questions for the maintainer are collected at the end and **must be answered
before translation begins**.

Source commit: branch `rust-rewrite` off `default-open-schedule`
(`40d49f7 Open the most recent schedule by default`).

---

## 1. Components

| Python module | Responsibility | Rust target |
|---|---|---|
| `solver.py` | CP-SAT model + lexicographic staged solve | FFI module to OR-Tools C++ (isolated crate) |
| `roster.py` | CSV / bytes / counts roster ingestion | pure Rust |
| `db.py` | SQLite schema + all helpers | `sqlx`/`rusqlite` access layer |
| `server.py` | FastAPI app, routes, auth, Yjs WS, CLI | `axum` handlers + `yrs` |
| `cli.py` | argparse CLI around `solve()` | `clap` (or hand-rolled) binary |
| `viewer.html` | embedded SPA (served verbatim) | **served byte-for-byte unchanged** |

`viewer.html` is a static embedded asset. It is **not** rewritten; the Rust
binary embeds and serves the identical bytes (`include_str!`/`include_bytes!`).

---

## 2. HTTP / WS API surface

Auth column legend: **public** = no session required; **auth** = valid session
(or open mode); **admin** = admin instructor (or open mode). "Open mode" = no DB,
or no instructor has a password yet, and `require_auth=False`.

### Optimizer / scheduling

| Method | Path | Auth | Notes / response |
|---|---|---|---|
| GET | `/`, `/index.html` | public | `text/html` = viewer.html, 200 |
| GET | `/config` | auth | `{boats, levels[], restricted[], diverse[], db: bool}` |
| GET | `/seasons` | auth | `[]` if no DB; else `[{id, year}]` newest-first |
| POST | `/seasons` | admin | body `{year:int}` → `{id, year}`; 503 no DB; 403 non-admin; 400 bad body; **500** on duplicate year (generic) |
| GET | `/boats` | auth | `[]` if no DB; else `[{id, name}]` ordered by id |
| POST | `/boat` | auth | body `{id:int, name?}` → `{ok:true}`; 503 no DB; 400 bad body; empty/missing name ⇒ stored NULL |
| GET | `/schedules?season_id=` | auth | `[]` if no DB or no season_id; else `[{id, created, start_date}]` newest-first |
| GET | `/schedule?id=` | auth | 503 no DB; 400 missing id; `{rows:[{day,boat,name,level}], start_date}`; unknown id ⇒ `rows:[]` |
| POST | `/solve?season_id=` | **admin (only when DB present)** | upload CSV (raw or multipart) → `{id: jobhex}`; size caps; 400 bad roster / missing returning members; 413 oversize |
| GET | `/status?id=` | auth | 404 unknown; running⇒`{status}`; done⇒`{status,result,stats[,schedule_id]}`; error⇒`{status,error}` |
| POST | `/swap` | **admin** | body `{schedule_id,day,name_a,name_b}` → `{ok:true}`; 503 no DB; 403 non-admin `{"detail":"Only admins may edit boat assignments"}`; 400 bad body; 500 member-not-found |
| POST | `/schedule/{id}/start-date` | auth | body `{start_date?}` → `{ok:true}`; 503 no DB |
| POST | `/schedule/{id}/commit` | **admin** | validates Yjs doc → `{ok:true}` or `{ok:false, conflicts:[{name,day,boats[]}]}`; 503 no DB; 403 non-admin same detail string |
| WS | `/collab/{schedule_id}` | auth | y-websocket sync; non-admin observes only (update frames dropped); closes 1011 no DB, 1008 unauth |

### Instructor / auth

| Method | Path | Auth | Notes |
|---|---|---|---|
| GET | `/instructors` | auth | `[]` if no DB; else `[{id,name,is_admin,has_password}]` |
| POST | `/instructor` | admin | `{name,password}`; first instructor (when no admin exists) auto-admin; 503 no DB; 403; 400 empty; 409 already instructor; weak-password 400 |
| POST | `/set-password` | self/admin | `{name,password}`; 503 no DB; 400 empty/weak; 404 not instructor; in auth mode 401 if no session, 403 if changing another's pw as non-admin |
| POST | `/api/auth/login` | public | `{name,password}`→`{ok,name}`+`bsession` cookie; open/no-DB accepts any creds as member 0; 401 bad; 429 rate-limited |
| POST | `/api/auth/logout` | public | clears session + cookie → `{ok:true}` |
| GET | `/api/auth/me` | auth | 401 if none; member 0/open ⇒`{member_id:0,name:"instructor",is_admin:true}`; else `{member_id,name,is_admin}` |
| POST | `/api/instructors/{member_id}/set-admin` | admin | `{is_admin:bool}`; 503; 403; 400 bad body; **400 cannot remove own admin**; 404 not instructor |
| GET | `/api/health` | **public** | `{status:"healthy",timestamp(ISO-UTC),version:"1.0"}` |

### Members (JSON)

| Method | Path | Auth | Notes |
|---|---|---|---|
| GET | `/api/members` | auth | 503 no DB; else `[{id,name}]` active, ordered by name |
| GET | `/api/members/progression` | auth | 503 no DB; `{seasons:[{id,year}] asc, members:[{id,name,levels:{ "<season_id>":level }}]}`; excludes instructors; season keys are **strings** |
| PUT | `/api/members/{id}` | admin | `{name}`→`{ok}`; 503; 403; 400 empty; 409 collision |
| DELETE | `/api/members/{id}` | admin | `{ok}`; 503; 403; 409 if instructor (Dutch msg incl. "instructeur") |
| POST | `/api/members/import` | admin | CSV upload → `{created,existing,total}`; 503; 403; 413; 400 no names |

### Enrollment / student levels

> `{enrollment_id}` path segment **is the season id** (enrollments keyed by
> `(member_id, season_id)`; no separate id column).

| Method | Path | Auth | Notes |
|---|---|---|---|
| PATCH | `/enrollment/{enrollment_id}/member/{member_id}/level` | admin | `{level}` ∈ `k\|b\|A\|B`; 503; 403 `{"detail":"Only admins may edit student levels"}`; 400 invalid level; 404 not an enrolled non-instructor student; ok⇒`{ok:true,level}` |
| GET | `/enrollment/{enrollment_id}/students/export` | admin | CSV `name,level` of non-instructor enrolled members; 503; 403 `{"detail":"Only admins may export students"}`; `Content-Disposition` filename `students_season_{id}.csv` |

### Known gaps in the original (reproduce as-is — do NOT "fix")

- `viewer.html` calls `GET/POST /api/progress` but **no such route exists**;
  the frontend wraps the call in try/catch so it fails silently. The Rust
  server should also **not** implement `/api/progress` (parity = 404/silent),
  unless the maintainer says otherwise (open question Q5).
- `skill` / `skill_result` / `v_level_card` are modelled in the DB but no API
  reads/writes them except the export and progression queries. Keep the schema;
  do not add endpoints.

### Cross-cutting middleware behaviour (must replicate exactly)

- **Auth middleware**: every path **not** in the public set
  `{"/", "/index.html", "/api/auth/login", "/api/auth/logout", "/api/health"}`
  requires a resolvable session (`_auth_member` ≠ None). Otherwise 401
  `{"error":"not authenticated"}` **before** the handler runs. WS auth is
  handled inside the handler, not the middleware.
- **Security headers** set (via `setdefault`) on every HTTP response:
  `X-Content-Type-Options: nosniff`, `X-Frame-Options: DENY`,
  `Referrer-Policy: no-referrer`.
- **Generic 500**: handler-level `except` returns
  `{"error":"internal server error"}`, status 500, and logs the real error to
  stderr. No internal detail leaks (asserted by `test_security`).
- **Unknown route** ⇒ 404 (framework default).

---

## 3. Authorization model (to encode in the type system)

States and the single source of truth:

- **Open mode** ⇔ `require_auth == false` AND (`db is None` OR no instructor row
  has a non-null `password_hash`). In open mode **everyone is admin** and
  `_auth_member` returns `0` for any/no token.
- **Auth mode** ⇔ otherwise. Session cookie `bsession` → in-memory
  `(member_id, expiry)`. Admin ⇔ `instructor.is_admin = 1` for that member.

Invariants to make unrepresentable where possible (and what each replaces):

1. **Boat-assignment edits are admin-only, always** (`/swap`, `/commit`, and WS
   update frames). The start-date "lock" does **not** gate the server for
   admins; it only drives a read-only UI badge. → encode as a capability token
   (`AdminCtx`) that the swap/commit/WS-apply paths *require*; replaces the
   scattered `if not _current_is_admin(): 403` checks and the removed
   start-date gate.
2. **Student-level edits & export are admin-only, always**, independent of any
   lock. → same `AdminCtx` requirement.
3. **Non-admin may open a `/collab` socket and receive updates, but any update
   frame they send is dropped** (no error). → represent connection as
   `Observer | Editor(AdminCtx)`; only `Editor` can reach the apply/persist/
   broadcast branch.
4. **Cannot remove your own admin** via set-admin (400). → caller identity vs
   target compared before the DB write.
5. **Level value is the closed enum `k|b|A|B`**, never free text (DB CHECK +
   API validation + solver ordering). → Rust `enum Level`.

> Note: in **open mode** the admin checks all pass (everyone admin). The Rust
> `AdminCtx` must therefore be obtainable in open mode for any session,
> mirroring `_member_is_admin` returning true when `_is_open()`.

---

## 4. Database schema

Created by `db._create_tables()` — **do not redesign**. Verbatim DDL is in
`db.py:24-114`. Summary with constraints:

| Table | Columns / constraints |
|---|---|
| `member` | `id PK`, `name TEXT NOT NULL`, `active BOOLEAN DEFAULT 1`, `joined DATE DEFAULT date('now')`. **No `level` column.** |
| `instructor` | `instructor_id PK`, `member_id UNIQUE NOT NULL → member(id)`, `password_hash BLOB`, `password_salt BLOB`, `is_admin BOOLEAN DEFAULT 0` (via migration) |
| `season` | `id PK`, `year INTEGER NOT NULL UNIQUE` |
| `schedule` | `id PK`, `season_id NOT NULL → season(id)`, `created TIMESTAMP DEFAULT datetime('now')`, `start_date DATE`, `config TEXT NOT NULL`, `yjs_state BLOB` (via migration) |
| `boat` | `boat_id PK`, `name TEXT`, `notes TEXT`; seeded `boat_id` 1..13 via `INSERT OR IGNORE` |
| `boat_assignment` | `id PK`, `schedule_id NOT NULL → schedule(id)`, `member_id NOT NULL → member(id)`, `boat_id NOT NULL → boat(boat_id)`, `day TEXT NOT NULL` |
| `enrollment` | `member_id → member(id)`, `season_id → season(id)`, `level TEXT NOT NULL CHECK level IN ('k','b','A','B')`, `PK(member_id, season_id)` |
| `skill` | `id PK`, `level CHECK k\|b\|A\|B`, `code`, `label`, `ordinal`, `retired BOOLEAN DEFAULT 0`, `UNIQUE(level, code)` |
| `skill_result` | `member_id`, `season_id`, `skill_id → skill(id)`, `status NOT NULL`, `note`, `PK(member_id, season_id, skill_id)`, `FK(member_id, season_id) → enrollment(member_id, season_id)` |

Indexes / views:
- `ix_unique_member_per_boat_assignment` UNIQUE on `boat_assignment(schedule_id, member_id, day)`
- `ix_unique_name` UNIQUE on `member(name)` — **rebuilt as `COLLATE NOCASE`** by `make_name_index_nocase`
- `ix_enrollment_season` on `enrollment(season_id)`
- `v_level_card` VIEW: `enrollment ⋈ member ⋈ season ⋈ skill(retired=0) ⟕ skill_result`

PRAGMAs on every connection: `journal_mode=WAL`, `foreign_keys=ON`.

### Migrations (idempotent, all run once in `create_app`, in this order)

`add_yjs_state_column`, `add_start_date_column`, `add_enrollment_tables`,
`drop_member_level_column` (needs SQLite ≥ 3.35 to actually drop; else no-op),
`drop_legacy_camp_tables` (drops `progress_record`, `day_allocation`),
`add_admin_column`, `auto_grant_first_admin` (if no admin exists, promote all
instructors), `make_name_index_nocase` (raises `RuntimeError` if names collide
only by case).

### NOCASE name matching (must preserve)

All name lookups use `WHERE name = ? COLLATE NOCASE`, and the unique index is
NOCASE. So `add_or_update_members`, `save_schedule`, `replace_assignments`,
`swap_assignment`, `verify_instructor_login`, `ensure_admin`, `set-password`,
`create_instructor` all match case-insensitively. **`sqlx`/`rusqlite` must use
the same collation** — do not emulate with `LOWER()` hacks.

### `db.py` helper semantics worth special care (parity-critical)

- `add_or_update_members(conn, members, season_id=None)`: only `level=='k'`
  rows are auto-created (`INSERT … ON CONFLICT(name) DO NOTHING`); every other
  level must already exist by NOCASE name or **RuntimeError** ("returning
  members not found…"). With `season_id`, upserts an enrollment row per member
  (`ON CONFLICT(member_id, season_id) DO UPDATE SET level`).
- `seed_members`: trims, drops blank/None, `INSERT OR IGNORE`, returns
  `{created, existing, total}` where `existing = len(clean) - created`.
- `get_schedule`: LEFT JOINs enrollment; if any scheduled member lacks an
  enrollment in the schedule's season ⇒ **RuntimeError** "members not enrolled
  in season …". Unknown schedule id ⇒ `[]`. Ordered by `day, boat_id, name`.
- `swap_assignment`: both members must be in that schedule+day (else
  RuntimeError "member not found in schedule for that day"); same boat ⇒ no-op.
- `replace_assignments`: deletes all rows for the schedule, re-inserts from
  `(name, day, boat_id)`; unknown name ⇒ rollback + RuntimeError "not found".
- `rename_member`: empty ⇒ ValueError "name must not be empty"; collision ⇒
  ValueError (Dutch "naam '…' bestaat al").
- `delete_member`: instructor ⇒ ValueError (Dutch "lid is een instructeur…");
  else `active=0`.
- `set_instructor_password`: **scrypt** `n=2**14, r=8, p=1`, 32-byte salt,
  64-byte derived key (`hashlib.scrypt`). `verify_instructor_login`:
  constant-time compare; NOCASE name; only rows with non-null hash+salt.
  **The Rust impl must use identical scrypt params and the same salt/hash byte
  layout** so existing DBs keep working (open question Q3).
- `ensure_admin`: idempotent bootstrap; sets password only if none exists,
  returns `True` on first seed else `False`.
- `list_seasons`: `ORDER BY year DESC`. `list_schedules`: `ORDER BY id DESC`.
- `list_members`/progression exclude instructors / inactive as documented.

---

## 5. OR-Tools CP-SAT model (`solver.py`) — exact spec

**Types**: `Kid = (name, level)`, `ScheduleRow = (day, boat:int, name, level)`.

**Config** (defaults): `boats=13`, `levels=("k","b","A","B")` (low→high),
`restricted=("Sat","Sun","Tue","Wed")`, `diverse=("Fri","Mon")`, `adj=1`,
`stage_seconds=20.0`, `seconds=115.0`, `workers=8`.
`days = list(restricted) + list(diverse)` (order matters for variable indices).

> CLI/server defaults differ: `cli.py` and `server.py` use `stage_seconds=45`,
> `seconds=60`. `Config` dataclass defaults are 20/115. Preserve all three.

**`validate(kids, cfg)`** raises `ValidationError` when: no days; `boats<=0`;
`len(kids) % boats != 0`; any kid level ∉ `levels`. Exact messages matched by
tests via regex (`"day"`, `"positive"`, `"unknown"`).

**Decision variables**: `x[k,d,b] ∈ {0,1}` for kid k, day d, boat b
(N·D·n_boats bools). `cap = N // n_boats`.

**Hard constraints**:
- Each kid on each day in exactly one boat: `Σ_b x[k,d,b] == 1`.
- Each boat each day holds exactly `cap`: `Σ_k x[k,d,b] == cap`.
- **Prefixing** (`_prefixing=True`): for each level whose count is divisible by
  `cap`, allocate a contiguous boat-index range; on **restricted** days forbid
  that level's kids from boats outside the range (`x==0`). Purely a presolve
  accelerator — does not change the feasible set of solutions materially
  (comment in source). Boat offsets accumulate across levels in order.
- **Adjacency** (`adj`, hard): on restricted days, for level pair (a,b) with
  `b-a > adj`, `presence[a] + presence[b] <= 1` (can't co-occur in a boat).

**Presence var** `pres[d,b,li]`: `AddMaxEquality(p, members_of_level_li)` or
`p==0` if none.

**Objectives, lexicographic A > B > C** (`staged_solve`):
- **A** `total_mixed` (minimise): per restricted day×boat, `mixed` bool with
  `nlev = Σ_li pres`, `mixed ⇒ nlev≥2`, `¬mixed ⇒ nlev≤1`. Sum over all.
- **B** `total_distinct` (maximise): Σ over diverse day×boat×level of `pres`.
- **C** `total_repeats` (minimise): for each kid pair (k1,k2) **not pruned**
  (pruned ⇔ no diverse days AND `|lvl diff| > adj`): for each day a `meet`
  bool = AND over boats of `x[k1]·x[k2]` (`AddMultiplicationEquality` then
  `AddMaxEquality`); `rep ≥ Σ_d meet − 1`; `total_repeats = Σ rep`.

**Staged solve**: run A (min, `stage_seconds`), add `A <= va`; run B (max,
`stage_seconds`), add `B >= vb`; warm-start C with hints = stage-B values of
**diverse-day** `x` vars only (or all vars if no diverse days), run C (min,
`seconds`). Each `run` sets solver params: `max_time_in_seconds`,
`num_search_workers = workers`, `linearization_level=2`, `symmetry_level=2`,
`cp_model_presolve=True`. A stage failing to reach OPTIMAL/FEASIBLE ⇒
`SolveError`. `on_progress(label, status_name, value)` called per stage with
labels exactly `"A mixed"`, `"B diversity"`, `"C repeats"`.

**Output** (`extract_schedule`): iterate d, b, k in order; emit
`(days[d], b+1, name, level)` where `x[k,d,b]==1`. Stats dict:
`{mixed:int, distinct:int, repeats:int, status_C:str}` (status_C ∈
`OPTIMAL|FEASIBLE`).

> ⚠ **Determinism risk — see open question Q1.** With `workers=8` and
> wall-clock limits, CP-SAT is **not** guaranteed to return the same assignment
> twice, even in pure Python; stage C may end `FEASIBLE` (not proven optimal).
> The existing tests assert only *structural invariants* and that objective
> values are non-negative ints / status ∈ {OPTIMAL, FEASIBLE} — never exact
> rows. This bounds what "identical solver output" can mean.

---

## 6. Yjs / CRDT integration (`server.py`)

- Per-schedule `pycrdt.Doc` (pycrdt 0.13.1 = native `yrs` bindings) cached in
  `docs: {schedule_id: Doc}`; lazily loaded from `schedule.yjs_state` BLOB.
- Doc model: top-level **Map `"schedule"`**, key `"{day}:{boat}"` (e.g.
  `"Sat:3"`), value = **Array of member-name strings**.
- On solve completion (`_run_job`): build doc from result rows grouped by key
  (keys inserted in sorted order), persist `doc.get_update()` to `yjs_state`.
- WS `/collab/{schedule_id}` implements the **y-websocket binary sync protocol**
  with lib0 varint framing (`MSG_SYNC=0`, `MSG_AWARENESS=1`; `SYNC_STEP1=0`,
  `SYNC_STEP2=1`, `SYNC_UPDATE=2`). Helpers `_write_varint`, `_read_varint`,
  `_write_buf`, `_read_buf`, `_encode_sync_step1/step2/update_msg`,
  `_parse_message` — these are exercised directly by `test_collab.py`, so the
  Rust equivalents must produce identical bytes for identical inputs.
- Server flow on connect: send `SYNC_STEP1(doc.get_state())`. On
  `SYNC_STEP1` from client → reply `SYNC_STEP2(doc.get_update(their_sv))`. On
  `SYNC_STEP2`/`SYNC_UPDATE` → **admins only**: `apply_update`, persist
  `doc.get_update()`, broadcast `SYNC_UPDATE` to other room members. On
  `MSG_AWARENESS` → broadcast raw to others. Oversized frames
  (`> max_ws_message_bytes`) dropped silently before parse. Malformed parse ⇒
  ignored. On last disconnect, drop the in-memory doc+room (reloads lazily).
- `/commit`: read the doc Map, build `(name,day)→boat`; duplicate `(name,day)`
  ⇒ collect conflict `{name,day,boats:[first,second]}`; if any ⇒
  `{ok:false, conflicts}` (no DB write); else `replace_assignments`.

> **`yrs` ownership/async note (flagged for review per instructions):** the
> Python version keeps the `Doc` in a plain dict guarded by an `asyncio.Lock`
> only around the rooms set, and calls `apply_update`/`get_update` directly on
> the event loop. In Rust, `yrs::Doc` is `!Send` for some transaction guards;
> the doc cache + per-room broadcast will need a deliberate concurrency design
> (e.g. an actor task per schedule owning the `Doc`, or `Mutex<Doc>` with
> transactions confined to a sync section). I will not pick "whatever compiles"
> — I will write up the chosen ownership model in a comment and flag it.

---

## 7. CLI (`cli.py`) and server CLI (`server.py:main`)

- `boatsched` CLI: mutually-exclusive `--roster` | `--counts LEVEL=N…`;
  required `--boats`; `--levels/--restricted/--diverse/--adj/--seconds/
  --stage-seconds/--workers/--out`. Exit codes: `0` ok, `2` roster/ValueError,
  `1` SolveError/ValidationError. Prints result header + per-day schedule to
  stdout, progress to stderr; `--out` writes CSV `day,boat,name,level`.
- `boatsched-server` CLI: `--port/--host/--boats/--levels/--restricted/
  --diverse/--adj/--stage-seconds(=45)/--seconds(=60)/--workers/--db`.
  `main()` enables production hardening when `--db` is set; env knobs
  `BOATSCHED_REQUIRE_AUTH`, `BOATSCHED_SECURE_COOKIES`,
  `BOATSCHED_MIN_PASSWORD_LEN` (8 prod/1 dev), `BOATSCHED_LOGIN_MAX_FAIL` (10),
  `BOATSCHED_ADMIN`/`BOATSCHED_ADMIN_PW` (bootstrap admin). Prints startup
  banner; warns when require_auth on but no admin/env set.

---

## 8. Security knobs (`create_app` signature — defaults must match)

```
require_auth=False, secure_cookies=False, session_ttl=12*3600,
login_max_failures=20, login_window=300, max_upload_bytes=5*1024*1024,
max_ws_message_bytes=1024*1024, min_password_len=1
```

- Session store: in-memory `token → (member_id, expiry)`, TTL eviction on read.
- Token = `secrets.token_urlsafe(32)`. Cookie `bsession`: `HttpOnly`,
  `SameSite=Lax`, `Secure=secure_cookies`, `Max-Age=session_ttl`, `Path=/`.
- Login throttle: per-IP sliding window (`login_window` s), block at
  `login_max_failures` ⇒ 429; client IP from `X-Real-IP` else socket addr;
  successful login clears the bucket.
- Upload guard: `Content-Length` check (413/400) **and** post-read length check
  (413) against `max_upload_bytes` on `/solve` and `/api/members/import`.
- Job store `JOBS`: TTL `3600`s, cap `200`, pruned under lock on each `/solve`.

---

## 9. What is NOT being changed

- `viewer.html` — embedded and served verbatim.
- DB schema and migration semantics — reproduced exactly.
- Solver model (variables/constraints/objective) — reproduced exactly via FFI.
- All documented status codes, response shapes, and (Dutch) error strings.

---

## 10. OPEN QUESTIONS (answer before translation)

**Q1 — Solver "identical results" definition (blocking).** CP-SAT with
`workers=8` and time limits is nondeterministic: repeated runs of the *Python*
solver can return different (equally optimal, or merely FEASIBLE) assignments.
Bit-identical `rows` parity is therefore not achievable even Python-vs-Python.
What is the parity bar?
  (a) Match objective values + structural invariants (the current tests),
      treating exact assignments as free among optima — **recommended**; or
  (b) Force determinism (`num_search_workers=1`, fixed `random_seed`) in *both*
      a re-run Python baseline and Rust, and compare exact rows — slower, still
      not guaranteed identical across the C++/lib versions; or
  (c) Something else.

**Q2 — OR-Tools FFI build strategy (blocking).** The installed OR-Tools is a
pip wheel: it ships `libortools.so.9` but **no C++ headers** and CP-SAT has **no
stable C API**. To FFI from Rust I must either (i) write a small C++ shim
(`extern "C"`) compiled with `cc`/CMake against OR-Tools headers + abseil +
protobuf — which requires obtaining/building the OR-Tools C++ SDK (large, not
just the wheel), or (ii) build OR-Tools from source. Which do you want, and may
I add the OR-Tools C++ SDK as a build dependency (and document the build)? If
neither is acceptable in this environment, the fallback is shelling out to a
tiny CP-SAT helper — but that keeps Python in the loop, which conflicts with a
"rewrite into Rust." Please confirm direction.

**Q3 — Password hash compatibility.** Should the Rust server stay
**byte-compatible** with existing scrypt hashes (`n=16384, r=8, p=1`, 32-byte
salt, 64-byte key) so current production DBs keep working (recommended), or is a
fresh hashing scheme acceptable (would invalidate existing logins)?

**Q4 — `member 0` / open-mode identity.** In open/no-DB mode the server issues a
session for `member_id = 0` and reports `{member_id:0, name:"instructor",
is_admin:true}`. Confirm I should reproduce this sentinel exactly (the frontend
relies on it).

**Q5 — `/api/progress`.** The frontend calls it but the route doesn't exist
(silently caught). Reproduce the 404/absent behaviour (recommended), or should I
take the opportunity to implement skill-result persistence now? (Out of scope
for parity; asking because the rewrite touches this area.)

**Q6 — DB driver.** `sqlx` (async, compile-time-checked) vs `rusqlite` (sync,
matches the original's thread-offloaded blocking calls). I lean **`rusqlite`**
behind `spawn_blocking` because it mirrors the original's `asyncio.to_thread`
model and SQLite is local — but `sqlx` is also fine. Preference?

**Q7 — Verbatim baseline capture.** For solver parity tests I plan to capture a
fixed set of `(roster, Config)` inputs and the Python solver's outputs/stats
into a fixtures file committed to the branch. OK to run the existing Python
solver to generate these golden fixtures?
