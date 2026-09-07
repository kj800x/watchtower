use crate::{
    db::{repo_state::RepoState, tag::Tag},
    error::AppResult,
};
use r2d2::PooledConnection;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::params;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Repo {
    pub id: u64,
    pub registry: String,
    pub name: String,
    pub active: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RepoEgg {
    pub registry: String,
    pub name: String,
}

impl Repo {
    pub fn from_row(row: &rusqlite::Row) -> AppResult<Self> {
        Ok(Repo {
            id: row.get(0)?,
            registry: row.get(1)?,
            name: row.get(2)?,
            active: row.get(3)?,
        })
    }

    pub fn all(conn: &PooledConnection<SqliteConnectionManager>) -> AppResult<Vec<Self>> {
        let mut stmt =
            conn.prepare("SELECT id, registry, name, active FROM repo ORDER BY name ASC")?;
        let mut rows = stmt.query(params![])?;
        let mut repos = Vec::new();
        while let Some(row) = rows.next()? {
            repos.push(Self::from_row(row)?);
        }
        Ok(repos)
    }

    pub fn all_active(conn: &PooledConnection<SqliteConnectionManager>) -> AppResult<Vec<Self>> {
        let mut stmt = conn.prepare(
            "SELECT id, registry, name, active FROM repo WHERE active = TRUE ORDER BY name ASC",
        )?;
        let mut rows = stmt.query(params![])?;
        let mut repos = Vec::new();
        while let Some(row) = rows.next()? {
            repos.push(Self::from_row(row)?);
        }
        Ok(repos)
    }

    pub fn get(
        id: u64,
        conn: &PooledConnection<SqliteConnectionManager>,
    ) -> AppResult<Option<Self>> {
        let mut stmt = conn.prepare("SELECT id, registry, name, active FROM repo WHERE id = ?1")?;
        let mut rows = stmt.query(params![id])?;

        // FIXME: There's a better way to get just one row, I'm sure.
        let mut repo = None;
        while let Some(row) = rows.next()? {
            repo = Some(Self::from_row(row)?);
        }

        Ok(repo)
    }

    pub fn get_by_egg(
        egg: &RepoEgg,
        conn: &PooledConnection<SqliteConnectionManager>,
    ) -> AppResult<Option<Self>> {
        let mut stmt = conn.prepare(
            "SELECT id, registry, name, active FROM repo WHERE registry = ?1 AND name = ?2",
        )?;
        let mut rows = stmt.query(params![egg.registry, egg.name])?;

        // FIXME: There's a better way to get just one row, I'm sure.
        let mut repo = None;
        while let Some(row) = rows.next()? {
            repo = Some(Self::from_row(row)?);
        }

        Ok(repo)
    }

    /// Insert the repo, or re-activate it if it exists. The id is kept on
    /// conflict (INSERT OR REPLACE would delete the row and orphan its tags).
    pub fn upsert(
        repo: &RepoEgg,
        conn: &PooledConnection<SqliteConnectionManager>,
    ) -> AppResult<Self> {
        Ok(conn
            .prepare(
                "INSERT INTO repo (registry, name, active) VALUES (?1, ?2, TRUE)
                 ON CONFLICT(registry, name) DO UPDATE SET active = TRUE
                 RETURNING id, registry, name, active",
            )?
            .query_row(params![repo.registry, repo.name], |row| {
                Ok(Self::from_row(row))
            })??)
    }

    pub fn save(
        &self,
        active: bool,
        conn: &PooledConnection<SqliteConnectionManager>,
    ) -> AppResult<()> {
        conn.prepare("UPDATE repo SET active = ?1 WHERE id = ?2")?
            .execute(params![active, self.id])?;

        Ok(())
    }

    pub fn state(
        &self,
        conn: &PooledConnection<SqliteConnectionManager>,
    ) -> AppResult<Option<RepoState>> {
        RepoState::get(self.id, conn)
    }

    /// (active, total) tag counts for this repo.
    pub fn tag_counts(
        &self,
        conn: &PooledConnection<SqliteConnectionManager>,
    ) -> AppResult<(u64, u64)> {
        Ok(conn
            .prepare("SELECT coalesce(sum(active), 0), count(*) FROM tag WHERE repo_id = ?1")?
            .query_row(params![self.id], |row| Ok((row.get(0)?, row.get(1)?)))?)
    }

    pub fn tags(&self, conn: &PooledConnection<SqliteConnectionManager>) -> AppResult<Vec<Tag>> {
        let mut stmt = conn.prepare(
            "SELECT id, repo_id, tag, active, first_seen_at, last_seen_at FROM tag WHERE repo_id = ?1 ORDER BY first_seen_at DESC, tag DESC",
        )?;
        let mut rows = stmt.query(params![self.id])?;

        let mut tags = Vec::new();
        while let Some(row) = rows.next()? {
            tags.push(Tag::from_row(row)?);
        }

        Ok(tags)
    }
}
