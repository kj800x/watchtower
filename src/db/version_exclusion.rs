//! Versions an admin has ruled out of version matching.
//!
//! Some registries carry tags that parse as a perfectly good version but
//! name no release of the image: linuxserver/qbittorrent has a `20.04.1`
//! (the Ubuntu base), linuxserver/sonarr has `5.14` (the mono version of
//! a 2016 build). Nothing about the tag itself tells a classifier that, so
//! a person marks the version excluded here and every tag naming it is
//! left out of the lookup responses and the event feed that consumers
//! (cicd) resolve ranges over. The tags are still tracked and shown in the
//! UI; only the API omits them.
//!
//! A rule is a version as `classify::parse_version` reads it (`20.04.1`),
//! or a prefix with a trailing `*` (`14.3.*`) for a whole family of them.

use std::collections::HashMap;

use r2d2::PooledConnection;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::{
    error::{AppError, AppResult},
    poller::classify,
};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct VersionExclusion {
    pub id: u64,
    pub repo_id: u64,
    /// The rule as typed: an exact version or a `prefix.*` pattern.
    pub version: String,
    pub note: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// The rules of one repo, ready to test tags against.
#[derive(Debug, Clone, Default)]
pub struct ExclusionSet {
    rules: Vec<VersionExclusion>,
}

impl ExclusionSet {
    pub fn new(rules: Vec<VersionExclusion>) -> Self {
        Self { rules }
    }

    /// The rule that excludes `tag`, if any. A tag that names no version
    /// is never excluded: nothing matches it as a version anyway.
    pub fn rule_for(&self, tag: &str) -> Option<&VersionExclusion> {
        let parsed = classify::parse_version(tag)?;
        self.rules.iter().find(|r| r.matches(&parsed.version))
    }

    pub fn excludes(&self, tag: &str) -> bool {
        self.rule_for(tag).is_some()
    }
}

/// Check that a rule is a dotted numeric version, optionally ending in
/// `.*`, and return it trimmed. Anything else would never match a parsed
/// version and would only confuse whoever reads the list later.
pub fn validate_rule(rule: &str) -> AppResult<String> {
    let rule = rule.trim();
    let core = rule.strip_suffix(".*").unwrap_or(rule);
    let numeric = |c: &str| !c.is_empty() && c.bytes().all(|b| b.is_ascii_digit());
    if core.is_empty() || !core.split('.').all(numeric) {
        return Err(AppError::InvalidInput(format!(
            "'{rule}' is not a version: expected digits and dots like 20.04.1, or a prefix like 14.3.*"
        )));
    }
    Ok(rule.to_string())
}

impl VersionExclusion {
    fn from_row(row: &rusqlite::Row) -> AppResult<Self> {
        Ok(VersionExclusion {
            id: row.get(0)?,
            repo_id: row.get(1)?,
            version: row.get(2)?,
            note: row.get(3)?,
            created_at: chrono::DateTime::from_timestamp(row.get::<usize, i64>(4)?, 0)
                .unwrap_or_default(),
        })
    }

    /// Whether this rule covers `version` (as parsed from a tag).
    pub fn matches(&self, version: &str) -> bool {
        match self.version.strip_suffix('*') {
            Some(prefix) => version.starts_with(prefix),
            None => self.version == version,
        }
    }

    pub fn for_repo(
        repo_id: u64,
        conn: &PooledConnection<SqliteConnectionManager>,
    ) -> AppResult<Vec<Self>> {
        let mut stmt = conn.prepare(
            "SELECT id, repo_id, version, note, created_at FROM version_exclusion
             WHERE repo_id = ?1 ORDER BY created_at DESC, id DESC",
        )?;
        let mut rows = stmt.query(params![repo_id])?;
        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            out.push(Self::from_row(row)?);
        }
        Ok(out)
    }

    pub fn set_for_repo(
        repo_id: u64,
        conn: &PooledConnection<SqliteConnectionManager>,
    ) -> AppResult<ExclusionSet> {
        Ok(ExclusionSet::new(Self::for_repo(repo_id, conn)?))
    }

    /// Every repo's rules, keyed by repo id, for filtering a feed that
    /// spans repos in one pass.
    pub fn all_by_repo(
        conn: &PooledConnection<SqliteConnectionManager>,
    ) -> AppResult<HashMap<u64, ExclusionSet>> {
        let mut stmt = conn.prepare(
            "SELECT id, repo_id, version, note, created_at FROM version_exclusion ORDER BY id",
        )?;
        let mut rows = stmt.query(params![])?;
        let mut grouped: HashMap<u64, Vec<Self>> = HashMap::new();
        while let Some(row) = rows.next()? {
            let rule = Self::from_row(row)?;
            grouped.entry(rule.repo_id).or_default().push(rule);
        }
        Ok(grouped
            .into_iter()
            .map(|(repo_id, rules)| (repo_id, ExclusionSet::new(rules)))
            .collect())
    }

    pub fn get(
        id: u64,
        conn: &PooledConnection<SqliteConnectionManager>,
    ) -> AppResult<Option<Self>> {
        conn.prepare(
            "SELECT id, repo_id, version, note, created_at FROM version_exclusion WHERE id = ?1",
        )?
        .query_row(params![id], |row| Ok(Self::from_row(row)))
        .optional()?
        .transpose()
    }

    /// Add a rule, or update the note of an existing one for the same
    /// version. The rule is validated first; see [`validate_rule`].
    pub fn add(
        repo_id: u64,
        version: &str,
        note: Option<&str>,
        at: chrono::DateTime<chrono::Utc>,
        conn: &PooledConnection<SqliteConnectionManager>,
    ) -> AppResult<Self> {
        let version = validate_rule(version)?;
        let note = note.map(str::trim).filter(|n| !n.is_empty());
        conn.prepare(
            "INSERT INTO version_exclusion (repo_id, version, note, created_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(repo_id, version) DO UPDATE SET note = coalesce(?3, note)
             RETURNING id, repo_id, version, note, created_at",
        )?
        .query_row(params![repo_id, version, note, at.timestamp()], |row| {
            Ok(Self::from_row(row))
        })?
    }

    /// Remove a rule. Returns whether one existed.
    pub fn remove(id: u64, conn: &PooledConnection<SqliteConnectionManager>) -> AppResult<bool> {
        Ok(conn.execute("DELETE FROM version_exclusion WHERE id = ?1", params![id])? > 0)
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

    fn rule(version: &str) -> VersionExclusion {
        VersionExclusion {
            id: 0,
            repo_id: 0,
            version: version.to_string(),
            note: None,
            created_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn exact_rules_match_the_parsed_version_of_a_tag() {
        let set = ExclusionSet::new(vec![rule("5.14"), rule("20.04.1")]);
        assert!(set.excludes("5.14"));
        assert!(
            set.excludes("5.14-2.0.0.5344-ls9"),
            "variants share the version"
        );
        assert!(set.excludes("v20.04.1"));
        assert!(!set.excludes("5.14.1"));
        assert!(!set.excludes("4.0.19.2979-ls323"));
        assert!(!set.excludes("latest"));
        assert!(
            !set.excludes("14.3.9.99202110311443-7435-01519b5e7ubuntu20.04.1-ls166"),
            "the version is what is before the dash, not what the variant mentions"
        );
    }

    #[test]
    fn wildcard_rules_match_a_prefix() {
        let set = ExclusionSet::new(vec![rule("14.3.*")]);
        assert!(set.excludes("14.3.9.99202110311443-7435-01519b5e7ubuntu20.04.1-ls166"));
        assert!(set.excludes("14.3.0"));
        assert!(!set.excludes("14.30.1"));
        assert!(
            !set.excludes("14.3"),
            "a prefix rule needs the dot after it"
        );
        assert!(ExclusionSet::new(vec![rule("14.*")]).excludes("14.3"));
        assert!(!ExclusionSet::new(vec![rule("14.*")]).excludes("140.3"));
    }

    #[test]
    fn rules_are_versions_or_dotted_prefixes() {
        assert_eq!(validate_rule(" 20.04.1 ").unwrap(), "20.04.1");
        assert_eq!(validate_rule("14.3.*").unwrap(), "14.3.*");
        assert_eq!(validate_rule("5").unwrap(), "5");
        for bad in [
            "",
            "*",
            ".*",
            "v1.2",
            "1.2.3-alpine",
            "1..2",
            "14.3*",
            "abc",
        ] {
            assert!(validate_rule(bad).is_err(), "{bad:?} should be rejected");
        }
    }

    #[test]
    fn rules_are_stored_per_repo_and_deduplicated_by_version() {
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
        let other = Repo::upsert(
            &RepoEgg {
                registry: "lscr.io".into(),
                name: "linuxserver/radarr".into(),
            },
            &conn,
        )
        .unwrap();
        let now = chrono::Utc::now();

        let first =
            VersionExclusion::add(repo.id, "5.14", Some("mono version"), now, &conn).unwrap();
        let again = VersionExclusion::add(repo.id, " 5.14", None, now, &conn).unwrap();
        assert_eq!(first.id, again.id);
        assert_eq!(
            again.note.as_deref(),
            Some("mono version"),
            "a blank note keeps the old one"
        );
        VersionExclusion::add(other.id, "5.14", None, now, &conn).unwrap();
        assert!(VersionExclusion::add(repo.id, "nope", None, now, &conn).is_err());

        assert_eq!(VersionExclusion::for_repo(repo.id, &conn).unwrap().len(), 1);
        let by_repo = VersionExclusion::all_by_repo(&conn).unwrap();
        assert_eq!(by_repo.len(), 2);
        assert!(by_repo[&repo.id].excludes("5.14-2.0.0.5344-ls9"));

        assert!(VersionExclusion::remove(first.id, &conn).unwrap());
        assert!(!VersionExclusion::remove(first.id, &conn).unwrap());
        assert!(VersionExclusion::get(first.id, &conn).unwrap().is_none());
        assert!(
            VersionExclusion::for_repo(repo.id, &conn)
                .unwrap()
                .is_empty()
        );
    }
}
