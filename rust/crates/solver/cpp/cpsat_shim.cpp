// C++ CP-SAT shim for boatsched. A faithful, line-by-line port of the model in
// the Python `solver.py` (build_model + staged_solve), exposed over a flat
// `extern "C"` ABI so the Rust `boatsched-solver` crate can drive it.
//
// Parity contract (maintainer decision Q1): the model — variables, hard
// constraints, the three lexicographic objectives A>B>C, the prefixing and
// pair-pruning accelerators, the warm-start hints, and the solver parameters —
// matches the Python original exactly. Exact assignments may differ among
// optima (CP-SAT is nondeterministic with multiple workers); objective values
// and structural invariants are the parity bar.

#include <cstdint>
#include <cstring>
#include <string>
#include <vector>

#include "ortools/sat/cp_model.h"
#include "ortools/sat/cp_model_solver.h"
#include "ortools/sat/sat_parameters.pb.h"
#include "ortools/util/sorted_interval_list.h"

using namespace operations_research;
using namespace operations_research::sat;

namespace {

// Index helper for the x[k][d][b] variable cube.
inline int xi(int k, int d, int b, int D, int B) { return (k * D + d) * B + b; }

}  // namespace

extern "C" {

// Returns:
//   0 = ran (inspect out_status/out_vals per stage; -1 status = stage not run)
//   2 = C++ exception (out_err populated)
//
// out_assign must have capacity n_kids*n_days; out_status/out_vals capacity 3
// (stages A, B, C).
int32_t bs_cpsat_solve(
    int32_t n_kids,
    const int32_t* lvl_of,            // [n_kids] level index per kid
    int32_t n_levels,
    int32_t n_boats,
    int32_t n_days,
    const int32_t* day_restricted,    // [n_days] 1 if restricted
    const int32_t* day_diverse,       // [n_days] 1 if diverse
    int32_t adj,
    double stage_seconds,
    double seconds,
    int32_t workers,
    int32_t prefixing,
    int32_t prune_pairs,
    const int32_t* present,           // [n_kids*n_days] 0 = absent, or null (all present)
    const int32_t* fixed,             // [n_kids*n_days] locked boat index, -1 = free, or null
    const int32_t* hint,              // [n_kids*n_days] previous boat index for warm start, -1 = none, or null
    int32_t* out_assign,              // [n_kids*n_days] boat index (0-based)
    int32_t* out_status,              // [3] CpSolverStatus per stage (-1 = skip)
    int64_t* out_vals,                // [3] objective value per stage
    char* out_err,
    int32_t err_cap) {
  try {
    const int N = n_kids, B = n_boats, D = n_days, L = n_levels;
    const int cap = N / B;
    for (int i = 0; i < 3; ++i) { out_status[i] = -1; out_vals[i] = 0; }

    bool has_diverse = false;
    for (int d = 0; d < D; ++d) has_diverse |= day_diverse[d] != 0;

    CpModelBuilder m;

    // x[k,d,b]
    std::vector<BoolVar> x(static_cast<size_t>(N) * D * B);
    for (int k = 0; k < N; ++k)
      for (int d = 0; d < D; ++d)
        for (int b = 0; b < B; ++b)
          x[xi(k, d, b, D, B)] = m.NewBoolVar();

    auto is_present = [&](int k, int d) { return present == nullptr || present[k * D + d] != 0; };

    // A present kid is in exactly one boat per day; an absent kid in none.
    for (int k = 0; k < N; ++k)
      for (int d = 0; d < D; ++d) {
        std::vector<BoolVar> row;
        row.reserve(B);
        for (int b = 0; b < B; ++b) row.push_back(x[xi(k, d, b, D, B)]);
        m.AddEquality(LinearExpr::Sum(row), is_present(k, d) ? 1 : 0);
      }
    // Boats are balanced per day: with P present kids each boat holds floor(P/B)
    // or ceil(P/B). With nobody absent (N % B == 0) that is exactly cap.
    for (int d = 0; d < D; ++d) {
      int present_d = 0;
      for (int k = 0; k < N; ++k) present_d += is_present(k, d) ? 1 : 0;
      const int lo = present_d / B, hi = lo + (present_d % B ? 1 : 0);
      for (int b = 0; b < B; ++b) {
        std::vector<BoolVar> col;
        col.reserve(N);
        for (int k = 0; k < N; ++k) col.push_back(x[xi(k, d, b, D, B)]);
        m.AddLinearConstraint(LinearExpr::Sum(col), Domain(lo, hi));
      }
    }

    // Locked assignments.
    if (fixed != nullptr)
      for (int k = 0; k < N; ++k)
        for (int d = 0; d < D; ++d) {
          int b = fixed[k * D + d];
          if (b >= 0 && b < B) m.AddEquality(x[xi(k, d, b, D, B)], 1);
        }

    // Warm start from a previous assignment (replaced by stage B's before stage C).
    if (hint != nullptr)
      for (int k = 0; k < N; ++k)
        for (int d = 0; d < D; ++d) {
          int hb = hint[k * D + d];
          if (hb < 0) continue;
          for (int b = 0; b < B; ++b) m.AddHint(x[xi(k, d, b, D, B)], b == hb ? 1 : 0);
        }

    // Prefixing: contiguous boat ranges per level on restricted days.
    if (prefixing) {
      int boat_offset = 0;
      std::vector<int> range_off(L, -1), range_n(L, 0);
      for (int li = 0; li < L; ++li) {
        int n_l = 0;
        for (int k = 0; k < N; ++k) if (lvl_of[k] == li) ++n_l;
        int n_b_for_level = n_l / cap;
        if (n_l % cap == 0) { range_off[li] = boat_offset; range_n[li] = n_b_for_level; }
        boat_offset += n_b_for_level;
      }
      for (int li = 0; li < L; ++li) {
        if (range_off[li] < 0) continue;
        int off = range_off[li], nb = range_n[li];
        for (int k = 0; k < N; ++k) {
          if (lvl_of[k] != li) continue;
          for (int d = 0; d < D; ++d) {
            if (!day_restricted[d]) continue;
            for (int b = 0; b < B; ++b)
              if (!(b >= off && b < off + nb))
                m.AddEquality(x[xi(k, d, b, D, B)], 0);
          }
        }
      }
    }

    // presence(d,b,li)
    auto presence = [&](int d, int b, int li) -> BoolVar {
      BoolVar p = m.NewBoolVar();
      std::vector<LinearExpr> mem;
      for (int k = 0; k < N; ++k)
        if (lvl_of[k] == li) mem.push_back(x[xi(k, d, b, D, B)]);
      if (!mem.empty()) m.AddMaxEquality(p, mem);
      else m.AddEquality(p, 0);
      return p;
    };

    // A: minimize mixed boats on restricted days (+ adjacency hard constraint).
    std::vector<BoolVar> mixed_terms;
    for (int d = 0; d < D; ++d) {
      if (!day_restricted[d]) continue;
      for (int b = 0; b < B; ++b) {
        std::vector<BoolVar> pres(L);
        for (int li = 0; li < L; ++li) pres[li] = presence(d, b, li);
        for (int a = 0; a < L; ++a)
          for (int bb = a + 1; bb < L; ++bb)
            if (bb - a > adj)
              m.AddLessOrEqual(LinearExpr(pres[a]) + LinearExpr(pres[bb]), 1);
        BoolVar mixed = m.NewBoolVar();
        LinearExpr nlev = LinearExpr::Sum(pres);
        m.AddGreaterOrEqual(nlev, 2).OnlyEnforceIf(mixed);
        m.AddLessOrEqual(nlev, 1).OnlyEnforceIf(mixed.Not());
        mixed_terms.push_back(mixed);
      }
    }
    IntVar total_mixed = m.NewIntVar(Domain(0, std::max<int>(1, (int)mixed_terms.size())));
    if (!mixed_terms.empty()) m.AddEquality(total_mixed, LinearExpr::Sum(mixed_terms));
    else m.AddEquality(total_mixed, 0);

    // B: maximize distinct levels on diverse days.
    std::vector<BoolVar> diverse_terms;
    for (int d = 0; d < D; ++d) {
      if (!day_diverse[d]) continue;
      for (int b = 0; b < B; ++b)
        for (int li = 0; li < L; ++li)
          diverse_terms.push_back(presence(d, b, li));
    }
    IntVar total_distinct = m.NewIntVar(Domain(0, std::max<int>(1, (int)diverse_terms.size())));
    if (!diverse_terms.empty()) m.AddEquality(total_distinct, LinearExpr::Sum(diverse_terms));
    else m.AddEquality(total_distinct, 0);

    // C: minimize repeat co-boatings (with pair pruning).
    std::vector<IntVar> pair_rep;
    for (int k1 = 0; k1 < N; ++k1) {
      for (int k2 = k1 + 1; k2 < N; ++k2) {
        if (prune_pairs) {
          int diff = lvl_of[k1] - lvl_of[k2];
          if (diff < 0) diff = -diff;
          bool can_share = has_diverse || diff <= adj;
          if (!can_share) continue;  // repeat is 0; omit variables
        }
        std::vector<BoolVar> meets;
        for (int d = 0; d < D; ++d) {
          std::vector<LinearExpr> sb;
          sb.reserve(B);
          for (int b = 0; b < B; ++b) {
            BoolVar s = m.NewBoolVar();
            m.AddMultiplicationEquality(s, x[xi(k1, d, b, D, B)], x[xi(k2, d, b, D, B)]);
            sb.push_back(s);
          }
          BoolVar md = m.NewBoolVar();
          m.AddMaxEquality(md, sb);
          meets.push_back(md);
        }
        IntVar rep = m.NewIntVar(Domain(0, D));
        m.AddGreaterOrEqual(rep, LinearExpr::Sum(meets) - 1);
        pair_rep.push_back(rep);
      }
    }
    IntVar total_repeats = m.NewIntVar(Domain(0, (int64_t)N * D));
    if (!pair_rep.empty()) m.AddEquality(total_repeats, LinearExpr::Sum(pair_rep));
    else m.AddEquality(total_repeats, 0);

    // ---- staged lexicographic solve A > B > C ----
    auto make_params = [&](double secs) {
      SatParameters p;
      p.set_max_time_in_seconds(secs);
      p.set_num_search_workers(workers);
      p.set_linearization_level(2);
      p.set_symmetry_level(2);
      p.set_cp_model_presolve(true);
      return p;
    };
    auto ok = [](int status) { return status == CpSolverStatus::OPTIMAL ||
                                      status == CpSolverStatus::FEASIBLE; };

    // Stage A (minimize).
    m.Minimize(total_mixed);
    CpSolverResponse rA = SolveWithParameters(m.Build(), make_params(stage_seconds));
    out_status[0] = (int32_t)rA.status();
    if (!ok(out_status[0])) return 0;  // Rust raises SolveError for stage A
    int64_t va = SolutionIntegerValue(rA, total_mixed);
    out_vals[0] = va;
    m.AddLessOrEqual(total_mixed, va);

    // Stage B (maximize).
    m.Maximize(total_distinct);
    CpSolverResponse rB = SolveWithParameters(m.Build(), make_params(stage_seconds));
    out_status[1] = (int32_t)rB.status();
    if (!ok(out_status[1])) return 0;
    int64_t vb = SolutionIntegerValue(rB, total_distinct);
    out_vals[1] = vb;
    m.AddGreaterOrEqual(total_distinct, vb);

    m.ClearHints();
    // Warm-start stage C from stage B's assignment of diverse-day vars (or all
    // vars if there are no diverse days).
    for (int k = 0; k < N; ++k)
      for (int d = 0; d < D; ++d)
        for (int b = 0; b < B; ++b)
          if (has_diverse ? (day_diverse[d] != 0) : true)
            m.AddHint(x[xi(k, d, b, D, B)], SolutionBooleanValue(rB, x[xi(k, d, b, D, B)]));

    // Stage C (minimize).
    m.Minimize(total_repeats);
    CpSolverResponse rC = SolveWithParameters(m.Build(), make_params(seconds));
    out_status[2] = (int32_t)rC.status();
    if (!ok(out_status[2])) return 0;
    int64_t vc = SolutionIntegerValue(rC, total_repeats);
    out_vals[2] = vc;

    // Extract assignment from stage C.
    for (int k = 0; k < N; ++k)
      for (int d = 0; d < D; ++d) {
        int boat = -1;
        for (int b = 0; b < B; ++b)
          if (SolutionBooleanValue(rC, x[xi(k, d, b, D, B)])) { boat = b; break; }
        out_assign[k * D + d] = boat;
      }
    return 0;
  } catch (const std::exception& e) {
    std::snprintf(out_err, err_cap, "%s", e.what());
    return 2;
  } catch (...) {
    std::snprintf(out_err, err_cap, "unknown C++ exception in bs_cpsat_solve");
    return 2;
  }
}

}  // extern "C"
