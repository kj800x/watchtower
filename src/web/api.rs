use itertools::Itertools;

use crate::{
    db::repo::{Repo, RepoEgg},
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

            HydratedTag {
                id: tag.id,
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
