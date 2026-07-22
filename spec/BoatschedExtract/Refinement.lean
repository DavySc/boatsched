/-
  Refinement proofs (Tier-2): the Aeneas-extracted model of the real
  `rust/crates/core` provably refines the hand-written `BoatschedSpec`, for all
  inputs and panic-free (the extracted functions live in Aeneas's `Result`
  monad, so an `= ok _` result rules out `fail`/`div`, i.e. panics/divergence).

  Theorem IDs (see spec/README.md traceability table):
    * EXT-AUTH — the pure authz predicates refine `BoatschedSpec.Authz`.
    * EXT-VAL  — `validate` refines `BoatschedSpec.validate`.
-/
import BoatschedExtract.Types
import BoatschedExtract.Funs
import BoatschedExtract.FunsExternal
import BoatschedSpec.Validate
import BoatschedSpec.Authz

open Aeneas Aeneas.Std Result

namespace Boatsched.Extract

/-! ### EXT-AUTH — authorization predicates -/

/-- Map the spec's `Except` outcome into the extracted Rust `Result` type. -/
def specToCore : Except Boatsched.SetAdminError Unit →
    core.result.Result Unit boatsched_core.authz.SetAdminError
  | .ok _ => .Ok ()
  | .error .cannotRemoveOwnAdmin => .Err .CannotRemoveOwnAdmin

/-- [EXT-AUTH-1] `can_perform` computes exactly `open || admin`, never panics. -/
theorem can_perform_eq (o a : Bool) :
    boatsched_core.authz.can_perform o a = ok (o || a) := by
  unfold boatsched_core.authz.can_perform
  cases o <;> simp

/-- [EXT-AUTH-2] `can_perform` refines the spec decision `AuthState.canPerform`
    for the matching state, so AUTH-1/AUTH-2 carry onto the real Rust code. -/
theorem can_perform_refines (s : Boatsched.AuthState) (m : Boatsched.MemberId)
    (act : Boatsched.GatedAction) :
    boatsched_core.authz.can_perform s.openMode (s.admins m)
      = ok (s.canPerform m act) := by
  rw [can_perform_eq]; rfl

/-- [EXT-AUTH-3] `guard_set_admin` refines the spec self-revoke guard
    `guardSetAdmin` (on the underlying integer ids), so AUTH-4/AUTH-5 carry onto
    the real Rust code — and it never panics. -/
theorem guard_set_admin_refines (caller target : I64) (grant : Bool) :
    boatsched_core.authz.guard_set_admin caller target grant
      = ok (specToCore (Boatsched.guardSetAdmin caller.val target.val grant)) := by
  unfold boatsched_core.authz.guard_set_admin Boatsched.guardSetAdmin
  have hbridge : (caller = target) ↔ (caller.val = target.val) := by
    constructor
    · intro h; rw [h]
    · intro h; scalar_tac
  by_cases h : caller = target
  · have hv : caller.val = target.val := hbridge.mp h
    rw [if_pos h, hv]
    cases grant <;> simp [specToCore]
  · have hv : caller.val ≠ target.val := fun hv => h (hbridge.mpr hv)
    rw [if_neg h]
    simp [hv, specToCore]

/-! ### EXT-VAL — roster/config validation

The extracted `validate` (over `Slice Kid` + `ProblemConfig`, in the `Result`
monad) refines `BoatschedSpec.validate`. We prove per-loop specs for
`knows_level`, `all_known`, and `days`, then compose them.

Extracted names live in the `boatsched_core` namespace; the spec is `Boatsched.*`.
`Kid`, `ValidationError`, `validate` exist in both, so those are qualified. -/

open boatsched_core

/-- Extracted `Kid` ↦ spec `Kid`. -/
@[simp] def kidCorr (k : boatsched_core.Kid) : Boatsched.Kid :=
  ⟨k.name, k.level⟩

/-- Extracted `ProblemConfig` ↦ spec `Config` (scalars via `.val`). -/
@[simp] def cfgCorr (cfg : ProblemConfig) : Boatsched.Config :=
  ⟨cfg.boats.val, cfg.levels.val, cfg.restricted.val, cfg.diverse.val, cfg.adj.val⟩

/-- [knows_level loop] returns whether `level` occurs in the not-yet-scanned
    suffix `self.levels[i:]`. -/
@[step]
theorem knows_level_loop_spec (self : ProblemConfig) (level : String) (i : Usize)
    (h : i.val ≤ self.levels.length) :
    ProblemConfig.knows_level_loop self level i ⦃ b =>
      b = (self.levels.val.drop i.val).contains level ⦄ := by
  unfold ProblemConfig.knows_level_loop
  simp only [alloc.string.String.Insts.CoreCmpPartialEqString.eq]
  split
  · rename_i hlt
    step as ⟨ s, hs ⟩
    have hcons : self.levels.val.drop i.val
        = self.levels.val[i.val] :: self.levels.val.drop (i.val + 1) :=
      List.drop_eq_getElem_cons (show i.val < self.levels.val.length by scalar_tac)
    simp only [hs]
    split
    · rename_i heq
      rw [hcons, List.contains_cons]
      simp_all [beq_iff_eq]
      exact Or.inl heq.symm
    · rename_i hne
      step as ⟨ i1 ⟩
      step as ⟨ b, hb ⟩
      have hi1 : i1.val = i.val + 1 := by scalar_tac
      rw [hb, hi1, hcons, List.contains_cons]
      simp_all [beq_iff_eq]
      intro h
      exact absurd h.symm hne
  · rename_i hge
    have : self.levels.val.drop i.val = [] := by
      apply List.drop_eq_nil_of_le; scalar_tac
    simp_all
termination_by self.levels.length - i.val
decreasing_by scalar_decr_tac

/-- [knows_level] returns whether `level` is a configured level — matching
    `BoatschedSpec.knownLevel`. -/
@[step]
theorem knows_level_spec (self : ProblemConfig) (level : String) :
    ProblemConfig.knows_level self level ⦃ b =>
      b = self.levels.val.contains level ⦄ := by
  unfold ProblemConfig.knows_level
  step as ⟨ b, hb ⟩
  simpa using hb

/-- [all_known loop] `true` iff every kid in the not-yet-scanned suffix
    `kids[i:]` has a configured level. -/
@[step]
theorem all_known_loop_spec (self : ProblemConfig) (kids : Slice boatsched_core.Kid)
    (i n : Usize) (hn : n.val = kids.val.length) (h : i.val ≤ kids.val.length) :
    ProblemConfig.all_known_loop self kids i n ⦃ b =>
      b = (kids.val.drop i.val).all (fun k => self.levels.val.contains k.level) ⦄ := by
  unfold ProblemConfig.all_known_loop
  have hcons : ∀ (hlt : i.val < kids.val.length),
      kids.val.drop i.val = kids.val[i.val] :: kids.val.drop (i.val + 1) :=
    fun hlt => List.drop_eq_getElem_cons hlt
  split
  · rename_i hlt
    step as ⟨ k, hk ⟩
    step as ⟨ b0, hb0 ⟩
    have hlt' : i.val < kids.val.length := by scalar_tac
    split
    · rename_i htrue
      step as ⟨ i1 ⟩
      step as ⟨ b, hb ⟩
      have hi1 : i1.val = i.val + 1 := by scalar_tac
      rw [hcons hlt', List.all_cons]
      simp_all
    · rename_i hbfalse
      rw [hcons hlt', List.all_cons]
      simp_all
  · rename_i hge
    have : kids.val.drop i.val = [] := List.drop_eq_nil_of_le (by scalar_tac)
    simp_all
termination_by kids.val.length - i.val
decreasing_by scalar_decr_tac

/-- [all_known] `true` iff every kid's level is configured — matching
    `BoatschedSpec.allKnown` under `cfgCorr`/`kidCorr`. -/
@[step]
theorem all_known_spec (self : ProblemConfig) (kids : Slice boatsched_core.Kid) :
    ProblemConfig.all_known self kids ⦃ b =>
      b = kids.val.all (fun k => self.levels.val.contains k.level) ⦄ := by
  unfold ProblemConfig.all_known
  step as ⟨ b, hb ⟩
  simpa using hb

/-- [days loop 0] appends `restricted[i:]` onto the accumulator `d`, and returns
    `diverse` unchanged as the first component. -/
@[step]
theorem days_loop0_spec (self : ProblemConfig) (d : alloc.vec.Vec String) (i : Usize)
    (h : i.val ≤ self.restricted.val.length)
    (hcap : d.val.length + (self.restricted.val.length - i.val) ≤ Usize.max) :
    ProblemConfig.days_loop0 self d i ⦃ v d' =>
      v = self.diverse ∧ d'.val = d.val ++ self.restricted.val.drop i.val ⦄ := by
  unfold ProblemConfig.days_loop0
  dsimp only
  simp only [alloc.string.String.Insts.CoreCloneClone.clone]
  split
  · rename_i hlt
    step as ⟨ s, hs ⟩
    step as ⟨ d1, hd1 ⟩
    step as ⟨ i2 ⟩
    have hi2 : i2.val = i.val + 1 := by scalar_tac
    have hcap' : d1.val.length + (self.restricted.val.length - i2.val) ≤ Usize.max := by
      rw [hd1]; simp; scalar_tac
    step as ⟨ v, d', hv, hd' ⟩
    have hcons : self.restricted.val.drop i.val
        = self.restricted.val[i.val] :: self.restricted.val.drop (i.val + 1) :=
      List.drop_eq_getElem_cons (by scalar_tac)
    refine ⟨hv, ?_⟩
    rw [hd', hd1, hi2, hs, hcons]
    simp
  · rename_i hge
    have : self.restricted.val.drop i.val = [] := List.drop_eq_nil_of_le (by scalar_tac)
    simp_all
termination_by self.restricted.val.length - i.val
decreasing_by scalar_decr_tac

/-- [days loop 1] appends `v[j:]` onto the accumulator `d`. -/
@[step]
theorem days_loop1_spec (v : alloc.vec.Vec String) (d : alloc.vec.Vec String) (j : Usize)
    (h : j.val ≤ v.val.length)
    (hcap : d.val.length + (v.val.length - j.val) ≤ Usize.max) :
    ProblemConfig.days_loop1 v d j ⦃ d' => d'.val = d.val ++ v.val.drop j.val ⦄ := by
  unfold ProblemConfig.days_loop1
  dsimp only
  simp only [alloc.string.String.Insts.CoreCloneClone.clone]
  split
  · rename_i hlt
    step as ⟨ s, hs ⟩
    step as ⟨ d1, hd1 ⟩
    step as ⟨ j1 ⟩
    have hj1 : j1.val = j.val + 1 := by scalar_tac
    have hcap' : d1.val.length + (v.val.length - j1.val) ≤ Usize.max := by
      rw [hd1]; simp; scalar_tac
    step as ⟨ d', hd' ⟩
    have hcons : v.val.drop j.val = v.val[j.val] :: v.val.drop (j.val + 1) :=
      List.drop_eq_getElem_cons (by scalar_tac)
    rw [hd', hd1, hj1, hs, hcons]
    simp
  · rename_i hge
    have : v.val.drop j.val = [] := List.drop_eq_nil_of_le (by scalar_tac)
    simp_all
termination_by v.val.length - j.val
decreasing_by scalar_decr_tac

/-- [days] the day list is `restricted ++ diverse`, matching
    `BoatschedSpec.Config.days`. -/
@[step]
theorem days_spec (self : ProblemConfig)
    (hcap : self.restricted.val.length + self.diverse.val.length ≤ Usize.max) :
    ProblemConfig.days self ⦃ d =>
      d.val = self.restricted.val ++ self.diverse.val ⦄ := by
  unfold ProblemConfig.days
  step as ⟨ v, d, hv, hd ⟩
  have hdlen : d.val.length = self.restricted.val.length := by rw [hd]; simp
  have hcap1 : d.val.length + (v.val.length - (0#usize).val) ≤ Usize.max := by
    rw [hdlen, hv]; simpa using hcap
  step as ⟨ d', hd' ⟩
  rw [hd', hd, hv]
  simp

/-! ### EXT-VAL — the top-level refinement -/

/-- Extracted `ValidationError` ↦ spec `ValidationError`. -/
def errToSpec : boatsched_core.ValidationError → Boatsched.ValidationError
  | .NoDays => .noDays
  | .BoatsNotPositive => .boatsNotPositive
  | .NotDivisible n b => .notDivisible n.val b.val
  | .UnknownLevels => .unknownLevels

/-- Extracted Rust `Result` ↦ spec `Except`. -/
def coreValToSpec :
    core.result.Result Unit boatsched_core.ValidationError → Except Boatsched.ValidationError Unit
  | .Ok _ => .ok ()
  | .Err e => .error (errToSpec e)

/-- Spec for our `Vec::is_empty` external model, so `step` can use it. -/
@[local step]
theorem is_empty_spec {T : Type} (A : Type) (w : alloc.vec.Vec T) :
    alloc.vec.Vec.is_empty A w ⦃ b => b = decide (w.val.length = 0) ⦄ := by
  simp only [alloc.vec.Vec.is_empty, WP.spec_ok]

/-- [EXT-VAL] The extracted `validate` refines `BoatschedSpec.validate`: it never
    panics and its result maps onto the spec verdict, for every roster/config
    (with two physically-always-true bounds: the day lists fit a `usize` and the
    roster fits an `i64`). This carries VAL-1/2/3 onto the real Rust code. -/
theorem validate_refines (kids : Slice boatsched_core.Kid) (cfg : ProblemConfig)
    (hcap : cfg.restricted.val.length + cfg.diverse.val.length ≤ Usize.max)
    (hlen : kids.val.length ≤ I64.max) :
    boatsched_core.validate kids cfg ⦃ r =>
      coreValToSpec r = Boatsched.validate (kids.val.map kidCorr) (cfgCorr cfg) ⦄ := by
  unfold boatsched_core.validate
  simp only [Boatsched.validate, cfgCorr, Boatsched.Config.days, Boatsched.allKnown,
    Boatsched.knownLevel]
  step as ⟨ v, hv ⟩
  step as ⟨ emp, hemp ⟩
  have hdays : v.val = cfg.restricted.val ++ cfg.diverse.val := hv
  -- Guard 1: no days.
  split
  · rename_i hempT
    have h0 : cfg.restricted.val ++ cfg.diverse.val = [] := by
      have hz : v.val.length = 0 := by rw [hemp] at hempT; simpa using hempT
      rw [hdays] at hz; exact List.length_eq_zero_iff.mp hz
    rw [if_pos h0]; simp [coreValToSpec, errToSpec]
  · rename_i hempF
    have hne : cfg.restricted.val ++ cfg.diverse.val ≠ [] := by
      intro hnil; apply hempF
      rw [hemp, hdays, hnil]; simp
    rw [if_neg hne]
    -- Guard 2: boats.
    split
    · rename_i hble
      rw [if_pos (show cfg.boats.val ≤ 0 by scalar_tac)]
      simp [coreValToSpec, errToSpec]
    · rename_i hbgt
      rw [if_neg (show ¬ cfg.boats.val ≤ 0 by scalar_tac)]
      have hlen' : kids.len.val ≤ IScalar.max IScalarTy.I64 := by
        rw [Slice.len_val]; scalar_tac
      step with UScalar.hcast_inBounds_spec as ⟨ n, hn ⟩
      step as ⟨ i1, hi1 ⟩
      have hnlen : n.val = (kids.val.map kidCorr).length := by
        rw [hn, Slice.len_val, List.length_map]
      have hi1val : i1.val = (kids.val.map kidCorr).length % cfg.boats.val := by
        rw [hi1, hnlen, Int.tmod_eq_emod_of_nonneg (by positivity)]
      -- Guard 3: divisibility.
      split
      · rename_i hdiv
        rw [if_pos (show (kids.val.map kidCorr).length % cfg.boats.val ≠ 0 by
          rw [← hi1val]; simp_all)]
        simp [coreValToSpec, errToSpec, hnlen]
      · rename_i hdivF
        rw [if_neg (show ¬ (kids.val.map kidCorr).length % cfg.boats.val ≠ 0 by
          rw [← hi1val]; simp_all)]
        -- Guard 4: all levels known.
        step as ⟨ b1, hb1 ⟩
        have hall : b1 = ((kids.val.map kidCorr).all
            fun k => cfg.levels.val.contains k.level) := by
          rw [hb1, List.all_map]; rfl
        split
        · rename_i hok
          rw [if_pos (show ((kids.val.map kidCorr).all
            fun k => cfg.levels.val.contains k.level) = true by rw [← hall]; simpa using hok)]
          simp [coreValToSpec]
        · rename_i hnok
          rw [if_neg (show ¬ ((kids.val.map kidCorr).all
            fun k => cfg.levels.val.contains k.level) = true by rw [← hall]; simpa using hnok)]
          simp [coreValToSpec, errToSpec]

end Boatsched.Extract
