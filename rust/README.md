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
| `boatsched-solver` | `solver.py` CP-SAT model via OR-Tools C++ FFI | ✅ |
| (server / cli) | `server.py`, `cli.py` | ⬜ pending |

```bash
cargo test            # whole workspace
cargo test -p boatsched-solver   # solver parity tests (needs the OR-Tools SDK)
```

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

Identical cases run through both implementations:

```bash
# Python (read-only; does not modify the implementation)
../.venv/bin/python bench/py_bench.py ../examples/roster_52.csv

# Rust
cargo run -p boatsched-solver --release --example bench -- ../examples/roster_52.csv
```

Representative run (8 workers, same machine):

| case | mixed | distinct | repeats (py / rs) | status_C | Python | Rust |
|------|-------|----------|-------------------|----------|--------|------|
| `tiny_2k2b`     | 0 | 4  | 0 / 0   | OPTIMAL  | 0.050 s | 0.013 s |
| `adj0_4k4b`     | 0 | 0  | 0 / 0   | OPTIMAL  | 0.009 s | 0.008 s |
| `mid_16_4lvl`   | 0 | 32 | 72 / 72 | OPTIMAL  | 0.211 s | 0.189 s |
| `full_roster52` | 0 | 98 | 50 / 48 | FEASIBLE | 72.6 s  | 72.6 s  |

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
