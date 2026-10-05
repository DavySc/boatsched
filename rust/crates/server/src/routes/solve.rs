use crate::error::AppError;
use crate::*;
use serde_json::json;

// ---------------------------------------------------------------------------
// Solve / status
// ---------------------------------------------------------------------------

pub(crate) async fn solve_endpoint(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
    body: Bytes,
) -> Handled {
    if state.db.is_some() && !current_is_admin(&state, bsession(&headers)).await {
        return Err(AppError::forbidden("admin required"));
    }
    too_large(&headers, state.opts.max_upload_bytes)?;
    let ctype = ctype_str(&headers);
    let mut raw = body.to_vec();
    if raw.len() > state.opts.max_upload_bytes {
        return Err(AppError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "payload too large",
        ));
    }
    if ctype.starts_with("multipart/form-data") {
        raw = extract_multipart(&raw, &ctype);
    }
    let kids = parse_roster_bytes(&raw).map_err(|RosterError(msg)| AppError::bad_request(msg))?;
    let season_id = q.get("season_id").cloned();

    if state.db.is_some() {
        let sid = season_id.as_ref().and_then(|s| s.parse::<i64>().ok());
        let kids_c = kids.clone();
        let res = with_db(&state, move |conn| {
            db::add_or_update_members(conn.unwrap(), &kids_c, sid)
        })
        .await;
        match res {
            Ok(()) => {}
            Err(db::DbError::Missing(msg)) => return Err(AppError::bad_request(msg)),
            Err(e) => return Err(AppError::internal("solve/add_members", e)),
        }
    }

    let jid = start_job(&state, kids, season_id, Constraints::default());
    Ok(Json(json!({ "id": jid })).into_response())
}

/// Register a job and run the solve in the background; returns the job id.
fn start_job(
    state: &SharedState,
    kids: Vec<boatsched_core::Kid>,
    season_id: Option<String>,
    constraints: Constraints,
) -> String {
    let jid = uuid::Uuid::new_v4().simple().to_string()[..12].to_string();
    {
        let mut jobs = state.jobs.lock().unwrap();
        prune_jobs(&mut jobs);
        jobs.insert(
            jid.clone(),
            Job {
                status: JobStatus::Running,
                result: None,
                stats: None,
                error: None,
                schedule_id: None,
                created: Instant::now(),
            },
        );
    }
    let state2 = state.clone();
    let jid2 = jid.clone();
    let cfg = state.cfg.clone();
    tokio::spawn(async move {
        run_job(state2, jid2, kids, cfg, season_id, constraints).await;
    });
    jid
}

/// The `(absences, locks)` JSON stored beside the solver config so a schedule can
/// be re-solved and displayed with the same inputs.
fn config_json(cfg: &Config, c: &Constraints) -> String {
    let mut v = serde_json::to_value(cfg).unwrap_or_else(|_| json!({}));
    if let Some(obj) = v.as_object_mut() {
        if !c.absences.is_empty() {
            obj.insert(
                "absences".into(),
                json!(c
                    .absences
                    .iter()
                    .map(|(name, day)| json!({ "name": name, "day": day }))
                    .collect::<Vec<_>>()),
            );
        }
        if !c.locked.is_empty() {
            obj.insert(
                "locks".into(),
                json!(c
                    .locked
                    .iter()
                    .map(|r| json!({ "name": r.name, "day": r.day, "boat": r.boat }))
                    .collect::<Vec<_>>()),
            );
        }
    }
    v.to_string()
}

pub(crate) async fn run_job(
    state: SharedState,
    jid: String,
    kids: Vec<boatsched_core::Kid>,
    cfg: Config,
    season_id: Option<String>,
    constraints: Constraints,
) {
    let cfg_solve = cfg.clone();
    let constraints_solve = constraints.clone();
    let solved = tokio::task::spawn_blocking(move || {
        solve_with(&kids, &cfg_solve, &constraints_solve, None)
    })
    .await;

    match solved {
        Ok(Ok((rows, stats))) => {
            let mut schedule_id = None;
            if let (Some(_), Some(sid)) = (
                state.db.as_ref(),
                season_id.as_ref().and_then(|s| s.parse::<i64>().ok()),
            ) {
                let cfg_json = config_json(&cfg, &constraints);
                let rows_c = rows.clone();
                // Initialise the Yjs doc from the result and persist it as
                // yjs_state, exactly like Python's _run_job.
                let yjs = collab::init_doc_update_from_rows(&rows);
                let saved = with_db(&state, move |conn| {
                    let conn = conn.unwrap();
                    let id = db::save_schedule(conn, sid, &cfg_json, &rows_c)?;
                    db::set_yjs_state(conn, id, &yjs)?;
                    Ok::<i64, db::DbError>(id)
                })
                .await;
                match saved {
                    Ok(id) => schedule_id = Some(id),
                    Err(e) => eprintln!("[db] save_schedule failed: {e:?}"),
                }
            }
            let mut jobs = state.jobs.lock().unwrap();
            if let Some(job) = jobs.get_mut(&jid) {
                job.status = JobStatus::Done;
                job.result = Some(rows);
                job.stats = Some(stats);
                job.schedule_id = schedule_id;
            }
        }
        Ok(Err(e)) => set_job_error(&state, &jid, e.to_string()),
        Err(e) => set_job_error(&state, &jid, format!("solver task panicked: {e}")),
    }
}

pub(crate) fn set_job_error(state: &SharedState, jid: &str, msg: String) {
    let mut jobs = state.jobs.lock().unwrap();
    if let Some(job) = jobs.get_mut(jid) {
        job.status = JobStatus::Error;
        job.error = Some(if msg.is_empty() { "error".into() } else { msg });
    }
}

pub(crate) fn prune_jobs(jobs: &mut HashMap<String, Job>) {
    let now = Instant::now();
    jobs.retain(|_, j| {
        matches!(j.status, JobStatus::Running) || now.duration_since(j.created) <= JOB_TTL
    });
    if jobs.len() > JOB_MAX {
        let mut by_age: Vec<(String, Instant)> =
            jobs.iter().map(|(k, v)| (k.clone(), v.created)).collect();
        by_age.sort_by_key(|(_, t)| *t);
        for (k, _) in by_age.into_iter().take(jobs.len() - JOB_MAX) {
            jobs.remove(&k);
        }
    }
}

pub(crate) async fn get_status(
    State(state): State<SharedState>,
    Query(q): Query<HashMap<String, String>>,
) -> Handled {
    let id = q.get("id").cloned().unwrap_or_default();
    let jobs = state.jobs.lock().unwrap();
    let job = match jobs.get(&id) {
        Some(j) => j,
        None => return Err(AppError::not_found("no such job")),
    };
    let mut out = serde_json::Map::new();
    match job.status {
        JobStatus::Running => {
            out.insert("status".into(), json!("running"));
        }
        JobStatus::Done => {
            out.insert("status".into(), json!("done"));
            out.insert("result".into(), serde_json::to_value(&job.result).unwrap());
            out.insert("stats".into(), stats_json(job.stats.as_ref()));
            if let Some(sid) = job.schedule_id {
                out.insert("schedule_id".into(), json!(sid));
            }
        }
        JobStatus::Error => {
            out.insert("status".into(), json!("error"));
            out.insert("error".into(), json!(job.error));
        }
    }
    Ok(Json(Value::Object(out)).into_response())
}

pub(crate) fn stats_json(stats: Option<&boatsched_solver::Stats>) -> Value {
    match stats {
        Some(s) => json!({
            "mixed": s.mixed,
            "distinct": s.distinct,
            "repeats": s.repeats,
            "status_A": s.status_a,
            "status_B": s.status_b,
            "status_C": s.status_c,
        }),
        None => Value::Null,
    }
}

fn cell_list(
    v: &Value,
    key: &str,
    with_boat: bool,
) -> Result<Vec<(String, String, i64)>, AppError> {
    let Some(items) = v.get(key) else {
        return Ok(Vec::new());
    };
    let items = items
        .as_array()
        .ok_or_else(|| AppError::bad_request(format!("'{key}' must be a list")))?;
    items
        .iter()
        .map(|it| {
            let name = it.get("name").and_then(|x| x.as_str());
            let day = it.get("day").and_then(|x| x.as_str());
            let boat = if with_boat {
                it.get("boat").and_then(json_int)
            } else {
                Some(0)
            };
            match (name, day, boat) {
                (Some(n), Some(d), Some(b)) => Ok((n.to_string(), d.to_string(), b)),
                _ => Err(AppError::bad_request(format!("bad entry in '{key}'"))),
            }
        })
        .collect()
}

/// Re-solve a schedule: keep the `locks` (name, day, boat) pinned, leave kids
/// listed in `absences` (name, day) out of every boat that day, and let the
/// solver rearrange the rest. Produces a new schedule in the same season.
pub(crate) async fn resolve_endpoint(
    db: Db,
    Path(schedule_id): Path<i64>,
    headers: HeaderMap,
    body: Bytes,
) -> Handled {
    db.admin(&headers, AppError::forbidden("admin required"))
        .await?;
    let v = parse_json(&body)?;
    let absences = cell_list(&v, "absences", false)?
        .into_iter()
        .map(|(n, d, _)| (n, d))
        .collect();
    let locked = cell_list(&v, "locks", true)?
        .into_iter()
        .map(|(name, day, boat)| ScheduleRow {
            day,
            boat,
            name,
            level: String::new(),
        })
        .collect();

    let loaded = db
        .run(move |conn| {
            let Some(sid) = db::get_schedule_season(conn, schedule_id)? else {
                return Ok::<_, db::DbError>(None);
            };
            let roster = db::get_season_roster(conn, sid)?;
            let prev = db::get_schedule(conn, schedule_id)?;
            Ok(Some((sid, roster, prev)))
        })
        .await;
    let (sid, roster, hint) = match loaded {
        Ok(Some(x)) => x,
        Ok(None) => return Err(AppError::not_found("no such schedule")),
        Err(db::DbError::Missing(m)) => return Err(AppError::bad_request(m)),
        Err(e) => return Err(AppError::internal("resolve", e)),
    };
    let kids = roster
        .into_iter()
        .map(|(name, level)| boatsched_core::Kid { name, level })
        .collect();
    let constraints = Constraints {
        absences,
        locked,
        hint,
    };
    let jid = start_job(db.state(), kids, Some(sid.to_string()), constraints);
    Ok(Json(json!({ "id": jid })).into_response())
}
