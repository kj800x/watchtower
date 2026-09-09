use itertools::Itertools;

use crate::{
    db::{
        event::Event,
        repo::{Repo, RepoEgg},
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

/// The repo for a registry and image name, hydrated, or 404. This is how a
/// consumer that knows an image (and not our id) reads its tags.
#[get("/api/lookup")]
pub async fn lookup_repo(
    pool: web::Data<Pool<SqliteConnectionManager>>,
    query: web::Query<RepoEgg>,
) -> Result<impl Responder, AppError> {
    let conn = pool.get()?;
    match Repo::get_by_egg(&query.into_inner(), &conn)? {
        Some(repo) => Ok(web::Json(hydrate(repo, &conn))),
        None => Err(AppError::NotFound("repo not found".to_string())),
    }
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

/// The event feed, oldest first from a cursor. See db/event.rs.
#[get("/api/events")]
pub async fn list_events(
    pool: web::Data<Pool<SqliteConnectionManager>>,
    query: web::Query<EventsQuery>,
) -> Result<impl Responder, AppError> {
    let conn = pool.get()?;
    let limit = query.limit.clamp(1, 1000);
    let events = Event::list_after(query.after, limit, &conn)?;
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
pub async fn list_hydrated_repos(pool: web::Data<Pool<SqliteConnectionManager>>) -> impl Responder {
    let conn = pool.get().unwrap();
    let repos: Vec<HydratedRepo> = Repo::all(&conn)
        .unwrap()
        .into_iter()
        .map(|repo| hydrate(repo, &conn))
        .collect_vec();

    web::Json(repos)
}

#[get("/api/hydrated/repo/{id}")]
pub async fn get_hydrated_repo(
    pool: web::Data<Pool<SqliteConnectionManager>>,
    id: web::Path<u64>,
) -> impl Responder {
    let conn = pool.get().unwrap();
    let repo = Repo::get(id.into_inner(), &conn).unwrap();

    if let Some(repo) = repo {
        web::Json(Some(hydrate(repo, &conn)))
    } else {
        web::Json(None)
    }
}

pub fn hydrate(repo: Repo, conn: &PooledConnection<SqliteConnectionManager>) -> HydratedRepo {
    let tag = repo
        .tags(conn)
        .unwrap()
        .into_iter()
        .map(|tag| {
            let history = tag
                .history(conn)
                .unwrap()
                .into_iter()
                .map(|h| HydratedTagHistory {
                    digest: h.digest,
                    first_seen_at: h.first_seen_at,
                    last_seen_at: h.last_seen_at,
                })
                .collect_vec();

            let parsed = crate::poller::classify::parse_version(&tag.tag);
            HydratedTag {
                id: tag.id,
                version: parsed.as_ref().map(|p| p.version.clone()),
                variant: parsed.and_then(|p| p.variant),
                tag: tag.tag,
                active: tag.active,
                first_seen_at: tag.first_seen_at,
                last_seen_at: tag.last_seen_at,
                history,
            }
        })
        .collect_vec();

    let state = repo.state(conn).unwrap();

    HydratedRepo {
        last_checked_at: state.as_ref().and_then(|s| s.last_checked_at),
        last_attempted_at: state.as_ref().and_then(|s| s.last_attempted_at),
        last_error: state.as_ref().and_then(|s| s.last_error.clone()),
        consecutive_failures: state.as_ref().map(|s| s.consecutive_failures).unwrap_or(0),
        id: repo.id,
        registry: repo.registry,
        name: repo.name,
        active: repo.active,
        tag,
    }
}
