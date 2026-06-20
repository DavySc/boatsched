//! Roster input: load `(name, level)` pairs from CSV files, bytes, or counts.
//! Mirrors the Python `roster.py` semantics exactly, including BOM stripping,
//! case-insensitive headers, whitespace trimming, and the error conditions.

use boatsched_core::Kid;
use thiserror::Error;

/// Raised when roster input is malformed (mirrors Python `RosterError`).
#[derive(Debug, Error, PartialEq, Eq)]
#[error("{0}")]
pub struct RosterError(pub String);

fn err(msg: &str) -> RosterError {
    RosterError(msg.to_string())
}

/// Strip a leading UTF-8 BOM, mirroring Python's `utf-8-sig` decoding.
fn strip_bom(bytes: &[u8]) -> &[u8] {
    bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes)
}

/// Core ingestion shared by the file and bytes loaders. Mirrors
/// `_read_dictreader`: requires `name` + `level` columns (matched
/// case-insensitively after trimming), trims cell values, skips rows whose
/// name is blank, and errors on an empty CSV or a header-only file.
fn read_records(text: &str) -> Result<Vec<Kid>, RosterError> {
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .flexible(true)
        .from_reader(text.as_bytes());

    let headers = reader.headers().map_err(|_| err("empty CSV"))?.clone();
    if headers.is_empty() {
        return Err(err("empty CSV"));
    }

    // Map normalized (lower + trim) header -> column index.
    let mut name_idx: Option<usize> = None;
    let mut level_idx: Option<usize> = None;
    for (i, h) in headers.iter().enumerate() {
        match h.to_lowercase().trim() {
            "name" if name_idx.is_none() => name_idx = Some(i),
            "level" if level_idx.is_none() => level_idx = Some(i),
            _ => {}
        }
    }
    let (name_idx, level_idx) = match (name_idx, level_idx) {
        (Some(n), Some(l)) => (n, l),
        _ => return Err(err("CSV must have columns: name, level")),
    };

    let mut kids = Vec::new();
    for rec in reader.records() {
        let rec = rec.map_err(|e| err(&format!("CSV parse error: {e}")))?;
        let name = rec.get(name_idx).unwrap_or("").trim();
        let level = rec.get(level_idx).unwrap_or("").trim();
        if !name.is_empty() {
            kids.push(Kid::new(name, level));
        }
    }
    if kids.is_empty() {
        return Err(err("no data rows"));
    }
    Ok(kids)
}

/// Load a roster from a CSV file with columns `name`, `level`.
/// The `io::Error` variant mirrors Python `load_roster` raising `OSError`.
pub fn load_roster(path: &str) -> Result<Vec<Kid>, LoadError> {
    let bytes = std::fs::read(path)?;
    Ok(parse_roster_bytes(&bytes)?)
}

/// Error for [`load_roster`]: either the file could not be read or its
/// contents were malformed.
#[derive(Debug, Error)]
pub enum LoadError {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Roster(#[from] RosterError),
}

/// Load a roster from raw CSV bytes (e.g. an HTTP upload).
pub fn parse_roster_bytes(raw: &[u8]) -> Result<Vec<Kid>, RosterError> {
    let text = String::from_utf8_lossy(strip_bom(raw));
    read_records(&text)
}

/// Build a synthetic roster from `[(level, n)]` counts, mirroring
/// `roster_from_counts`. Names are `"{level}{j}"`. Iteration order follows the
/// input order (Python dict insertion order).
pub fn roster_from_counts(counts: &[(&str, usize)]) -> Vec<Kid> {
    let mut kids = Vec::new();
    for (level, n) in counts {
        for j in 0..*n {
            kids.push(Kid::new(format!("{level}{j}"), *level));
        }
    }
    kids
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- parse_roster_bytes (mirrors TestParseRosterBytes + test_boatsched) ----
    #[test]
    fn bom_stripped() {
        let kids = parse_roster_bytes(b"\xef\xbb\xbfname,level\nAlice,k\n").unwrap();
        assert_eq!(kids, vec![Kid::new("Alice", "k")]);
    }

    #[test]
    fn case_insensitive_headers() {
        let kids = parse_roster_bytes(b"Name,Level\nAlice,k\n").unwrap();
        assert_eq!(kids, vec![Kid::new("Alice", "k")]);
    }

    #[test]
    fn whitespace_stripped_from_values() {
        let kids = parse_roster_bytes(b"name,level\n  Alice  ,  k  \n").unwrap();
        assert_eq!(kids[0], Kid::new("Alice", "k"));
    }

    #[test]
    fn missing_name_column() {
        assert!(parse_roster_bytes(b"person,level\nAlice,k\n").is_err());
    }

    #[test]
    fn missing_level_column() {
        assert!(parse_roster_bytes(b"name,grade\nAlice,k\n").is_err());
    }

    #[test]
    fn empty_input() {
        assert!(parse_roster_bytes(b"").is_err());
    }

    #[test]
    fn header_row_only_no_data() {
        assert!(parse_roster_bytes(b"name,level\n").is_err());
    }

    #[test]
    fn skips_rows_with_blank_name() {
        let kids = parse_roster_bytes(b"name,level\nAlice,k\n,b\nBob,b\n").unwrap();
        let names: Vec<&str> = kids.iter().map(|k| k.name.as_str()).collect();
        assert_eq!(names, vec!["Alice", "Bob"]);
    }

    #[test]
    fn multiple_kids() {
        let kids = parse_roster_bytes(b"name,level\nA,k\nB,b\nC,A\n").unwrap();
        assert_eq!(kids.len(), 3);
        assert_eq!(kids[2], Kid::new("C", "A"));
    }

    #[test]
    fn parse_bytes_requires_columns() {
        assert!(parse_roster_bytes(b"foo,bar\n1,2\n").is_err());
    }

    #[test]
    fn parse_bytes_ok() {
        let kids = parse_roster_bytes(b"name,level\nAlice,k\nBob,b\n").unwrap();
        assert_eq!(kids, vec![Kid::new("Alice", "k"), Kid::new("Bob", "b")]);
    }

    // ---- roster_from_counts (mirrors TestRosterFromCounts) ----
    #[test]
    fn counts_correct_total() {
        assert_eq!(roster_from_counts(&[("k", 3), ("b", 2)]).len(), 5);
    }

    #[test]
    fn counts_level_counts() {
        let kids = roster_from_counts(&[("k", 2), ("b", 3), ("A", 1)]);
        let count = |lv: &str| kids.iter().filter(|k| k.level == lv).count();
        assert_eq!(count("k"), 2);
        assert_eq!(count("b"), 3);
        assert_eq!(count("A"), 1);
    }

    #[test]
    fn counts_names_are_unique() {
        let kids = roster_from_counts(&[("k", 10)]);
        let mut names: Vec<&str> = kids.iter().map(|k| k.name.as_str()).collect();
        let n = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), n);
    }

    #[test]
    fn counts_names_unique_across_levels() {
        let kids = roster_from_counts(&[("k", 5), ("b", 5)]);
        let mut names: Vec<&str> = kids.iter().map(|k| k.name.as_str()).collect();
        let n = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), n);
    }

    #[test]
    fn counts_empty_returns_empty() {
        assert!(roster_from_counts(&[]).is_empty());
    }

    // ---- load_roster (mirrors TestLoadRoster) ----
    #[test]
    fn loads_from_file() {
        let dir = std::env::temp_dir().join(format!("bs_roster_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("roster.csv");
        std::fs::write(&f, "name,level\nAlice,k\nBob,b\n").unwrap();
        let kids = load_roster(f.to_str().unwrap()).unwrap();
        assert_eq!(kids, vec![Kid::new("Alice", "k"), Kid::new("Bob", "b")]);
    }

    #[test]
    fn bom_file() {
        let dir = std::env::temp_dir().join(format!("bs_roster_bom_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("roster.csv");
        std::fs::write(&f, b"\xef\xbb\xbfname,level\nAlice,k\n").unwrap();
        let kids = load_roster(f.to_str().unwrap()).unwrap();
        assert_eq!(kids, vec![Kid::new("Alice", "k")]);
    }

    #[test]
    fn missing_file_raises() {
        match load_roster("/no/such/file.csv") {
            Err(LoadError::Io(_)) => {}
            other => panic!("expected Io error, got {other:?}"),
        }
    }
}
