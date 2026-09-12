use itertools::Itertools;

use crate::{
    db::{
        event::Event,
        repo::{Repo, RepoEgg},
        version_exclusion::{ExclusionSet, VersionExclusion},
    },
    error::AppError,
    poller::RefreshRequester,
    prelude::*,
};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HydratedTagHistory {
    pub digest: String,
    pub first_seen_at: chrono::DateTime<chrono::Utc>,
    pub last_seen_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HydratedTag {
    pub id: u64,
    pub tag: String,
    pub active: bool,
    /// The version the tag names (`15.11`, `1.27.3`), when it names one;
    /// see `classify::parse_version`.
    pub version: Option<String>,
    /// What follows the first `-` of a versioned tag (`alpine`, `rc1`).
    pub variant: Option<String>,
    /// Build metadata split off the version (`ubu2604.ls48` for
    /// `12.0ubu2604-ls48`): what linuxserver.io glues on and its build
    /// number. Distinguishes rebuilds of one version; never a variant.
    pub build: Option<String>,
    /// Whether this tag's version is excluded from version matching, by an
    /// admin's rule or a publisher rule (`classify::publisher_alias`). Excluded tags are left out of responses unless asked for
    /// with `include_excluded=true`, so this is false in the default view.
    pub excluded: bool,
    pub first_seen_at: chrono::DateTime<chrono::Utc>,
    pub last_seen_at: chrono::DateTime<chrono::Utc>,
    pub history: Vec<HydratedTagHistory>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HydratedRepo {
    pub id: u64,
    pub registry: String,
    pub name: String,
    pub active: bool,
    pub tag: Vec<HydratedTag>,
    /// The admin's version exclusions for this repo; see
    /// `db/version_exclusion.rs`.
    pub exclusions: Vec<VersionExclusion>,
    pub last_checked_at: Option<chrono::DateTime<chrono::Utc>>,
    pub last_attempted_at: Option<chrono::DateTime<chrono::Utc>>,
    pub last_error: Option<String>,
    pub consecutive_failures: u32,
}

#[get("/api/repo")]
pub async fn list_repos(pool: web::Data<Pool<SqliteConnectionManager>>) -> impl Responder {
    let conn = pool.get().unwrap();
    let repos: Vec<Repo> = Repo::all(&conn).unwrap();

    web::Json(repos)
}

#[get("/api/repo/{id}")]
pub async fn get_repo(
    pool: web::Data<Pool<SqliteConnectionManager>>,
    id: web::Path<u64>,
) -> impl Responder {
    let conn = pool.get().unwrap();
    let repo = Repo::get(id.into_inner(), &conn).unwrap();

    web::Json(repo)
}

/// Options for a hydrated response.
#[derive(Debug, Deserialize)]
pub struct HydrateQuery {
    /// Also return tags whose version an admin has excluded, flagged with
    /// `excluded: true`. Off by default: consumers resolving versions must
    /// not see them.
    #[serde(default)]
    pub include_excluded: bool,
}

#[derive(Debug, Deserialize)]
pub struct LookupQuery {
    pub registry: String,
    pub name: String,
    #[serde(default)]
    pub include_excluded: bool,
}

/// The repo for a registry and image name, hydrated, or 404. This is how a
/// consumer that knows an image (and not our id) reads its tags.
#[get("/api/lookup")]
pub async fn lookup_repo(
    pool: web::Data<Pool<SqliteConnectionManager>>,
    query: web::Query<LookupQuery>,
) -> Result<impl Responder, AppError> {
    let conn = pool.get()?;
    let query = query.into_inner();
    let egg = RepoEgg {
        registry: query.registry,
        name: query.name,
    };
    match Repo::get_by_egg(&egg, &conn)? {
        Some(repo) => Ok(web::Json(hydrate(repo, query.include_excluded, &conn)?)),
        None => Err(AppError::NotFound("repo not found".to_string())),
    }
}

/// The admin's version exclusions for a repo.
#[get("/api/repo/{id}/exclusions")]
pub async fn list_exclusions(
    pool: web::Data<Pool<SqliteConnectionManager>>,
    id: web::Path<u64>,
) -> Result<impl Responder, AppError> {
    let conn = pool.get()?;
    let id = id.into_inner();
    if Repo::get(id, &conn)?.is_none() {
        return Err(AppError::NotFound("repo not found".to_string()));
    }
    Ok(web::Json(VersionExclusion::for_repo(id, &conn)?))
}

#[derive(Debug, Deserialize)]
pub struct NewExclusion {
    /// A version as watchtower reads it (`20.04.1`) or a prefix (`14.3.*`).
    pub version: String,
    pub note: Option<String>,
}

/// Exclude a version from version matching for a repo. Adding a version
/// that is already excluded updates its note.
#[post("/api/repo/{id}/exclusions")]
pub async fn add_exclusion(
    pool: web::Data<Pool<SqliteConnectionManager>>,
    id: web::Path<u64>,
    body: web::Json<NewExclusion>,
) -> Result<impl Responder, AppError> {
    let conn = pool.get()?;
    let id = id.into_inner();
    if Repo::get(id, &conn)?.is_none() {
        return Err(AppError::NotFound("repo not found".to_string()));
    }
    let body = body.into_inner();
    let rule = VersionExclusion::add(
        id,
        &body.version,
        body.note.as_deref(),
        chrono::Utc::now(),
        &conn,
    )?;
    Ok(web::Json(rule))
}

#[delete("/api/exclusion/{id}")]
pub async fn remove_exclusion(
    pool: web::Data<Pool<SqliteConnectionManager>>,
    id: web::Path<u64>,
) -> Result<impl Responder, AppError> {
    let conn = pool.get()?;
    if !VersionExclusion::remove(id.into_inner(), &conn)? {
        return Err(AppError::NotFound("exclusion not found".to_string()));
    }
    Ok(HttpResponse::NoContent().finish())
}

#[derive(Debug, Deserialize)]
pub struct EventsQuery {
    /// Return events with an id greater than this. 0 (the default) means
    /// from the beginning; `latest_id` from an earlier response means "from
    /// now".
    #[serde(default)]
    pub after: u64,
    #[serde(default = "default_event_limit")]
    pub limit: usize,
}

fn default_event_limit() -> usize {
    200
}

#[derive(Debug, Serialize)]
pub struct EventsPage {
    pub events: Vec<Event>,
    /// The newest event id that exists, whether or not it is in this page.
    /// A consumer whose cursor equals this has nothing more to read.
    pub latest_id: u64,
}

/// The event feed, oldest first from a cursor, minus events of excluded
/// versions. See db/event.rs.
#[get("/api/events")]
pub async fn list_events(
    pool: web::Data<Pool<SqliteConnectionManager>>,
    query: web::Query<EventsQuery>,
) -> Result<impl Responder, AppError> {
    let conn = pool.get()?;
    let limit = query.limit.clamp(1, 1000);
    let events = Event::feed(query.after, limit, &conn)?;
    let latest_id = Event::latest_id(&conn)?;
    Ok(web::Json(EventsPage { events, latest_id }))
}

#[post("/api/repo")]
pub async fn create_repo(
    pool: web::Data<Pool<SqliteConnectionManager>>,
    repo: web::Json<RepoEgg>,
) -> impl Responder {
    let conn = pool.get().unwrap();
    let repo = Repo::upsert(&repo.into_inner(), &conn).unwrap();

    web::Json(repo)
}

#[post("/api/repo/{id}/active")]
pub async fn set_repo_active(
    pool: web::Data<Pool<SqliteConnectionManager>>,
    id: web::Path<u64>,
    active: web::Json<bool>,
) -> impl Responder {
    let conn = pool.get().unwrap();
    let repo = Repo::get(id.into_inner(), &conn).unwrap();

    if let Some(repo) = repo {
        repo.save(active.into_inner(), &conn).unwrap();
        return Ok(());
    } else {
        return Err(AppError::NotFound("repo not found".to_string()));
    }
}

#[post("/api/repo/{id}/refresh")]
pub async fn refresh_repo_now(
    pool: web::Data<Pool<SqliteConnectionManager>>,
    refresh: web::Data<RefreshRequester>,
    id: web::Path<u64>,
) -> Result<impl Responder, AppError> {
    let conn = pool.get().map_err(AppError::from)?;
    let id = id.into_inner();

    if Repo::get(id, &conn)?.is_none() {
        return Err(AppError::NotFound("repo not found".to_string()));
    }

    refresh.request(id);
    Ok(HttpResponse::Accepted().json(serde_json::json!({ "refresh_requested": id })))
}

#[get("/api/hydrated/repo")]
pub async fn list_hydrated_repos(
    pool: web::Data<Pool<SqliteConnectionManager>>,
    query: web::Query<HydrateQuery>,
) -> Result<impl Responder, AppError> {
    let conn = pool.get()?;
    let repos: Vec<HydratedRepo> = Repo::all(&conn)?
        .into_iter()
        .map(|repo| hydrate(repo, query.include_excluded, &conn))
        .collect::<AppResult<_>>()?;

    Ok(web::Json(repos))
}

#[get("/api/hydrated/repo/{id}")]
pub async fn get_hydrated_repo(
    pool: web::Data<Pool<SqliteConnectionManager>>,
    id: web::Path<u64>,
    query: web::Query<HydrateQuery>,
) -> Result<impl Responder, AppError> {
    let conn = pool.get()?;
    let repo = Repo::get(id.into_inner(), &conn)?;

    Ok(match repo {
        Some(repo) => web::Json(Some(hydrate(repo, query.include_excluded, &conn)?)),
        None => web::Json(None),
    })
}

/// A repo with its tags as consumers read them. Tags of an excluded
/// version are left out unless `include_excluded`, in which case they are
/// returned flagged.
pub fn hydrate(
    repo: Repo,
    include_excluded: bool,
    conn: &PooledConnection<SqliteConnectionManager>,
) -> AppResult<HydratedRepo> {
    let exclusions = VersionExclusion::for_repo(repo.id, conn)?;
    let excluded_set = ExclusionSet::new(&repo.name, exclusions.clone());
    let mut tag = Vec::new();
    for t in repo.tags(conn)? {
        let excluded = excluded_set.excludes(&t.tag);
        if excluded && !include_excluded {
            continue;
        }
        let history = t
            .history(conn)?
            .into_iter()
            .map(|h| HydratedTagHistory {
                digest: h.digest,
                first_seen_at: h.first_seen_at,
                last_seen_at: h.last_seen_at,
            })
            .collect_vec();

        let parsed = crate::poller::classify::parse_version(&t.tag);
        tag.push(HydratedTag {
            id: t.id,
            version: parsed.as_ref().map(|p| p.version.clone()),
            variant: parsed.as_ref().and_then(|p| p.variant.clone()),
            build: parsed.and_then(|p| p.build),
            excluded,
            tag: t.tag,
            active: t.active,
            first_seen_at: t.first_seen_at,
            last_seen_at: t.last_seen_at,
            history,
        });
    }

    let state = repo.state(conn)?;

    Ok(HydratedRepo {
        last_checked_at: state.as_ref().and_then(|s| s.last_checked_at),
        last_attempted_at: state.as_ref().and_then(|s| s.last_attempted_at),
        last_error: state.as_ref().and_then(|s| s.last_error.clone()),
        consecutive_failures: state.as_ref().map(|s| s.consecutive_failures).unwrap_or(0),
        id: repo.id,
        registry: repo.registry,
        name: repo.name,
        active: repo.active,
        tag,
        exclusions,
    })
}
