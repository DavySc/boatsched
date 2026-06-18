# Rust rewrite — final report

Branch: `rust-rewrite` (the `main` branch was never modified, and the Python
implementation under `src/boatsched/` was never modified — both benches import
it read-only).

## 1. Stage / parity checklist status

All six prescribed porting stages are complete and committed, each compiling and
tested before the next:

| Stage | Crate / area | Commit | Status |
|------|---------------|--------|--------|
| inventory | `REWRITE_INVENTORY.md` + `REWRITE_TEST_PARITY.md` | `4b3590b` | ✅ |
| (a) | domain types + DB schema + roster (`core`, `db/schema`, `roster`) | `6c01fa4` | ✅ |
| (b) | SQLite access layer (`db/helpers`) | `e05afad` | ✅ |
| (e) | OR-Tools CP-SAT FFI (`solver` + C++ shim) | `ae03a2c` | ✅ |
| (c) | authorization model as types (`auth`) | `7776f3a` | ✅ |
| (d) | axum HTTP routes + `boatsched-server` binary (`server`) | `532158f` | ✅ |
| (f) | yrs collaborative-editing layer (`server/collab`) | `b639750` | ✅ |

(FFI was done before authz/routes per the maintainer's explicit sequencing choice.)

`REWRITE_TEST_PARITY.md` is checked off per Python test file; the route suites
carry a stage-(d) note and the collab suite a stage-(f) note.

## 2. Test coverage comparison

| Python suite | Py tests | Rust home | Rust tests |
|--------------|---------:|-----------|-----------:|
| test_boatsched | 6 | core/roster/solver | covered |
| test_solver | 29 | core units + `solver/tests/solver_parity.rs` | 12 + 22 |
| test_roster | 17 | `roster` units | 19 |
| test_db | 93 | `db` units + `db/tests/helpers_parity.rs` | 11 + 31 |
| test_auth | 37 | `auth` units + `server/tests/api.rs` | 11 + (api) |
| test_security | 27 | `server/tests/api.rs` (throttle, ttl, cap, policy, headers) | (api) |
| test_members | 33 | `server/tests/api.rs` | (api) |
| test_level_editing | 13 | `server/tests/api.rs` | (api) |
| test_server(+misc) | 89 | `server/tests/api.rs` | (api) |
| test_collab | 12 | `server/{src/collab.rs,tests/collab.rs}` | 8 + 8 |
| test_cli | 19 | — (see gap below) | 0 |
| **Total** | **375** | workspace | **148** |

Rust test count is lower because the tests are coarser-grained — a single Rust
HTTP test typically drives a whole flow and asserts several branches (status +
body key + message) where Python splits those across many `def test_*`. The Rust
set covers **every route and every distinct status/authz branch**, plus areas
the Python suite is thin on (added solver golden objective tests; varint
known-encoding test; the cross-impl pycrdt↔yrs wire-compat test).

Run it all: `cd rust && cargo test` (needs the OR-Tools SDK — see `rust/README.md`).

## 3. Behaviours that could not be made byte-identical (with reasons)

1. **Solver `repeats` on the full roster (FEASIBLE stage C).** CP-SAT with 8
   workers + a wall-clock budget is nondeterministic about *which* optimum it
   returns; on the 52-kid roster stage C only reaches FEASIBLE in the budget in
   **both** implementations, so the incumbent differs run-to-run (captured:
   Python 50, Rust 48). Every stage that solves to OPTIMAL produces **identical**
   objective values. This is the agreed parity bar (objectives + invariants) and
   is documented at length in `rust/README.md`. **This is the one divergence the
   directive said to stop and surface — surfaced.**

2. **Malformed-JSON error *message text*.** For a missing body key the Rust side
   returns the same `"'name'"`-style body Python's `KeyError` produces, but for
   genuinely malformed JSON it returns `serde_json`'s parse message rather than
   Python's. The **status code (400) and the `error` key match**; only the free
   text differs. Noted in `server/src/lib.rs`.

3. **`/collab` rejection mechanism.** Python accepts the socket then calls
   `ws.close(1011/1008)` for no-DB / unauthenticated; the Rust handler denies the
   handshake with HTTP 503/401 instead. The connection is refused either way;
   only the rejection channel differs. Noted in the handler.

4. **Commit doc caching.** Python's commit reads through `_get_doc`, which caches
   the doc in the app-wide `docs` map; the Rust commit reads the live room doc if
   a session is active, else loads the persisted state ephemerally. Observable
   behaviour (committing the current state) is identical; only the in-memory
   caching of a doc by a commit-without-session differs. Noted in the handler.

5. **`/api/health` timestamp format.** Only the key's presence is asserted by the
   Python test; Rust emits a plausible RFC-3339 UTC string via a small local
   civil-date routine (no chrono dependency).

## 4. Places where original behaviour was guessed at — flagged

The ports are otherwise mechanical, but these decisions involved interpretation
rather than a 1:1 reading and deserve review:

- **`AdminCtx` gating granularity.** The directive asked to make illegal states
  unrepresentable. The data-layer DB helpers (e.g. `swap_assignment`) were left
  taking plain args (they're pure data access); the capability is enforced at the
  route boundary — each admin handler must construct an `AdminCtx` before the
  mutation is reachable. This is a judgement call about *where* the type-level
  gate sits; an alternative would thread `&AdminCtx` all the way into the db
  crate. Flagged in `auth/src/lib.rs` and the stage-(c) commit.
- **`json_str` / `json_int` / `json_bool` coercion.** Python relies on
  `str(...)`, `int(...)`, `bool(...)` truthiness of decoded JSON. I reproduced
  the common cases (incl. `True`/`False`/`None` textual forms); exotic inputs
  (e.g. `str()` of a nested object) may format differently. Flagged in code.
- **`pycrdt↔yrs` wire compatibility** is *assumed* from the shared Yjs v1 spec.
  I did not leave it assumed — `pycrdt_update_decodes_in_yrs` pins it with a real
  captured pycrdt update — but the reverse direction (yrs update consumed by a
  pycrdt client) is only exercised yrs-side; a JS/pycrdt client in CI would
  harden it further.
- **DB threading model.** Python shares one `sqlite3` connection across a
  threadpool; I used `Arc<Mutex<Connection>>` + `spawn_blocking` (the agreed
  rusqlite pattern). Semantics match (SQLite serialises writes), but the
  concurrency shape is a deliberate adaptation, not a literal translation.

## 5. Known gap (not yet ported)

- **`cli.py` — the `boatsched` solver CLI** (argparse wrapper around `solve()`,
  ~19 tests in `test_cli.py`). The solver library it wraps is fully ported and
  tested; only the thin CLI front-end binary is missing. It's a small, isolated
  add (a `boatsched` bin mirroring `cli.py`'s args, stdout `RESULT` header,
  stderr A/B/C progress, CSV out). Flagged here rather than silently skipped.

The `/api/progress` route remains intentionally absent (it never existed in the
Python app — reproduced as-is per the inventory's "known gaps").

## 6. Performance summary

- **Solver:** identical when budget-bound (same OR-Tools C++ engine); Rust faster
  on cases that finish before the budget (no interpreter / `cp_model` graph) —
  e.g. tiny case ~4×. Table in `rust/README.md`.
- **CRDT/collab:** build ~3.4×, commit-scan ~5.3× faster than pycrdt; raw update
  apply ties (pycrdt is itself a yrs binding). Table + bench in `rust/README.md`.
- **Server:** single binary, no GIL (real parallelism for concurrent
  requests/solves), lower idle memory and cold start. The full "expected UX
  differences" writeup is in `rust/README.md`.

Benches are reproducible and read-only against the Python implementation:
`rust/bench/py_bench.py`, `rust/bench/py_crdt_bench.py`, and the matching
`cargo run --example bench` / `--example crdt_bench`.
