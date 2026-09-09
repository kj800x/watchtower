use crate::{
    db::{
        event::{Event, EventKind},
        repo::Repo,
        tag::Tag,
    },
    poller::classify,
    prelude::*,
    web::{
        formatting::{format_relative_time, format_short_digest},
        header::{html_response, page},
    },
};

#[get("/repo/{id}")]
pub async fn repo_detail_page(
    pool: web::Data<Pool<SqliteConnectionManager>>,
    id: web::Path<u64>,
) -> Result<impl Responder, AppError> {
    let conn = pool.get()?;
    let Some(repo) = Repo::get(id.into_inner(), &conn)? else {
        return Err(AppError::NotFound("repo not found".to_string()));
    };

    let title = format!("{}/{}", repo.registry, repo.name);
    let content = html! {
        div id="repo-detail"
            hx-get=(format!("/fragments/repo/{}", repo.id))
            hx-trigger="every 10s"
            hx-swap="morph:innerHTML" {
            (render_repo_detail(&repo, &conn)?)
        }
    };

    Ok(html_response(page(&title, "repos", "repo-detail", content)))
}

#[get("/fragments/repo/{id}")]
pub async fn repo_detail_fragment(
    pool: web::Data<Pool<SqliteConnectionManager>>,
    id: web::Path<u64>,
) -> Result<impl Responder, AppError> {
    let conn = pool.get()?;
    let Some(repo) = Repo::get(id.into_inner(), &conn)? else {
        return Err(AppError::NotFound("repo not found".to_string()));
    };
    Ok(html_response(render_repo_detail(&repo, &conn)?))
}

fn render_repo_detail(
    repo: &Repo,
    conn: &PooledConnection<SqliteConnectionManager>,
) -> AppResult<Markup> {
    let state = repo.state(conn)?;
    let tags = repo.tags(conn)?;

    let last_checked = state.as_ref().and_then(|s| s.last_checked_at);
    let last_attempted = state.as_ref().and_then(|s| s.last_attempted_at);
    let failures = state.as_ref().map(|s| s.consecutive_failures).unwrap_or(0);
    let last_error = state.as_ref().and_then(|s| s.last_error.clone());

    let mut tag_rows = Vec::new();
    for tag in &tags {
        tag_rows.push(render_tag_row(tag, conn)?);
    }
    let events = Event::recent_for_repo(repo.id, 20, conn)?;

    Ok(html! {
        div class="page-heading" {
            h1 {
                (repo.registry) "/" (repo.name)
                " "
                @if repo.active {
                    span class="badge badge-active" { "active" }
                } @else {
                    span class="badge badge-inactive" { "paused" }
                }
            }
            div {
                button class="button button-primary"
                    hx-post=(format!("/api/repo/{}/refresh", repo.id))
                    hx-swap="none"
                    title="Queue an immediate refresh" { "Refresh now" }
                " "
                form class="inline-form" action=(format!("/repo/{}/toggle", repo.id)) method="post" {
                    @if repo.active {
                        button class="button button-danger" type="submit" { "Pause tracking" }
                    } @else {
                        button class="button" type="submit" { "Resume tracking" }
                    }
                }
            }
        }

        @if let Some(error) = &last_error {
            div class="alert alert-danger" {
                strong { "Last refresh failed" }
                @if failures > 1 { " (" (failures) " consecutive failures)" }
                pre { (error) }
            }
        }

        div class="card" {
            div class="state-grid" {
                div class="stat" {
                    div class="stat-label" { "Last refreshed" }
                    div class="stat-value" { (format_relative_time(last_checked)) }
                }
                div class="stat" {
                    div class="stat-label" { "Last attempted" }
                    div class="stat-value" { (format_relative_time(last_attempted)) }
                }
                div class="stat" {
                    div class="stat-label" { "Consecutive failures" }
                    div class="stat-value" { (failures) }
                }
                div class="stat" {
                    div class="stat-label" { "Tags tracked" }
                    div class="stat-value" { (tags.len()) }
                }
            }
        }

        @if !events.is_empty() {
            div class="card" {
                h2 { "Recent events" }
                p class="cell-secondary" { "What changed since the first refresh, newest first. Consumers read the same rows from " code { "/api/events" } "." }
                table class="data-table" {
                    thead { tr { th { "When" } th { "Event" } th { "Tag" } th { "Digest" } } }
                    tbody {
                        @for e in &events {
                            tr {
                                td class="cell-secondary" { (format_relative_time(Some(e.at))) }
                                td {
                                    @match e.kind {
                                        EventKind::TagAdded => { span class="badge badge-active" { "added" } }
                                        EventKind::TagMoved => { span class="badge badge-floating" { "moved" } }
                                        EventKind::TagRemoved => { span class="badge badge-error" { "removed" } }
                                    }
                                }
                                td { code { (e.tag) } }
                                td {
                                    @if let Some(from) = &e.previous_digest {
                                        code class="digest" title=(from) { (format_short_digest(from)) } " → "
                                    }
                                    @if let Some(d) = &e.digest {
                                        code class="digest" title=(d) { (format_short_digest(d)) }
                                    } @else {
                                        span class="cell-secondary" { "—" }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        @if tags.is_empty() {
            div class="empty-state" {
                h2 { "No tags yet" }
                p { "This repo hasn't been refreshed yet. Use \"Refresh now\" to fetch it immediately." }
            }
        } @else {
            table class="data-table" {
                thead {
                    tr {
                        th { "Tag" }
                        th { "Class" }
                        th { "Digest" }
                        th { "Digest since" }
                        th { "Last verified" }
                        th { "First seen" }
                        th { "History" }
                    }
                }
                tbody {
                    @for row in tag_rows { (row) }
                }
            }
        }
    })
}

fn render_tag_row(
    tag: &Tag,
    conn: &PooledConnection<SqliteConnectionManager>,
) -> AppResult<Markup> {
    let history = tag.history(conn)?;
    let latest = history.first();

    Ok(html! {
        tr {
            td {
                code { (tag.tag) }
                @if !tag.active {
                    " " span class="badge badge-error" title="No longer present in the registry's tag list" { "gone" }
                }
            }
            td {
                @if classify::is_immutable(&tag.tag) {
                    span class="badge badge-immutable" { "immutable" }
                } @else {
                    span class="badge badge-floating" { "floating" }
                }
                @if let Some(parsed) = classify::parse_version(&tag.tag) {
                    " " span class="cell-secondary" title="Version and variant as consumers read this tag" {
                        (parsed.version)
                        @if let Some(variant) = &parsed.variant { " · " (variant) }
                    }
                }
            }
            td {
                @if let Some(h) = latest {
                    code class="digest" title=(h.digest) { (format_short_digest(&h.digest)) }
                } @else {
                    span class="cell-secondary" { "not yet fetched" }
                }
            }
            td class="cell-secondary" { (format_relative_time(latest.map(|h| h.first_seen_at))) }
            td class="cell-secondary" { (format_relative_time(latest.map(|h| h.last_seen_at))) }
            td class="cell-secondary" { (format_relative_time(Some(tag.first_seen_at))) }
            td {
                @if history.len() > 1 {
                    details class="history-details" {
                        summary { ((history.len() - 1)) " earlier " @if history.len() == 2 { "digest" } @else { "digests" } }
                        ul {
                            @for h in history.iter().skip(1) {
                                li {
                                    code class="digest" title=(h.digest) { (format_short_digest(&h.digest)) }
                                    " — " (format_relative_time(Some(h.first_seen_at)))
                                    " to " (format_relative_time(Some(h.last_seen_at)))
                                }
                            }
                        }
                    }
                } @else {
                    span class="cell-secondary" { "—" }
                }
            }
        }
    })
}
