use crate::error::AppResult;
use r2d2::PooledConnection;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RepoState {
    pub repo_id: u64,
    pub last_checked_at: Option<chrono::DateTime<chrono::Utc>>,
    pub last_attempted_at: Option<chrono::DateTime<chrono::Utc>>,
    pub last_error: Option<String>,
    pub consecutive_failures: u32,
}

fn timestamp(value: Option<i64>) -> Option<chrono::DateTime<chrono::Utc>> {
    value.and_then(|ts| chrono::DateTime::from_timestamp(ts, 0))
}

impl RepoState {
    pub fn from_row(row: &rusqlite::Row) -> AppResult<Self> {
        Ok(RepoState {
            repo_id: row.get(0)?,
            last_checked_at: timestamp(row.get(1)?),
            last_attempted_at: timestamp(row.get(2)?),
            last_error: row.get(3)?,
            consecutive_failures: row.get(4)?,
        })
    }

    pub fn get(
        repo_id: u64,
        conn: &PooledConnection<SqliteConnectionManager>,
    ) -> AppResult<Option<Self>> {
        Ok(conn
            .prepare(
                "SELECT repo_id, last_checked_at, last_attempted_at, last_error, consecutive_failures
                 FROM repo_state WHERE repo_id = ?1",
            )?
            .query_row(params![repo_id], |row| {
                Ok(Self::from_row(row))
            })
            .optional()?
            .transpose()?)
    }

    pub fn record_success(
        repo_id: u64,
        at: chrono::DateTime<chrono::Utc>,
        conn: &PooledConnection<SqliteConnectionManager>,
    ) -> AppResult<()> {
        conn.prepare(
            "INSERT INTO repo_state (repo_id, last_checked_at, last_attempted_at, last_error, consecutive_failures)
             VALUES (?1, ?2, ?2, NULL, 0)
             ON CONFLICT(repo_id) DO UPDATE SET
                 last_checked_at = ?2,
                 last_attempted_at = ?2,
                 last_error = NULL,
                 consecutive_failures = 0",
        )?
        .execute(params![repo_id, at.timestamp()])?;
        Ok(())
    }

    pub fn record_failure(
        repo_id: u64,
        at: chrono::DateTime<chrono::Utc>,
        error: &str,
        conn: &PooledConnection<SqliteConnectionManager>,
    ) -> AppResult<()> {
        conn.prepare(
            "INSERT INTO repo_state (repo_id, last_attempted_at, last_error, consecutive_failures)
             VALUES (?1, ?2, ?3, 1)
             ON CONFLICT(repo_id) DO UPDATE SET
                 last_attempted_at = ?2,
                 last_error = ?3,
                 consecutive_failures = consecutive_failures + 1",
        )?
        .execute(params![repo_id, at.timestamp(), error])?;
        Ok(())
    }
}
