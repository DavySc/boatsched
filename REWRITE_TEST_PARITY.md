# REWRITE_TEST_PARITY.md

Parity checklist mapping every existing Python test to its planned Rust
equivalent. **This checklist is the definition of done for behavioural parity.**
Each row is unchecked (`[ ]`) until the Rust test exists and passes.

Counts (Python `pytest` items): see the per-file headers below.
Rust home: `crates/*/tests/` (integration) and `#[cfg(test)]` units. Solver
parity tests live in the isolated FFI crate against golden fixtures (Q7).

Legend for "Rust strategy":
- **unit** — `#[cfg(test)]` in the relevant module.
- **http** — axum integration test via `tower::ServiceExt::oneshot` / a test
  client; asserts status + JSON body shape, mirroring `TestClient`.
- **ws** — WebSocket integration test exercising the yrs sync protocol.
- **golden** — compare against captured Python solver output (pending Q1/Q7).

---

## tests/test_boatsched.py (7)
- [ ] `test_roster_from_counts` → unit (roster)
- [ ] `test_parse_bytes_requires_columns` → unit (roster, error)
- [ ] `test_parse_bytes_ok` → unit (roster)
- [ ] `test_validation_divisibility` → unit (solver validate, ValidationError)
- [ ] `test_validation_unknown_level` → unit (solver validate, ValidationError)
- [ ] `test_tiny_solve` → golden/integration (4 kids, 2 boats; 8 rows; per-day placement; stats keys)

## tests/test_solver.py (≈30)
TestValidate (6):
- [ ] `test_no_days_raises` / `test_boats_zero_raises` / `test_boats_negative_raises`
- [ ] `test_indivisible_raises` / `test_unknown_level_raises` / `test_valid_passes_no_exception`
TestConfigDays (4):
- [ ] `test_restricted_before_diverse` / `test_only_restricted` / `test_only_diverse` / `test_empty_both`
TestSolveInvariants (10):
- [ ] row_count / every_kid_placed_each_day / uniform_boat_capacity / boat_numbers_in_range
- [ ] days_in_output_match_config / names_in_output_match_input / levels_in_output_match_input
- [ ] stats_keys_present / stats_are_nonneg_ints / status_C_is_str (∈ OPTIMAL|FEASIBLE)
TestOnProgress (4):
- [ ] called_three_times (labels exactly "A mixed","B diversity","C repeats")
- [ ] values_are_ints / values_are_nonneg / statuses_are_strings
TestSolveConstraints (5):
- [ ] single_level_never_mixed (mixed==0) / adj0_homogeneous_restricted_boats
- [ ] adj1_blocks_nonadjacent_levels / single_day_zero_repeats / repeats_bounded_by_days

## tests/test_roster.py (≈20)
- [ ] TestParseRosterBytes: bom_stripped, case_insensitive_headers, whitespace_stripped,
      missing_name_column, missing_level_column, empty_input, header_row_only_no_data,
      skips_rows_with_blank_name, multiple_kids
- [ ] TestRosterFromCounts: correct_total, level_counts, names_are_unique,
      names_unique_across_levels, empty_returns_empty
- [ ] TestLoadRoster: loads_from_file, bom_file, missing_file_raises (OSError-equiv)

## tests/test_cli.py (≈22)
TestParseArgs (10):
- [ ] roster_and_boats, counts_and_boats, defaults (levels/restricted/diverse/adj/seconds=60/
      stage_seconds=45/workers=8/out=None), custom_levels, out_flag, stage_seconds_flag,
      roster_and_counts_mutually_exclusive (exit), roster_or_counts_required (exit),
      boats_required (exit)
TestMain (≈11):
- [ ] returns_zero_on_success, result_header_in_stdout ("RESULT"), progress_to_stderr
      (A/B/C labels), writes_csv_file (8 rows, header), csv_day_values ({Sat,Fri}),
      bad_roster_returns_2, indivisible_kids_returns_1, uses_roster_file,
      counts_parsing, custom_levels_flag

## tests/test_db.py (≈80)
- [ ] TestInsert (5): single/multiple/row_count/non_k_not_found_raises/empty
- [ ] TestUpdate (3): no_duplicate_on_conflict / unchanged_stays / repeat_idempotent
- [ ] TestCsvRoundTrip (3): all_names / row_count / large_roster
- [ ] TestEnrollment (9): level_stored, updated_on_conflict, no_enrollment_without_season,
      multiple_seasons_independent, add_enrollment_helper, get_season_roster (3 variants)
- [ ] TestSkillResultForeignKey (1): requires_enrollment (FK error)
- [ ] `test_get_schedule_raises_on_missing_enrollment` (RuntimeError "not enrolled")
- [ ] TestSkillRetirement (3): keeps_existing_results, hidden_in_view, replacement_appears
- [ ] TestVLevelCard (3): one_row_per_live_skill_null, filled_shows_status_note, unfilled_null_alongside
- [ ] TestCreateSeason (4): returns_int_id, in_db, distinct_ids, duplicate_raises
- [ ] TestListSeasons (4): empty, returns_created, id_year_keys, newest_first
- [ ] TestSaveSchedule (6): returns_id, linked_to_season, config_stored, assignment_count,
      links_correct_member, unknown_member_raises
- [ ] TestListSchedules (5): empty, returns_saved, id_created_keys, newest_first, isolated_by_season
- [ ] TestGetSchedule (5): all_rows, expected_keys, names_match, levels_from_enrollment, empty_unknown_id
- [ ] TestSwapAssignment (5): exchanges, same_boat_noop, reversible, unknown_raises, count_unchanged
- [ ] TestBoats (7): thirteen_seeded, ids_1_13, no_name_default, rename_sets, rename_none_clears,
      rename_empty_clears, only_target_affected
- [ ] TestMigrationIdempotency (5): add_admin/yjs/start_date idempotent,
      auto_grant skips_when_admin_exists, promotes_all_when_none
- [ ] TestSeedMembers (6): inserts_new, existing_count, total_reflects_table, empty_zero,
      blank_filtered, strips_whitespace
- [ ] TestListMembers (4): active, inactive_excluded, ordered_by_name, id_name_keys
- [ ] TestDeleteMemberDb (2): sets_active_zero, raises_if_instructor
- [ ] TestRenameMemberDb (3): updates, empty_raises, collision_raises
- [ ] TestSetAdminDb (2): promote, demote
- [ ] TestYjsState (3): none_when_not_set, bytes_after_set, none_unknown_id
- [ ] TestScheduleStartDate (3): none_when_not_set, set_get_roundtrip, overwrite
- [ ] TestReplaceAssignments (2): replaces_all_rows, unknown_member_raises

## tests/test_auth.py (≈40)
- [ ] TestOpenMode (7): root_no_login, api_no_login, config_open, login_accepts_any,
      set_password_open, set_password_non_instructor_404
- [ ] TestAuthMode (12): root_returns_viewer, api_401, config_401, valid_login_sets_cookie,
      invalid_pw_401_no_cookie, wrong_name_401, authed_root, authed_api, logout_clears,
      set_password_requires_auth, set_password_authed
- [ ] TestCreateInstructor (7): open, requires_auth, authed, promotes_existing, duplicate_409,
      empty_400, appears_in_list
- [ ] TestInstructorsEndpoint (1): lists_with_password_status (has_password 1/0)
- [ ] TestBootstrapMode (4): root, api, create_first, login_after_first
- [ ] TestApiAuth (4): me_401, me_after_login, login_ok_cookie, logout_clears
- [ ] TestPasswordHashing (4): not_plaintext (len 64 hash, 32 salt), wrong_none,
      correct_returns_mid, two_same_pw_diff_hashes

## tests/test_collab.py (≈12) — ws / yrs
- [ ] TestWebSocketSync (4): server_sends_step1_on_connect, client_step1_gets_step2,
      step2_populates_doc (keys "{day}:{boat}"), correct_member_count (8)
- [ ] TestTwoClientConvergence (3): both_see_same_initial, update_reaches_other (broadcast SYNC_UPDATE),
      update_persisted_to_db
- [ ] TestCommitEndpoint (5): no_db_503, valid_returns_ok, updates_assignments,
      detects_conflict ({name,day,boats}), late_join_sees_current_state
- [ ] (also covers the varint/framing helpers `_parse_message`, `_encode_*`)

## tests/test_level_editing.py (≈14) — http
- [ ] TestBoatAssignmentAuthz (5): admin_unlocked_ok, non_admin_unlocked_403,
      non_admin_locked_403, admin_locked_ok, non_admin_commit_403 (detail strings exact)
- [ ] TestLevelEditing (6): admin_update_ok_unlocked, non_admin_403_unlocked,
      non_admin_403_locked, invalid_level_400, instructor_not_student_404, not_enrolled_404
- [ ] TestStudentExport (2): export_excludes_instructors (header + names set), export_403_non_admin

## tests/test_members.py (≈26) — http
- [ ] TestListMembers (4): active_list, id_name, deleted_not_in_list, no_db_503
- [ ] TestDeleteMember (5): sets_active_zero, disappears, cannot_delete_instructor_409,
      non_admin_403, no_db_503
- [ ] TestRenameMember (6): updates, appears_in_list, duplicate_409, empty_400, non_admin_403, no_db_503
- [ ] TestImportMembers (6): creates_new, existing_counts, total_reflects, empty_400, non_admin_403, no_db_503
- [ ] TestMembersProgression (6): seasons_members_keys, excludes_instructors, includes_non_instructors,
      levels_after_enrollment, season_ids_are_strings, no_db_503
- [ ] TestSetAdmin (6): promote, demote_other, cannot_remove_own_400, not_instructor_404,
      non_admin_403, no_db_503

## tests/test_security.py (≈25) — http + unit
- [ ] TestRequireAuth (5): protected_blocked, anon_login_rejected, seeded_admin_login,
      me_not_anonymous_admin, no_passwordless_member_access
- [ ] TestSecureCookie (2): secure_flag_when_enabled (Secure+HttpOnly), absent_by_default
- [ ] TestSessionTtl (1): session_expires (ttl=1)
- [ ] TestLoginRateLimit (2): blocks_after_max_failures (429 even w/ correct pw),
      successful_login_resets_counter
- [ ] TestUploadSizeCap (3): solve_rejects_oversized_413, import_rejects_oversized_413, normal_ok
- [ ] TestPasswordPolicy (3): create_instructor_short_400, long_ok, set_password_short_400
- [ ] TestSecurityHeaders (3): headers_present, health_public, health_no_sensitive_data
- [ ] TestGenericErrors (1): duplicate_season_500_generic (no "UNIQUE" leak)
- [ ] TestEnsureAdmin (4): creates_with_password, idempotent_keeps_pw, promotes_existing, empty_pw_raises
- [ ] TestMainBootstrap (2): seeds_admin_from_env, warns_when_no_admin_require_auth
- [ ] TestWebSocketSizeCap (1): oversized_frame_dropped (not persisted)

## tests/test_server.py (≈40) — http
- [ ] TestExtractMultipart (3): extracts_file_part, falls_back_no_filename, quoted_boundary
- [ ] TestGetRoot (3): 200, html, index_html_alias
- [ ] TestGetConfig (2): 200, expected_fields (boats/levels/restricted/diverse)
- [ ] TestGetUnknown (1): 404
- [ ] TestPostSolve (4): valid_csv_job_id, bad_csv_400, unknown_path_404, multipart_upload
- [ ] TestGetStatus (6): unknown_404, freshly_submitted_status, completes_with_result,
      result_row_count (8), result_keys, stats_keys
- [ ] TestConfigDbField (2): db_false_without_db, db_true_with_db
- [ ] TestSeasonsNoDb (2): get_empty_list, post_503
- [ ] TestSeasonsApi (6): create_id_year, appears_in_list, newest_first, duplicate_500, bad_body_400
- [ ] TestSolveWithDb (4): schedule_persisted, assignments_persisted (8), config_json_valid,
      done_includes_schedule_id
- [ ] TestSchedulesApi (6): empty_before_solve, appears_after_solve, get_rows (8),
      row_keys, no_db_empty, schedule_no_db_503
- [ ] TestSwapApi (5): returns_ok, actually_exchanges, no_db_503, bad_body_400, unknown_member_500

## tests/test_server_misc.py (≈35) — http + unit
- [ ] TestParseNamesCsv (7): name_column, case_insensitive, fallback_first_token,
      skips_header_fallback, empty, blank_lines, comma_separated_first_column
- [ ] TestApiMe (5): unauth_401, admin_true, non_admin_false, open_mode_true, returns_member_id
- [ ] TestApiLoginEdgeCases (2): bad_body_400, wrong_pw_no_cookie
- [ ] TestApiLogoutEdgeCases (2): logout_without_cookie_ok, logout_clears_cookie
- [ ] TestNoDbEndpoints (6): instructors_empty, create_instructor_503, boats_empty,
      rename_boat_503, schedule_503, set_password_503
- [ ] TestGetScheduleEdgeCases (2): missing_id_400, nonexistent_id_empty_rows
- [ ] TestSetPasswordEdgeCases (4): bad_body_400, empty_400, non_admin_other_403, non_admin_own_ok
- [ ] TestCreateSeasonAdminGuard (2): non_admin_403, admin_ok
- [ ] TestRenameBoat (2): success, bad_body_400
- [ ] TestStartDateAndLocking (5): set_returns_ok, persisted, past_date_no_block_admin_swap,
      past_date_no_block_admin_commit, no_db_503
- [ ] TestSolveAdminGuard (2): non_admin_cannot_solve, admin_can_solve
- [ ] TestApiHealth (2): returns_healthy, has_timestamp
- [ ] TestParseArgs (3): defaults (port 8000/boats 13/db None), custom_values, levels_list
- [ ] TestMain (2): calls_uvicorn_run (port kwarg) → Rust equiv asserts server-build/bind,
      main_without_db returns 0

---

## Tests to ADD where the Python suite is thin (priority #2)

- [ ] **Varint/framing round-trip** unit tests (`write_varint`/`read_varint` for
      multi-byte values, buffer framing) — Python only exercises them indirectly.
- [ ] **NOCASE name matching** explicit tests (e.g. "van Aarle" vs "Van Aarle")
      across add_or_update_members, swap, login — only implicitly covered today.
- [ ] **`make_name_index_nocase` collision RuntimeError** path.
- [ ] **Auth middleware** table-driven test: every public path reachable
      unauthenticated, representative protected path 401.
- [ ] **Job pruning** (`JOB_TTL`/`JOB_MAX` eviction) — untested in Python.
- [ ] **`_too_large` Content-Length=invalid → 400** branch.
- [ ] **Solver golden fixtures** (pending Q1/Q7): objective values + invariants
      for a small matrix of `(counts, boats, adj, days)`.

---

## Coverage baseline (to beat or match)

Python suite: ~330 test items across 12 files; branch coverage configured
(`--cov=boatsched --cov-report=term-missing`). The Rust suite must cover every
row above; target ≥ the Python line/branch coverage on the ported modules. A
`cargo llvm-cov` run will be reported in the final summary alongside this list.
