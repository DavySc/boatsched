use crate::*;

// ---------------------------------------------------------------------------
// Small ports of server.py module helpers
// ---------------------------------------------------------------------------

/// Port of `_extract_multipart`: pull the first file part's body out of a
/// multipart/form-data payload.
pub(crate) fn extract_multipart(raw: &[u8], ctype: &str) -> Vec<u8> {
    let boundary = match ctype.split("boundary=").last() {
        Some(b) => b.trim().trim_matches('"'),
        None => return raw.to_vec(),
    };
    let sep = format!("--{boundary}").into_bytes();
    for part in split_bytes(raw, &sep) {
        if let Some(idx) = find_sub(part, b"\r\n\r\n") {
            let head = &part[..idx];
            if find_sub(head, b"filename=").is_some() {
                let body = &part[idx + 4..];
                // strip the trailing CRLF before the next boundary
                let end = body
                    .windows(2)
                    .rposition(|w| w == b"\r\n")
                    .unwrap_or(body.len());
                return body[..end].to_vec();
            }
        }
    }
    raw.to_vec()
}

/// Port of `_parse_names_csv`: extract names from a CSV with a `name` column,
/// else treat each line's first comma-token as a name.
pub(crate) fn parse_names_csv(text: &str) -> Vec<String> {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    if lines.is_empty() {
        return Vec::new();
    }
    // Header with a "name" column?
    let header: Vec<String> = lines[0]
        .split(',')
        .map(|c| c.trim().to_lowercase())
        .collect();
    if let Some(col) = header.iter().position(|c| c == "name") {
        return lines[1..]
            .iter()
            .filter_map(|line| {
                line.split(',')
                    .nth(col)
                    .map(|c| c.trim().to_string())
                    .filter(|s| !s.is_empty())
            })
            .collect();
    }
    // No 'name' header: first token of each line.
    lines
        .iter()
        .map(|line| {
            line.trim()
                .split(',')
                .next()
                .unwrap_or("")
                .trim()
                .trim_matches('"')
                .to_string()
        })
        .filter(|name| !name.is_empty() && !matches!(name.to_lowercase().as_str(), "name" | "naam"))
        .collect()
}

pub(crate) fn split_bytes<'a>(data: &'a [u8], sep: &[u8]) -> Vec<&'a [u8]> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i + sep.len() <= data.len() {
        if &data[i..i + sep.len()] == sep {
            out.push(&data[start..i]);
            i += sep.len();
            start = i;
        } else {
            i += 1;
        }
    }
    out.push(&data[start..]);
    out
}

pub(crate) fn find_sub(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    (0..=haystack.len() - needle.len()).find(|&i| &haystack[i..i + needle.len()] == needle)
}

// ---------------------------------------------------------------------------
// JSON coercion helpers — mirror Python's permissive str()/int()/bool() coercion
// ---------------------------------------------------------------------------

/// Like Python `str(body[k])`: numbers/bools become their textual form.
pub(crate) fn json_str(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => {
            if *b {
                "True".into()
            } else {
                "False".into()
            }
        }
        Value::Null => "None".into(),
        other => other.to_string(),
    }
}

/// Like Python `int(body[k])`: accept JSON ints and numeric strings.
pub(crate) fn json_int(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
        Value::String(s) => s.trim().parse::<i64>().ok(),
        _ => None,
    }
}

/// Like Python `bool(body[k])`: truthiness of common JSON shapes.
pub(crate) fn json_bool(v: &Value) -> bool {
    match v {
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().map_or(false, |f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Null => false,
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

/// Render `["k", "b", "A", "B"]` the way Python's `list(...)` repr does, for the
/// invalid-level error message.
pub(crate) fn py_list(items: &[&str]) -> String {
    let inner = items
        .iter()
        .map(|s| format!("'{s}'"))
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{inner}]")
}

/// Best-effort UTC ISO-8601 timestamp (the health endpoint's `timestamp`; only
/// its presence is asserted). Avoids a chrono dependency.
pub(crate) fn utc_now_iso() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs() as i64;
    let days = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400);
    let (h, mi, s) = (tod / 3600, (tod % 3600) / 60, tod % 60);
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}+00:00")
}

/// Days since 1970-01-01 → (year, month, day). Howard Hinnant's algorithm.
pub(crate) fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    (y + (m <= 2) as i64, m, d)
}
