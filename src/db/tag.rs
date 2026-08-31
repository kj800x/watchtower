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
