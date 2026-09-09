//! The event feed: one row per change the poller observes, in order.
//!
//! Consumers (cicd) read `GET /api/events?after=<id>` and keep the last id
//! they handled as a cursor, so either side can be down for as long as it
//! likes and nothing is lost. Rows are append-only and never edited.
//!
//! A repo's first successful refresh emits nothing: a newly registered
//! image with thousands of historical tags is a baseline, not thousands of
//! additions. From the second refresh on, every new tag, every floating tag
//! that moves to a new digest, and every tag that disappears is one event.

use r2d2::PooledConnection;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::{db::version_exclusion::VersionExclusion, error::AppResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    /// A tag appeared in the registry's tag list.
    TagAdded,
    /// A tag that was already known now points at a different digest.
    TagMoved,
    /// A tag is no longer in the registry's tag list.
    TagRemoved,
}

impl EventKind {
    fn as_str(self) -> &'static str {
        match self {
            EventKind::TagAdded => "tag_added",
            EventKind::TagMoved => "tag_moved",
            EventKind::TagRemoved => "tag_removed",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s {
            "tag_added" => Some(EventKind::TagAdded),
            "tag_moved" => Some(EventKind::TagMoved),
            "tag_removed" => Some(EventKind::TagRemoved),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Event {
    pub id: u64,
    pub repo_id: u64,
    pub registry: String,
    pub name: String,
    pub tag: String,
    /// The version and variant the tag names, as `classify::parse_version`
    /// reads it, so a consumer can match a range without parsing tags.
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub variant: Option<String>,
    pub kind: EventKind,
    /// The digest the tag points at after this event; absent for removals
    /// and for additions whose digest is not fetched yet.
    pub digest: Option<String>,
    /// The digest before a move.
    pub previous_digest: Option<String>,
    pub at: chrono::DateTime<chrono::Utc>,
}

/// What to append; the id is assigned on insert.
#[derive(Debug, Clone)]
pub struct NewEvent {
    pub repo_id: u64,
    pub tag: String,
    pub kind: EventKind,
    pub digest: Option<String>,
    pub previous_digest: Option<String>,
}

const COLUMNS: &str =
    "e.id, e.repo_id, r.registry, r.name, e.tag, e.kind, e.digest, e.previous_digest, e.at";

impl Event {
    fn from_row(row: &rusqlite::Row) -> AppResult<Self> {
        let kind: String = row.get(5)?;
        let tag: String = row.get(4)?;
        let parsed = crate::poller::classify::parse_version(&tag);
        Ok(Event {
            id: row.get(0)?,
            repo_id: row.get(1)?,
            registry: row.get(2)?,
            name: row.get(3)?,
            version: parsed.as_ref().map(|p| p.version.clone()),
            variant: parsed.and_then(|p| p.variant),
            tag,
            kind: EventKind::parse(&kind).unwrap_or(EventKind::TagAdded),
            digest: row.get(6)?,
            previous_digest: row.get(7)?,
            at: chrono::DateTime::from_timestamp(row.get::<usize, i64>(8)?, 0).unwrap_or_default(),
        })
    }

    pub fn record(
        new: &NewEvent,
        at: chrono::DateTime<chrono::Utc>,
        conn: &PooledConnection<SqliteConnectionManager>,
    ) -> AppResult<u64> {
        conn.prepare(
            "INSERT INTO event (repo_id, tag, kind, digest, previous_digest, at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )?
        .execute(params![
            new.repo_id,
            new.tag,
            new.kind.as_str(),
            new.digest,
            new.previous_digest,
            at.timestamp()
        ])?;
        Ok(conn.last_insert_rowid() as u64)
    }

    /// Events with an id greater than `after`, oldest first, at most `limit`.
    pub fn list_after(
        after: u64,
        limit: usize,
        conn: &PooledConnection<SqliteConnectionManager>,
    ) -> AppResult<Vec<Self>> {
        let mut stmt = conn.prepare(&format!(
            "SELECT {COLUMNS} FROM event e JOIN repo r ON r.id = e.repo_id
             WHERE e.id > ?1 ORDER BY e.id ASC LIMIT ?2"
        ))?;
        let mut rows = stmt.query(params![after, limit as i64])?;
        let mut events = Vec::new();
        while let Some(row) = rows.next()? {
            events.push(Self::from_row(row)?);
        }
        Ok(events)
    }

    /// The feed as consumers read it: like [`Event::list_after`], minus
    /// events of tags whose version an admin has excluded (see
    /// `db/version_exclusion.rs`). Ids stay sparse rather than renumbered,
    /// so a cursor works the same way; the scan continues past skipped
    /// rows until `limit` events are found or the table ends.
    pub fn feed(
        after: u64,
        limit: usize,
        conn: &PooledConnection<SqliteConnectionManager>,
    ) -> AppResult<Vec<Self>> {
        let exclusions = VersionExclusion::all_by_repo(conn)?;
        if exclusions.is_empty() {
            return Self::list_after(after, limit, conn);
        }
        let mut events = Vec::with_capacity(limit);
        let mut cursor = after;
        loop {
            let batch = Self::list_after(cursor, limit, conn)?;
            let exhausted = batch.len() < limit;
            for event in batch {
                cursor = event.id;
                let excluded = exclusions
                    .get(&event.repo_id)
                    .is_some_and(|set| set.excludes(&event.tag));
                if !excluded {
                    events.push(event);
                    if events.len() == limit {
                        return Ok(events);
                    }
                }
            }
            if exhausted {
                return Ok(events);
            }
        }
    }

    /// The newest event id, or 0 when there are none. A consumer that wants
    /// to start "from now" uses this as its first cursor.
    pub fn latest_id(conn: &PooledConnection<SqliteConnectionManager>) -> AppResult<u64> {
        Ok(conn
            .prepare("SELECT coalesce(max(id), 0) FROM event")?
            .query_row(params![], |row| row.get(0))?)
    }

    /// Recent events for one repo, newest first.
    pub fn recent_for_repo(
        repo_id: u64,
        limit: usize,
        conn: &PooledConnection<SqliteConnectionManager>,
    ) -> AppResult<Vec<Self>> {
        let mut stmt = conn.prepare(&format!(
            "SELECT {COLUMNS} FROM event e JOIN repo r ON r.id = e.repo_id
             WHERE e.repo_id = ?1 ORDER BY e.id DESC LIMIT ?2"
        ))?;
        let mut rows = stmt.query(params![repo_id, limit as i64])?;
        let mut events = Vec::new();
        while let Some(row) = rows.next()? {
            events.push(Self::from_row(row)?);
        }
        Ok(events)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{
        SqliteConnectionCustomizer,
        migrations::migrate,
        repo::{Repo, RepoEgg},
    };
    use r2d2::Pool;

    fn pool() -> Pool<SqliteConnectionManager> {
        let pool = Pool::builder()
            .max_size(1)
            .connection_customizer(Box::new(SqliteConnectionCustomizer))
            .build(SqliteConnectionManager::memory())
            .unwrap();
        migrate(pool.get().unwrap()).unwrap();
        pool
    }

    #[test]
    fn events_are_listed_in_order_after_a_cursor() {
        let pool = pool();
        let conn = pool.get().unwrap();
        let repo = Repo::upsert(
            &RepoEgg {
                registry: "docker.io".into(),
                name: "library/nginx".into(),
            },
            &conn,
        )
        .unwrap();
        let now = chrono::Utc::now();
        assert_eq!(Event::latest_id(&conn).unwrap(), 0);
        let first = Event::record(
            &NewEvent {
                repo_id: repo.id,
                tag: "1.27.3".into(),
                kind: EventKind::TagAdded,
                digest: Some("sha256:a".into()),
                previous_digest: None,
            },
            now,
            &conn,
        )
        .unwrap();
        let second = Event::record(
            &NewEvent {
                repo_id: repo.id,
                tag: "1.27".into(),
                kind: EventKind::TagMoved,
                digest: Some("sha256:b".into()),
                previous_digest: Some("sha256:a".into()),
            },
            now,
            &conn,
        )
        .unwrap();
        assert!(second > first);
        assert_eq!(Event::latest_id(&conn).unwrap(), second);

        let all = Event::list_after(0, 10, &conn).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].kind, EventKind::TagAdded);
        assert_eq!(all[0].name, "library/nginx");
        let rest = Event::list_after(first, 10, &conn).unwrap();
        assert_eq!(rest.len(), 1);
        assert_eq!(rest[0].previous_digest.as_deref(), Some("sha256:a"));
        assert!(Event::list_after(second, 10, &conn).unwrap().is_empty());
        assert_eq!(
            Event::recent_for_repo(repo.id, 1, &conn).unwrap()[0].id,
            second
        );
    }

    #[test]
    fn the_feed_skips_excluded_versions_without_stalling_the_cursor() {
        let pool = pool();
        let conn = pool.get().unwrap();
        let repo = Repo::upsert(
            &RepoEgg {
                registry: "lscr.io".into(),
                name: "linuxserver/sonarr".into(),
            },
            &conn,
        )
        .unwrap();
        let now = chrono::Utc::now();
        let record = |tag: &str| {
            Event::record(
                &NewEvent {
                    repo_id: repo.id,
                    tag: tag.into(),
                    kind: EventKind::TagAdded,
                    digest: None,
                    previous_digest: None,
                },
                now,
                &conn,
            )
            .unwrap()
        };
        // Three excluded events in a row, wider than the page, then a real one.
        let bogus: Vec<u64> = (5..8)
            .map(|n| record(&format!("5.14-2.0.0.5344-ls{n}")))
            .collect();
        let real = record("4.0.19.2979-ls323");
        let latest = record("latest");
        VersionExclusion::add(repo.id, "5.14", None, now, &conn).unwrap();

        let page = Event::feed(0, 2, &conn).unwrap();
        assert_eq!(
            page.iter().map(|e| e.id).collect::<Vec<_>>(),
            vec![real, latest],
            "the page is filled from past the excluded run"
        );
        assert_eq!(Event::feed(0, 1, &conn).unwrap()[0].id, real);
        assert!(Event::feed(latest, 10, &conn).unwrap().is_empty());
        assert_eq!(Event::list_after(0, 10, &conn).unwrap().len(), 5);
        assert_eq!(Event::list_after(0, 10, &conn).unwrap()[0].id, bogus[0]);
        let other = Repo::upsert(
            &RepoEgg {
                registry: "lscr.io".into(),
                name: "linuxserver/radarr".into(),
            },
            &conn,
        )
        .unwrap();
        let elsewhere = Event::record(
            &NewEvent {
                repo_id: other.id,
                tag: "5.14".into(),
                kind: EventKind::TagAdded,
                digest: None,
                previous_digest: None,
            },
            now,
            &conn,
        )
        .unwrap();
        assert_eq!(
            Event::feed(latest, 10, &conn).unwrap()[0].id,
            elsewhere,
            "rules are per repo"
        );
    }
}
