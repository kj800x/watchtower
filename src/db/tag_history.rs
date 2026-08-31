use r2d2::PooledConnection;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::error::AppResult;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TagHistory {
    pub tag_id: u64,
    pub digest: String,
    pub first_seen_at: chrono::DateTime<chrono::Utc>,
    pub last_seen_at: chrono::DateTime<chrono::Utc>,
}

impl TagHistory {
    pub fn from_row(row: &rusqlite::Row) -> AppResult<Self> {
        Ok(TagHistory {
            tag_id: row.get(0)?,
            digest: row.get(1)?,
            first_seen_at: chrono::DateTime::from_timestamp(row.get::<usize, i64>(2)?, 0).unwrap(),
            last_seen_at: chrono::DateTime::from_timestamp(row.get::<usize, i64>(3)?, 0).unwrap(),
        })
    }

    pub fn latest(
        tag_id: u64,
        conn: &PooledConnection<SqliteConnectionManager>,
    ) -> AppResult<Option<Self>> {
        Ok(conn
            .prepare(
                "SELECT tag_id, digest, first_seen_at, last_seen_at FROM tag_history
                 WHERE tag_id = ?1 ORDER BY first_seen_at DESC LIMIT 1",
            )?
            .query_row(params![tag_id], |row| Ok(Self::from_row(row)))
            .optional()?
            .transpose()?)
    }

    /// Record a digest observation. If the tag still points at the same digest
    /// as its latest history row, bump that row's `last_seen_at`; if the
    /// digest changed (or this is the first observation), append a new row.
    pub fn record(
        tag_id: u64,
        digest: &str,
        seen_at: chrono::DateTime<chrono::Utc>,
        conn: &PooledConnection<SqliteConnectionManager>,
    ) -> AppResult<()> {
        match Self::latest(tag_id, conn)? {
            Some(latest) if latest.digest == digest => {
                conn.prepare(
                    "UPDATE tag_history SET last_seen_at = ?1
                     WHERE tag_id = ?2 AND digest = ?3 AND first_seen_at = ?4",
                )?
                .execute(params![
                    seen_at.timestamp(),
                    tag_id,
                    digest,
                    latest.first_seen_at.timestamp()
                ])?;
            }
            _ => {
                conn.prepare(
                    "INSERT INTO tag_history (tag_id, digest, first_seen_at, last_seen_at)
                     VALUES (?1, ?2, ?3, ?3)",
                )?
                .execute(params![tag_id, digest, seen_at.timestamp()])?;
            }
        }
        Ok(())
    }
}
