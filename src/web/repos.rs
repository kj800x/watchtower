use std::collections::HashMap;

use crate::{
    db::repo::{Repo, RepoEgg},
    prelude::*,
    web::{
        formatting::format_relative_time,
        header::{html_response, page},
    },
};

#[get("/")]
pub async fn repos_page(
    pool: web::Data<Pool<SqliteConnectionManager>>,
) -> Result<impl Responder, AppError> {
    let conn = pool.get()?;

    let content = html! {
        div class="page-heading" {
            h1 { "Tracked repos" }
            form class="add-repo-form" action="/repo" method="post" {
                input type="text" name="registry" placeholder="registry (e.g. ghcr.io)" required;
                input type="text" name="name" placeholder="repository (e.g. qdm12/gluetun)" required;
                button class="button button-primary" type="submit" { "Track repo" }
            }
        }
        div id="repo-table"
            hx-get="/fragments/repo-rows"
            hx-trigger="every 5s"
            hx-swap="morph:innerHTML" {
            (render_repo_table(&conn)?)
        }
    };

    Ok(html_response(page("Repos", "repos", "repos", content)))
}

#[get("/fragments/repo-rows")]
pub async fn repo_rows_fragment(
    pool: web::Data<Pool<SqliteConnectionManager>>,
) -> Result<impl Responder, AppError> {
    let conn = pool.get()?;
    Ok(html_response(render_repo_table(&conn)?))
}

fn render_repo_table(conn: &PooledConnection<SqliteConnectionManager>) -> AppResult<Markup> {
    let repos = Repo::all(conn)?;

    if repos.is_empty() {
        return Ok(html! {
            div class="empty-state" {
                h2 { "No repos tracked yet" }
                p { "Add a registry + repository pair above to start tracking its tags." }
            }
        });
    }

    let mut rows = Vec::new();
    for repo in repos {
        rows.push(render_repo_row(&repo, conn)?);
    }

    Ok(html! {
        table class="data-table" {
            thead {
                tr {
                    th { "Repo" }
                    th { "Status" }
                    th { "Tags" }
                    th { "Last refreshed" }
                    th { "Health" }
                    th {}
                }
            }
            tbody {
                @for row in rows { (row) }
            }
        }
    })
}

fn render_repo_row(
    repo: &Repo,
    conn: &PooledConnection<SqliteConnectionManager>,
) -> AppResult<Markup> {
    let state = repo.state(conn)?;
    let (active_tags, total_tags) = repo.tag_counts(conn)?;

    let last_checked = state.as_ref().and_then(|s| s.last_checked_at);
    let failures = state.as_ref().map(|s| s.consecutive_failures).unwrap_or(0);

    let (dot_class, health) = if failures > 0 {
        ("status-error", format!("{} consecutive failures", failures))
    } else if last_checked.is_some() {
        ("status-ok", "ok".to_string())
    } else {
        ("status-pending", "awaiting first refresh".to_string())
    };

    Ok(html! {
        tr {
            td {
                a href=(format!("/repo/{}", repo.id)) { (repo.name) }
                div class="cell-secondary" { (repo.registry) }
            }
            td {
                @if repo.active {
                    span class="badge badge-active" { "active" }
                } @else {
                    span class="badge badge-inactive" { "paused" }
                }
            }
            td {
                (active_tags)
                @if total_tags > active_tags {
                    span class="cell-secondary" { " (+" ((total_tags - active_tags)) " gone)" }
                }
            }
            td class="cell-secondary" { (format_relative_time(last_checked)) }
            td class="cell-secondary" {
                span class=(format!("status-dot {}", dot_class)) {}
                (health)
            }
            td class="cell-actions" {
                button class="button button-small"
                    hx-post=(format!("/api/repo/{}/refresh", repo.id))
                    hx-swap="none"
                    title="Queue an immediate refresh" { "Refresh now" }
                " "
                form class="inline-form" action=(format!("/repo/{}/toggle", repo.id)) method="post" {
                    @if repo.active {
                        button class="button button-small button-danger" type="submit" { "Pause" }
                    } @else {
                        button class="button button-small" type="submit" { "Resume" }
                    }
                }
            }
        }
    })
}

#[post("/repo")]
pub async fn create_repo_form(
    pool: web::Data<Pool<SqliteConnectionManager>>,
    form: web::Form<HashMap<String, String>>,
) -> Result<impl Responder, AppError> {
    let registry = form.get("registry").map(|s| s.trim()).unwrap_or_default();
    let name = form.get("name").map(|s| s.trim()).unwrap_or_default();

    if registry.is_empty() || name.is_empty() {
        return Err(AppError::InvalidInput(
            "registry and name are required".to_string(),
        ));
    }

    let conn = pool.get()?;
    let repo = Repo::upsert(
        &RepoEgg {
            registry: registry.to_string(),
            name: name.to_string(),
        },
        &conn,
    )?;

    Ok(HttpResponse::SeeOther()
        .append_header(("Location", format!("/repo/{}", repo.id)))
        .finish())
}

#[post("/repo/{id}/toggle")]
pub async fn toggle_repo_active(
    pool: web::Data<Pool<SqliteConnectionManager>>,
    id: web::Path<u64>,
    req: actix_web::HttpRequest,
) -> Result<impl Responder, AppError> {
    let conn = pool.get()?;
    let Some(repo) = Repo::get(id.into_inner(), &conn)? else {
        return Err(AppError::NotFound("repo not found".to_string()));
    };

    repo.save(!repo.active, &conn)?;

    let back = req
        .headers()
        .get("Referer")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("/")
        .to_string();
    Ok(HttpResponse::SeeOther()
        .append_header(("Location", back))
        .finish())
}
