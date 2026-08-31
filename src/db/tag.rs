use crate::{db::tag_history::TagHistory, error::AppResult};
use r2d2::PooledConnection;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::params;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Tag {
    pub id: u64,
    pub repo_id: u64,
    pub tag: String,
    pub active: bool,
    pub first_seen_at: chrono::DateTime<chrono::Utc>,
    pub last_seen_at: chrono::DateTime<chrono::Utc>,
}

impl Tag {
    pub fn from_row(row: &rusqlite::Row) -> AppResult<Self> {
        Ok(Tag {
            id: row.get(0)?,
            repo_id: row.get(1)?,
            tag: row.get(2)?,
            active: row.get(3)?,
            first_seen_at: chrono::DateTime::from_timestamp(row.get::<usize, i64>(4)?, 0).unwrap(),
            last_seen_at: chrono::DateTime::from_timestamp(row.get::<usize, i64>(5)?, 0).unwrap(),
        })
    }

    /// Insert the tag if new, otherwise bump `last_seen_at` and re-activate.
    /// `seen_at` should be the single timestamp used for the whole refresh
    /// pass so that `deactivate_missing` can distinguish this pass's tags.
    pub fn upsert(
        repo_id: u64,
        tag: &str,
        seen_at: chrono::DateTime<chrono::Utc>,
        conn: &PooledConnection<SqliteConnectionManager>,
    ) -> AppResult<Self> {
        Ok(conn
            .prepare(
                "INSERT INTO tag (repo_id, tag, active, first_seen_at, last_seen_at)
                 VALUES (?1, ?2, TRUE, ?3, ?3)
                 ON CONFLICT(repo_id, tag) DO UPDATE SET
                     active = TRUE,
                     last_seen_at = ?3
                 RETURNING id, repo_id, tag, active, first_seen_at, last_seen_at",
            )?
            .query_row(params![repo_id, tag, seen_at.timestamp()], |row| {
                Ok(Self::from_row(row))
            })??)
    }

    /// Mark tags that were not seen in the refresh pass at `seen_at` (i.e.
    /// have disappeared from the registry's tag list) as inactive.
    pub fn deactivate_missing(
        repo_id: u64,
        seen_at: chrono::DateTime<chrono::Utc>,
        conn: &PooledConnection<SqliteConnectionManager>,
    ) -> AppResult<usize> {
        Ok(conn
            .prepare(
                "UPDATE tag SET active = FALSE
                 WHERE repo_id = ?1 AND active = TRUE AND last_seen_at < ?2",
            )?
            .execute(params![repo_id, seen_at.timestamp()])?)
    }

    pub fn history(
        &self,
        conn: &PooledConnection<SqliteConnectionManager>,
    ) -> AppResult<Vec<TagHistory>> {
        let mut stmt =
            conn.prepare("SELECT tag_id, digest, first_seen_at, last_seen_at FROM tag_history WHERE tag_id = ?1 ORDER BY first_seen_at DESC")?;
        let mut rows = stmt.query(params![self.id])?;
        let mut history = Vec::new();

        while let Some(row) = rows.next()? {
            history.push(TagHistory::from_row(row)?);
        }

        Ok(history)
    }
}
