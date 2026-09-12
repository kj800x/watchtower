use std::collections::HashMap;

use crate::{
    db::{
        event::{Event, EventKind},
        repo::Repo,
        tag::Tag,
        version_exclusion::{ExclusionSet, VersionExclusion},
    },
    poller::classify,
    prelude::*,
    web::{
        formatting::{format_relative_time, format_short_digest},
        header::{html_response, page},
    },
};

/// The repo page: a static heading and the exclusion rules (both change
/// only through forms on this page, which reload it), then the refresh
/// state, events and tags, which the poller changes and which re-render
/// every few seconds.
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
        (render_heading(&repo))
        (render_exclusions(&repo, &conn)?)
        div id="repo-detail"
            hx-get=(format!("/fragments/repo/{}", repo.id))
            hx-trigger="every 10s"
            hx-swap="morph:innerHTML" {
            (render_repo_detail(&repo, &conn)?)
        }
    };

    Ok(html_response(page(&title, "repos", "repo-detail", content)))
}

fn render_heading(repo: &Repo) -> Markup {
    html! {
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
    }
}

/// Client-side shape of a rule; the server validates the same way.
const RULE_PATTERN: &str = r"[0-9]+(\.[0-9]+)*(\.\*)?";

/// The admin's version exclusions: the rules in force, and a form to add
/// one by hand (the tag table has a button per version too).
fn render_exclusions(
    repo: &Repo,
    conn: &PooledConnection<SqliteConnectionManager>,
) -> AppResult<Markup> {
    let rules = VersionExclusion::for_repo(repo.id, conn)?;

    Ok(html! {
        div class="card" id="exclusions" {
            h2 { "Excluded versions" }
            p class="cell-secondary" {
                "Versions that look real but aren't releases of this image. Tags naming them stay tracked here but are left out of "
                code { "/api/lookup" } " and " code { "/api/events" }
                ", so consumers never pick them. A trailing " code { ".*" } " excludes a whole family."
            }
            @if let Some(why) = classify::publisher_alias(&repo.name, "0.0.0") {
                p class="cell-secondary" { "Built-in for this publisher: " (why) "." }
            }
            @if !rules.is_empty() {
                table class="data-table exclusion-table" {
                    thead { tr { th { "Version" } th { "Note" } th { "Added" } th {} } }
                    tbody {
                        @for rule in &rules {
                            tr {
                                td { code { (rule.version) } }
                                td {
                                    @if let Some(note) = &rule.note { (note) }
                                    @else { span class="cell-secondary" { "—" } }
                                }
                                td class="cell-secondary" { (format_relative_time(Some(rule.created_at))) }
                                td class="cell-actions" {
                                    form class="inline-form" action=(format!("/exclusion/{}/remove", rule.id)) method="post" {
                                        button class="button button-small" type="submit"
                                            title="Let this version be matched again" { "Include again" }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            form class="add-repo-form exclusion-form" action=(format!("/repo/{}/exclusions", repo.id)) method="post" {
                input type="text" name="version" placeholder="version (e.g. 20.04.1 or 14.3.*)"
                    pattern=(RULE_PATTERN) title="Digits and dots, optionally ending in .*" required;
                input type="text" name="note" placeholder="note (optional)" class="exclusion-note";
                button class="button" type="submit" { "Exclude version" }
            }
        }
    })
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

    let exclusions = VersionExclusion::set_for_repo(repo, conn)?;
    let excluded_count = tags.iter().filter(|t| exclusions.excludes(&t.tag)).count();

    let mut tag_rows = Vec::new();
    for tag in &tags {
        tag_rows.push(render_tag_row(repo, tag, &exclusions, conn)?);
    }
    let events = Event::recent_for_repo(repo.id, 20, conn)?;

    Ok(html! {
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
                    div class="stat-value" {
                        (tags.len())
                        @if excluded_count > 0 {
                            " " span class="cell-secondary" { "(" (excluded_count) " excluded)" }
                        }
                    }
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
                                td {
                                    code { (e.tag) }
                                    @if exclusions.excludes(&e.tag) {
                                        " " span class="badge badge-excluded" title="Not in /api/events: this version is excluded" { "excluded" }
                                    }
                                }
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
                        th {}
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
    repo: &Repo,
    tag: &Tag,
    exclusions: &ExclusionSet,
    conn: &PooledConnection<SqliteConnectionManager>,
) -> AppResult<Markup> {
    let history = tag.history(conn)?;
    let latest = history.first();
    let parsed = classify::parse_version(&tag.tag);
    let rule = exclusions.rule_for(&tag.tag);

    Ok(html! {
        tr class=[rule.as_ref().map(|_| "tag-excluded")] {
            td {
                code { (tag.tag) }
                @if !tag.active {
                    " " span class="badge badge-error" title="No longer present in the registry's tag list" { "gone" }
                }
                @if let Some(rule) = &rule {
                    " " span class="badge badge-excluded" title=(rule.describe()) { "excluded" }
                }
            }
            td {
                @if classify::is_immutable(&tag.tag) {
                    span class="badge badge-immutable" { "immutable" }
                } @else {
                    span class="badge badge-floating" { "floating" }
                }
                @if let Some(parsed) = &parsed {
                    " " span class="cell-secondary" title="Version, variant and build as consumers read this tag" {
                        (parsed.version)
                        @if let Some(variant) = &parsed.variant { " · " (variant) }
                        @if let Some(build) = &parsed.build { " +" (build) }
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
            td class="cell-actions" {
                @if let (Some(parsed), None) = (&parsed, rule) {
                    form class="inline-form" action=(format!("/repo/{}/exclusions", repo.id)) method="post" {
                        input type="hidden" name="version" value=(parsed.version);
                        button class="button button-small" type="submit"
                            title=(format!("Exclude version {} from version matching", parsed.version)) { "Exclude" }
                    }
                }
            }
        }
    })
}

/// The exclusion form on the repo page (and the per-tag "Exclude" button).
#[post("/repo/{id}/exclusions")]
pub async fn add_exclusion_form(
    pool: web::Data<Pool<SqliteConnectionManager>>,
    id: web::Path<u64>,
    form: web::Form<HashMap<String, String>>,
) -> Result<impl Responder, AppError> {
    let conn = pool.get()?;
    let id = id.into_inner();
    if Repo::get(id, &conn)?.is_none() {
        return Err(AppError::NotFound("repo not found".to_string()));
    }
    let version = form.get("version").map(String::as_str).unwrap_or_default();
    VersionExclusion::add(
        id,
        version,
        form.get("note").map(String::as_str),
        chrono::Utc::now(),
        &conn,
    )?;
    Ok(HttpResponse::SeeOther()
        .append_header(("Location", format!("/repo/{id}#exclusions")))
        .finish())
}

#[post("/exclusion/{id}/remove")]
pub async fn remove_exclusion_form(
    pool: web::Data<Pool<SqliteConnectionManager>>,
    id: web::Path<u64>,
) -> Result<impl Responder, AppError> {
    let conn = pool.get()?;
    let Some(rule) = VersionExclusion::get(id.into_inner(), &conn)? else {
        return Err(AppError::NotFound("exclusion not found".to_string()));
    };
    VersionExclusion::remove(rule.id, &conn)?;
    Ok(HttpResponse::SeeOther()
        .append_header(("Location", format!("/repo/{}#exclusions", rule.repo_id)))
        .finish())
}
