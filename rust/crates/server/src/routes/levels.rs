use crate::*;
use serde_json::json;

// ---------------------------------------------------------------------------
// Student levels & export
// ---------------------------------------------------------------------------

pub(crate) async fn set_student_level(
    State(state): State<SharedState>,
    Path((enrollment_id, member_id)): Path<(i64, i64)>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if state.db.is_none() {
        return e_error(StatusCode::SERVICE_UNAVAILABLE, "no database");
    }
    let _admin = match require_admin(&state, bsession(&headers)).await {
        Ok(c) => c,
        Err(_) => return e_detail(StatusCode::FORBIDDEN, "Only admins may edit student levels"),
    };
    let v = match parse_json(&body) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let level = match v.get("level") {
        Some(l) => json_str(l).trim().to_string(),
        None => return e_error(StatusCode::BAD_REQUEST, "'level'"),
    };
    if !VALID_LEVELS.contains(&level.as_str()) {
        return e_detail(
            StatusCode::BAD_REQUEST,
            format!("level must be one of {}", py_list(&VALID_LEVELS)),
        );
    }
    let level_c = level.clone();
    let updated = with_db(&state, move |conn| {
        let conn = conn.unwrap();
        // 404 unless the member is an enrolled, non-instructor student.
        if !db::is_student(conn, member_id, enrollment_id).unwrap_or(false) {
            return Ok::<bool, db::DbError>(false);
        }
        db::set_enrollment_level(conn, member_id, enrollment_id, &level_c)
    })
    .await;
    match updated {
        Ok(true) => Json(json!({ "ok": true, "level": level })).into_response(),
        Ok(false) => e_detail(
            StatusCode::NOT_FOUND,
            "student not found in this enrollment",
        ),
        Err(e) => server_error("set_student_level", e),
    }
}

pub(crate) async fn export_students(
    State(state): State<SharedState>,
    Path(enrollment_id): Path<i64>,
    headers: HeaderMap,
) -> Response {
    if state.db.is_none() {
        return e_error(StatusCode::SERVICE_UNAVAILABLE, "no database");
    }
    let _admin = match require_admin(&state, bsession(&headers)).await {
        Ok(c) => c,
        Err(_) => return e_detail(StatusCode::FORBIDDEN, "Only admins may export students"),
    };
    let students = match with_db(&state, move |conn| {
        db::list_enrolled_students(conn.unwrap(), enrollment_id)
    })
    .await
    {
        Ok(s) => s,
        Err(e) => return server_error("export_students", e),
    };
    // csv.writer uses \r\n terminators — match byte-for-byte.
    let mut csv = String::from("name,level\r\n");
    for (name, level) in students {
        csv.push_str(&csv_field(&name));
        csv.push(',');
        csv.push_str(&csv_field(&level));
        csv.push_str("\r\n");
    }
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/csv".to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"students_season_{enrollment_id}.csv\""),
            ),
        ],
        csv,
    )
        .into_response()
}

/// Minimal RFC4180 field quoting matching Python's `csv.writer` defaults.
pub(crate) fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}
