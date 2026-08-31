use r2d2::CustomizeConnection;
use rusqlite::Connection;
use std::ops::Deref;

pub mod migrations;
pub mod repo;
pub mod repo_state;
pub mod tag;
pub mod tag_history;

pub struct ExistenceResult {
    id: u64,
}

impl Deref for ExistenceResult {
    type Target = u64;

    fn deref(&self) -> &Self::Target {
        &self.id
    }
}

#[derive(Debug)]
pub struct SqliteConnectionCustomizer;

impl CustomizeConnection<Connection, rusqlite::Error> for SqliteConnectionCustomizer {
    fn on_acquire(&self, conn: &mut Connection) -> Result<(), rusqlite::Error> {
        conn.pragma_update(None, "foreign_keys", "ON")?;
        Ok(())
    }
}
