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
        run_job(state2, jid2, kids, cfg, season_id).await;
    });
    Ok(Json(json!({ "id": jid })).into_response())
}

pub(crate) async fn run_job(
    state: SharedState,
    jid: String,
    kids: Vec<boatsched_core::Kid>,
    cfg: Config,
    season_id: Option<String>,
) {
    let cfg_solve = cfg.clone();
    let solved = tokio::task::spawn_blocking(move || solve(&kids, &cfg_solve, None)).await;

    match solved {
        Ok(Ok((rows, stats))) => {
            let mut schedule_id = None;
            if let (Some(_), Some(sid)) = (
                state.db.as_ref(),
                season_id.as_ref().and_then(|s| s.parse::<i64>().ok()),
            ) {
                let cfg_json = serde_json::to_string(&cfg).unwrap_or_else(|_| "{}".into());
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
            "status_C": s.status_c,
        }),
        None => Value::Null,
    }
}
