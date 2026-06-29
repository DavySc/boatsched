# boatsched — Rust rewrite

Cargo workspace porting the Python `boatsched` app to Rust at functional parity.
See `../REWRITE_INVENTORY.md` for the full behavioural inventory and
`../REWRITE_TEST_PARITY.md` for the per-test parity checklist.

## Crates

| crate | mirrors | status |
|-------|---------|--------|
| `boatsched-core` | `solver.py` domain types + `validate` + `Config` | ✅ |
| `boatsched-roster` | `roster.py` | ✅ |
| `boatsched-db` | `db.py` (rusqlite, bundled SQLite) | ✅ |
| `boatsched-auth` | `server.py` authz model as types (capability `AdminCtx`) | ✅ |
| `boatsched-solver` | `solver.py` CP-SAT model via OR-Tools C++ FFI | ✅ |
| `boatsched-server` | `server.py` axum routes + `boatsched-server` binary + yrs collab | ✅ |

```bash
cargo test            # whole workspace
cargo test -p boatsched-solver   # solver parity tests (needs the OR-Tools SDK)
```

## Running locally

> **Prerequisite:** the OR-Tools C++ SDK must be present (see the next section).
> Everything that links the solver — including the server binary and most tests —
> needs it. The build script auto-discovers it under `rust/vendor/`.

All commands run from `rust/`.

```bash
# Run the test suite (whole workspace)
cargo test

# Just the Lean↔Rust conformance bridge (no SDK needed)
cargo test -p boatsched-core --test conformance
```

### Start the server

```bash
# Dev mode — no DB → "open mode": no auth, everyone is treated as admin.
# Good for clicking around the embedded viewer.html SPA.
cargo run -p boatsched-server

# With a database → production hardening switches on automatically
# (require_auth, secure cookies, min_password_len=8, login throttle).
# Seed the bootstrap admin via env vars, or nobody can log in.
BOATSCHED_ADMIN=davy BOATSCHED_ADMIN_PW='a-long-password' \
  cargo run -p boatsched-server -- --db boatsched.db --port 8000

# Real performance (the solver is much faster optimized):
BOATSCHED_ADMIN=davy BOATSCHED_ADMIN_PW='a-long-password' \
  cargo run -p boatsched-server --release -- --db boatsched.db
```

Then open <http://localhost:8000> — it serves the same `viewer.html` SPA as the
Python build. Flags mirror `server.py`: `--port`, `--host`, `--db`, `--boats`,
`--adj`, `--seconds`, `--stage-seconds`, `--workers`, and the space-separated
lists `--levels` / `--restricted` / `--diverse`.

**Notes**
- **First build is slow** — it compiles the C++ shim and links OR-Tools
  (~56 MB lib); later builds are incremental.
- Without `--db` the server is unauthenticated (dev). With `--db` and no admin
  seeded you'll get a warning that nobody can log in — set
  `BOATSCHED_ADMIN` / `BOATSCHED_ADMIN_PW`.
- There is no standalone `boatsched` solver CLI yet; to drive a solve without the
  server, use `cargo run -p boatsched-solver --release --example bench`.

## OR-Tools SDK (required by `boatsched-solver`)

The solver crate compiles a thin C++ shim (`crates/solver/cpp/cpsat_shim.cpp`)
and links it against the **OR-Tools C++ SDK**, which is large (~56 MB packed,
headers + `libortools.so` + bundled abseil/protobuf) and therefore **not
vendored into git** (`rust/vendor/` is gitignored).

**Version is pinned: `9.15.6755`.** Using a different OR-Tools build risks a
protobuf-version mismatch at compile time.

Obtain it one of two ways:

1. **Auto-discovery** — unpack the prebuilt SDK under `rust/vendor/` so the
   directory is named `or-tools_*_cpp_*` and contains
   `include/ortools/sat/cp_model.h`. The build script finds it automatically.

   ```bash
   mkdir -p rust/vendor && cd rust/vendor
   curl -LO https://github.com/google/or-tools/releases/download/v9.15/or-tools_amd64_archlinux_cpp_v9.15.6755.tar.gz
   tar xzf or-tools_amd64_archlinux_cpp_v9.15.6755.tar.gz
   ```

2. **Explicit path** — point `ORTOOLS_ROOT` at any unpacked SDK:

   ```bash
   ORTOOLS_ROOT=/opt/or-tools cargo test -p boatsched-solver
   ```

The build script (`crates/solver/build.rs`) sets an rpath to the SDK's `lib/`
so `libortools.so` and its bundled deps resolve at runtime without
`LD_LIBRARY_PATH`.

## Solver parity & performance

The C++ shim is a faithful port of `solver.py`'s `build_model` + `staged_solve`
(same variables, hard constraints, the three lexicographic objectives A > B > C,
prefixing + pair-pruning accelerators, warm-start hints, and SatParameters).

**Parity bar = objective values + structural invariants.** CP-SAT with
`num_search_workers > 1` is nondeterministic about *which* optimal assignment it
returns, so bit-identical rows are impossible even Python-vs-Python. What *is*
reproducible is the optimum objective *value* and all structural invariants
(capacity, placement, adjacency, etc.) — those are asserted in
`crates/solver/tests/solver_parity.rs`, including two golden tests pinning the
exact OPTIMAL objective triples captured from Python.

### Benchmark

```bash
cargo run -p boatsched-solver --release --example bench -- ../examples/roster_52.csv
```

The table below was captured against the legacy Python implementation during the
rewrite (its `bench/py_bench.py` baseline was removed along with the Python code);
it is kept as the parity record. Representative run (8 workers, same machine):

| case | mixed | distinct | repeats (py / rs) | status_C | Python | Rust    |
|------|-------|----------|-------------------|----------|--------|---------|
| `tiny_2k2b`     | 0 | 4  | 0 / 0            | OPTIMAL  |0.050 s | 0.013 s |
| `adj0_4k4b`     | 0 | 0  | 0 / 0            | OPTIMAL  |0.009 s | 0.008 s |
| `mid_16_4lvl`   | 0 | 32 | 72 / 72          | OPTIMAL  |0.211 s | 0.189 s |
| `full_roster52` | 0 | 98 | 50 / 48          | FEASIBLE |72.6 s  | 72.6 s  |

**Reading the numbers.** Both implementations call the *same* OR-Tools C++
engine, so the heavy solving time is identical by construction. The Rust win is
in everything *around* the solve — no Python interpreter, no `cp_model` Python
object graph to build — which shows up as lower latency on the small/medium
cases that finish before the time budget (e.g. `tiny` ~4× faster). For the full
roster, stage C runs to its wall-clock budget in **both** implementations, so
total time is budget-bound and effectively equal.

### ⚠️ Surfaced divergence (per the rewrite directive)

On `full_roster52`, stage C reaches only **FEASIBLE** within the 60 s budget in
both implementations (it does not prove optimality). The returned incumbent
therefore differs run-to-run: in the captured run Python got `repeats = 50` and
Rust got `repeats = 48`.

This is **not** a modelling divergence — it is the expected nondeterminism of an
unconverged CP-SAT search, and falls squarely within the agreed parity bar:

- Every stage that solves to **OPTIMAL** (A and B here, all stages of the
  smaller cases) produces **identical** objective values.
- The FEASIBLE-only objective differs only because neither side proved
  optimality in the budget; Rust's value happened to be *better* (lower) in this
  run, which is luck of the search, not a correctness difference.

If a future change makes stage C converge to OPTIMAL on the full roster, the
`repeats` value will become reproducible and can be pinned as a golden test.

## Collaborative editing (yrs) performance

`crates/server/src/collab.rs` ports the y-websocket binary sync protocol
(lib0 varint framing reproduced byte-for-byte) and the per-schedule CRDT doc.
Payloads are Yjs **v1** encodings via the `yrs` crate; a captured-pycrdt-update
fixture test (`tests/collab.rs::pycrdt_update_decodes_in_yrs`) pins
pycrdt↔yrs wire compatibility.

### Benchmark

```bash
cargo run -p boatsched-server --release --example crdt_bench
```

The Python (pycrdt) column below was captured during the rewrite (its
`bench/py_crdt_bench.py` baseline was removed with the Python code) and is kept
as the parity record. Realistic schedule (13 boats × 6 days × 4 kids = 312
entries / 78 keys), per-op:

| op | what | Python (pycrdt) | Rust (yrs) | speedup |
|----|------|-----------------|------------|---------|
| `build` | group rows + build doc + encode full update | 714 µs | 210 µs | ~3.4× |
| `apply` | decode + apply update into a fresh doc | 88 µs | 87 µs | ~1× |
| `scan` | apply + read map + conflict-scan (the commit path) | 919 µs | 172 µs | ~5.3× |

**Why `apply` ties:** pycrdt is itself a Rust binding to `yrs`, so the raw
update decode is the *same native code* on both sides. The Rust wins on `build`
and `scan` come from staying in native code where pycrdt pays a Python↔Rust FFI
crossing per map/array element (constructing the doc, and iterating it for the
commit conflict scan).

## Expected UX differences (Python → Rust)

The two builds are behaviourally identical (that's the whole point — see the
parity checklist), so day-to-day the app *looks and acts the same*. Where users
or operators may notice a difference:

- **Solver: no change in the common case.** A real solve runs to its wall-clock
  budget in OR-Tools C++ on both sides, so a full-roster optimise takes the same
  time and yields the same objective quality. Small/preview solves that finish
  before the budget return a little quicker (no Python model-build overhead).
- **Snappier collaborative editing.** Opening a schedule (`build`) and pressing
  *Commit* (the conflict `scan`) are 3–5× cheaper server-side. At this scale
  that's sub-millisecond either way — imperceptible to one user, but it lowers
  tail latency when several instructors edit at once.
- **Real parallelism, no GIL.** The Python server runs solve jobs on threads
  under the GIL and DB work via a threadpool; the Rust server uses Tokio +
  `spawn_blocking` over a single SQLite connection. Concurrent requests during a
  running solve stay responsive without the GIL contention the Python build can
  show under load.
- **Lighter, simpler deployment.** A single statically-ish linked binary
  (`boatsched-server`) plus the OR-Tools shared lib replaces the Python runtime
  + venv + wheels image. Lower idle memory and faster cold start; the Docker
  image is smaller.
- **Same security posture.** Production hardening (require_auth, secure cookies,
  login throttle, upload caps, generic 500s, security headers, admin bootstrap)
  is preserved with identical defaults, so admins configure it exactly as before.

What users will **not** notice: the API, the embedded `viewer.html` SPA, every
URL, request/response shape, status code, the Dutch UI strings, and the
scheduling results are unchanged.
