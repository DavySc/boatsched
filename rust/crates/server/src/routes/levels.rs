use crate::error::AppError;
use crate::*;
use serde_json::json;

pub(crate) async fn set_student_level(
    db: Db,
    Path((enrollment_id, member_id)): Path<(i64, i64)>,
    headers: HeaderMap,
    body: Bytes,
) -> Handled {
    let _admin = db
        .admin(
            &headers,
            AppError::detail(StatusCode::FORBIDDEN, "Only admins may edit student levels"),
        )
        .await?;
    let v = parse_json(&body)?;
    let level = v
        .get("level")
        .map(|l| json_str(l).trim().to_string())
        .ok_or_else(|| AppError::bad_request("'level'"))?;
    if !VALID_LEVELS.contains(&level.as_str()) {
        return Err(AppError::detail(
            StatusCode::BAD_REQUEST,
            format!("level must be one of {}", py_list(&VALID_LEVELS)),
        ));
    }
    let level_c = level.clone();
    let updated = db
        .run(move |conn| {
            // 404 unless the member is an enrolled, non-instructor student.
            if !db::is_student(conn, member_id, enrollment_id).unwrap_or(false) {
                return Ok::<bool, db::DbError>(false);
            }
            db::set_enrollment_level(conn, member_id, enrollment_id, &level_c)
        })
        .await
        .map_err(|e| AppError::internal("set_student_level", e))?;
    if updated {
        Ok(Json(json!({ "ok": true, "level": level })).into_response())
    } else {
        Err(AppError::detail(
            StatusCode::NOT_FOUND,
            "student not found in this enrollment",
        ))
    }
}

pub(crate) async fn export_students(
    db: Db,
    Path(enrollment_id): Path<i64>,
    headers: HeaderMap,
) -> Handled {
    let _admin = db
        .admin(
            &headers,
            AppError::detail(StatusCode::FORBIDDEN, "Only admins may export students"),
        )
        .await?;
    let students = db
        .run(move |conn| db::list_enrolled_students(conn, enrollment_id))
        .await
        .map_err(|e| AppError::internal("export_students", e))?;
    // csv.writer uses \r\n terminators — match byte-for-byte.
    let mut csv = String::from("name,level\r\n");
    for (name, level) in students {
        csv.push_str(&csv_field(&name));
        csv.push(',');
        csv.push_str(&csv_field(&level));
        csv.push_str("\r\n");
    }
    Ok((
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
        .into_response())
}

/// Minimal RFC4180 field quoting matching Python's `csv.writer` defaults.
fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}
