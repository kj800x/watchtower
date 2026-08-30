use serde::{Deserialize, Serialize};

use crate::error::AppResult;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TagHistory {
    pub tag_id: u64,
    pub digest: String,
    pub seen_at: chrono::DateTime<chrono::Utc>,
}

impl TagHistory {
    pub fn from_row(row: &rusqlite::Row) -> AppResult<Self> {
        Ok(TagHistory {
            tag_id: row.get(0)?,
            digest: row.get(1)?,
            seen_at: chrono::DateTime::from_timestamp(row.get::<usize, i64>(2)?, 0).unwrap(),
        })
    }
}
