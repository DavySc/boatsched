//! Data-access helpers, grouped by domain. SQL strings, `COLLATE NOCASE`
//! matching, `ORDER BY`, and `ON CONFLICT` clauses are load-bearing for
//! compatibility with the live database — don't "tidy" them.

mod eval;
mod instructors;
mod members;
mod schedules;

pub use eval::*;
pub use instructors::*;
pub use members::*;
pub use schedules::*;

/// Errors from the data layer.
#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    /// A referenced row doesn't exist (e.g. unknown member or missing
    /// enrollment); the server maps it to 400.
    #[error("{0}")]
    Missing(String),
    /// Rejected input or a conflict (e.g. empty/colliding name, deleting an
    /// instructor); the server maps it to 400/409. Messages are user-facing.
    #[error("{0}")]
    Invalid(String),
}

type Result<T> = std::result::Result<T, DbError>;
