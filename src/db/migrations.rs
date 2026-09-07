use crate::prelude::*;
use indoc::indoc;

pub fn migrate(mut conn: PooledConnection<SqliteConnectionManager>) -> AppResult<()> {
    let migrations: Migrations = Migrations::new(vec![
        M::up(indoc! { r#"
          CREATE TABLE repo (
              id INTEGER PRIMARY KEY NOT NULL,
              registry TEXT NOT NULL,
              name TEXT NOT NULL,
              active BOOLEAN NOT NULL DEFAULT TRUE,
              UNIQUE (registry, name)
          );

          CREATE TABLE tag (
              id INTEGER PRIMARY KEY NOT NULL,
              repo_id INTEGER NOT NULL,
              tag TEXT NOT NULL,
              active BOOLEAN NOT NULL DEFAULT TRUE,
              first_seen_at INTEGER NOT NULL,
              last_seen_at INTEGER NOT NULL,
              FOREIGN KEY(repo_id) REFERENCES repo(id),
              UNIQUE (repo_id, tag)
          );

          -- Interval-style rows: one row per distinct digest a tag has pointed
          -- at, with the window [first_seen_at, last_seen_at] over which we
          -- observed it. Re-observing the same digest bumps last_seen_at on
          -- the latest row; a changed digest appends a new row.
          CREATE TABLE tag_history (
              tag_id INTEGER NOT NULL,
              digest TEXT NOT NULL,
              first_seen_at INTEGER NOT NULL,
              last_seen_at INTEGER NOT NULL,
              FOREIGN KEY(tag_id) REFERENCES tag(id)
          );

          CREATE TABLE repo_state (
              repo_id INTEGER PRIMARY KEY NOT NULL,
              last_checked_at INTEGER,
              last_attempted_at INTEGER,
              last_error TEXT,
              consecutive_failures INTEGER NOT NULL DEFAULT 0,
              FOREIGN KEY(repo_id) REFERENCES repo(id)
          );

          CREATE INDEX tag_history_tag_id_first_seen_at
              ON tag_history (tag_id, first_seen_at DESC);
      "#}),
        // The event feed: append-only, read by cursor (id). See db/event.rs.
        M::up(indoc! { r#"
          CREATE TABLE event (
              id INTEGER PRIMARY KEY NOT NULL,
              repo_id INTEGER NOT NULL,
              tag TEXT NOT NULL,
              kind TEXT NOT NULL,
              digest TEXT,
              previous_digest TEXT,
              at INTEGER NOT NULL,
              FOREIGN KEY(repo_id) REFERENCES repo(id)
          );
          CREATE INDEX event_repo_id_id ON event (repo_id, id DESC);
      "#}),
    ]);

    conn.pragma_update_and_check(None, "journal_mode", "WAL", |_| Ok(()))?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    migrations
        .to_latest(&mut conn)
        .map_err(|e| AppError::DatabaseMigration(e.to_string()))?;
    Ok(())
}
