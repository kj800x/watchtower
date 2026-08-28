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
              FOREIGN KEY(repo_id) REFERENCES repo(id),
              UNIQUE (repo_id, tag)
          );

          CREATE TABLE tag_history (
              tag_id INTEGER NOT NULL,
              digest TEXT NOT NULL,
              seen_at INTEGER NOT NULL,
              FOREIGN KEY(tag_id) REFERENCES tag(id)
          );

          CREATE TABLE repo_state (
              repo_id INTEGER PRIMARY KEY NOT NULL,
              last_checked_at INTEGER,
              FOREIGN KEY(repo_id) REFERENCES repo(id)
          );

          CREATE INDEX tag_history_tag_id_seen_at
              ON tag_history (tag_id, seen_at DESC);
      "#}),
        // M::up(indoc! { r#"
        //   ALTER TABLE foobar ADD COLUMN barfoo INTEGER;
        // "#}),
    ]);

    conn.pragma_update_and_check(None, "journal_mode", "WAL", |_| Ok(()))?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    migrations
        .to_latest(&mut conn)
        .map_err(|e| AppError::DatabaseMigration(e.to_string()))?;
    Ok(())
}
