use itertools::Itertools;

use crate::{
    db::repo::{Repo, RepoEgg},
    prelude::*,
};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HydratedTagHistory {
    pub digest: String,
    pub seen_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HydratedTag {
    pub id: u64,
    pub tag: String,
    pub active: bool,
    pub first_seen_at: chrono::DateTime<chrono::Utc>,
    pub history: Vec<HydratedTagHistory>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HydratedRepo {
    pub id: u64,
    pub registry: String,
    pub name: String,
    pub active: bool,
    pub tag: Vec<HydratedTag>,
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

pub fn hydrate(repo: Repo, pool: &PooledConnection<SqliteConnectionManager>) -> HydratedRepo {
    return todo!();
}
