mod classify;

use std::time::Duration;

use chrono::{DateTime, Utc};
use futures_util::StreamExt;
use oci_client::Reference;
use r2d2::{Pool, PooledConnection};
use r2d2_sqlite::SqliteConnectionManager;
use tokio::time;

use crate::{
    db::{repo::Repo, repo_state::RepoState, tag::Tag, tag_history::TagHistory},
    error::{AppError, AppResult, format_error_chain},
};

/// How often a repo's registry metadata is considered stale.
const REFRESH_INTERVAL: chrono::Duration = chrono::Duration::hours(6);
/// How often the scheduler looks for repos due for a refresh.
const SCHEDULER_TICK: Duration = Duration::from_secs(60);
/// Tag list page size requested from the registry.
const TAG_PAGE_SIZE: usize = 100;
/// Hard cap on tag list pages per refresh, as a runaway guard.
const MAX_TAG_PAGES: usize = 200;
/// Base delay before retrying a failing repo; doubles per consecutive
/// failure, capped at REFRESH_INTERVAL.
const BACKOFF_BASE: chrono::Duration = chrono::Duration::minutes(1);
/// Concurrent digest fetches per repo refresh.
const DIGEST_CONCURRENCY: usize = 8;
/// Max immutable-tag digest backfills per refresh pass. Floating tags are
/// always fetched; the immutable backlog trickles in at this rate so a
/// newly added repo with thousands of historical tags is useful within
/// seconds and completes over subsequent passes.
const BACKFILL_BUDGET: usize = 500;

pub async fn start_update_poller(pool: Pool<SqliteConnectionManager>) {
    let client = oci_client::Client::default();

    log::info!("Starting update poller");

    let mut interval = time::interval(SCHEDULER_TICK);
    loop {
        interval.tick().await;
        if let Err(e) = scheduler_pass(&client, &pool).await {
            log::error!("Poller scheduler pass failed: {}", format_error_chain(&e));
        }
    }
}

async fn scheduler_pass(
    client: &oci_client::Client,
    pool: &Pool<SqliteConnectionManager>,
) -> AppResult<()> {
    let conn = pool.get()?;
    let now = Utc::now();

    for repo in Repo::all_active(&conn)? {
        let state = RepoState::get(repo.id, &conn)?;
        if !is_due(state.as_ref(), now) {
            continue;
        }

        let started_at = Utc::now();
        match refresh_repo(client, &conn, &repo, started_at).await {
            Ok(summary) => {
                RepoState::record_success(repo.id, started_at, &conn)?;
                log::info!(
                    "Refreshed {}/{}: {} tags ({} new, {} excluded), {} digests fetched ({} failed, {} deferred), {} tags deactivated{}",
                    repo.registry,
                    repo.name,
                    summary.tags_seen,
                    summary.tags_new,
                    summary.tags_excluded,
                    summary.digests_fetched,
                    summary.digests_failed,
                    summary.digests_deferred,
                    summary.tags_deactivated,
                    if summary.rate_limited {
                        " [rate limited, digest fetches abandoned]"
                    } else {
                        ""
                    },
                );
            }
            Err(e) => {
                let message = format_error_chain(&e);
                log::warn!(
                    "Failed to refresh {}/{}: {}",
                    repo.registry,
                    repo.name,
                    message
                );
                RepoState::record_failure(repo.id, started_at, &message, &conn)?;
            }
        }
    }

    Ok(())
}

/// A repo is due when it has never been successfully refreshed, or its last
/// success is older than REFRESH_INTERVAL. While it is failing, retries are
/// paced by exponential backoff from the last attempt instead.
fn is_due(state: Option<&RepoState>, now: DateTime<Utc>) -> bool {
    let Some(state) = state else {
        return true;
    };

    if state.consecutive_failures > 0 {
        let exponent = state.consecutive_failures.saturating_sub(1).min(16);
        let backoff = std::cmp::min(BACKOFF_BASE * 2_i32.saturating_pow(exponent), REFRESH_INTERVAL);
        return match state.last_attempted_at {
            Some(attempted) => now >= attempted + backoff,
            None => true,
        };
    }

    match state.last_checked_at {
        Some(checked) => now >= checked + REFRESH_INTERVAL,
        None => true,
    }
}

struct RefreshSummary {
    tags_excluded: usize,
    tags_seen: usize,
    tags_new: usize,
    digests_fetched: usize,
    digests_failed: usize,
    digests_deferred: usize,
    tags_deactivated: usize,
    rate_limited: bool,
}

async fn refresh_repo(
    client: &oci_client::Client,
    conn: &PooledConnection<SqliteConnectionManager>,
    repo: &Repo,
    now: DateTime<Utc>,
) -> AppResult<RefreshSummary> {
    let reference: Reference = format!("{}/{}", repo.registry, repo.name)
        .parse()
        .map_err(|e: oci_client::ParseError| AppError::Parse(e.to_string()))?;

    let all_tags = list_all_tags(client, &reference).await?;
    let total = all_tags.len();
    let tags: Vec<String> = all_tags
        .into_iter()
        .filter(|t| !classify::is_excluded(t))
        .collect();

    let mut summary = RefreshSummary {
        tags_excluded: total - tags.len(),
        tags_seen: tags.len(),
        tags_new: 0,
        digests_fetched: 0,
        digests_failed: 0,
        digests_deferred: 0,
        tags_deactivated: 0,
        rate_limited: false,
    };

    // Floating tags are re-fetched every refresh; immutable tags only need
    // their digest resolved once, and the backlog of never-resolved ones is
    // drained on a per-pass budget, floating first.
    let mut floating: Vec<(u64, String)> = Vec::new();
    let mut backfill: Vec<(u64, String)> = Vec::new();

    for tag_name in &tags {
        let tag = Tag::upsert(repo.id, tag_name, now, conn)?;
        if tag.first_seen_at.timestamp() == now.timestamp() {
            summary.tags_new += 1;
        }

        if classify::is_immutable(tag_name) {
            if TagHistory::latest(tag.id, conn)?.is_none() {
                backfill.push((tag.id, tag_name.clone()));
            }
        } else {
            floating.push((tag.id, tag_name.clone()));
        }
    }

    summary.digests_deferred = backfill.len().saturating_sub(BACKFILL_BUDGET);
    backfill.truncate(BACKFILL_BUDGET);
    floating.extend(backfill);

    let mut fetches = futures_util::stream::iter(floating.into_iter().map(|(tag_id, tag_name)| {
        let tag_reference = Reference::with_tag(
            reference.registry().to_string(),
            reference.repository().to_string(),
            tag_name.clone(),
        );
        async move {
            let result = client
                .fetch_manifest_digest(
                    &tag_reference,
                    &oci_client::secrets::RegistryAuth::Anonymous,
                )
                .await;
            (tag_id, tag_name, result)
        }
    }))
    .buffer_unordered(DIGEST_CONCURRENCY);

    while let Some((tag_id, tag_name, result)) = fetches.next().await {
        match result {
            Ok(digest) => {
                TagHistory::record(tag_id, &digest, now, conn)?;
                summary.digests_fetched += 1;
            }
            Err(e) if is_rate_limit(&e) => {
                // Stop hammering the registry: abandon the rest of this
                // pass's fetches. Whatever is missing is picked up next pass.
                log::warn!(
                    "Rate limited by {} while fetching {}:{}; abandoning remaining digest fetches this pass: {}",
                    repo.registry,
                    repo.name,
                    tag_name,
                    e
                );
                summary.rate_limited = true;
                break;
            }
            // A single tag's digest failing shouldn't fail the whole refresh;
            // it stays stale and is retried next pass.
            Err(e) => {
                log::warn!(
                    "Failed to fetch digest for {}/{}:{}: {}",
                    repo.registry,
                    repo.name,
                    tag_name,
                    e
                );
                summary.digests_failed += 1;
            }
        }
    }

    summary.tags_deactivated = Tag::deactivate_missing(repo.id, now, conn)?;

    Ok(summary)
}

/// Whether a registry error is a rate-limit response. oci_client doesn't
/// expose the status code structurally for API-error envelopes, so this
/// matches on the rendered message (429 / TOOMANYREQUESTS code / the
/// retry-after hint some registries include in the error body).
fn is_rate_limit(e: &oci_client::errors::OciDistributionError) -> bool {
    let message = e.to_string().to_lowercase();
    message.contains("429")
        || message.contains("toomanyrequests")
        || message.contains("too many requests")
        || message.contains("retry-after")
}

/// Fetch the complete tag list, following OCI `n`/`last` pagination until a
/// short page. Ordering is registry-dependent, so the result is just a set.
async fn list_all_tags(
    client: &oci_client::Client,
    reference: &Reference,
) -> AppResult<Vec<String>> {
    let mut tags: Vec<String> = Vec::new();
    let mut last: Option<String> = None;

    for _ in 0..MAX_TAG_PAGES {
        let response = client
            .list_tags(
                reference,
                &oci_client::secrets::RegistryAuth::Anonymous,
                Some(TAG_PAGE_SIZE),
                last.as_deref(),
            )
            .await?;

        let page_len = response.tags.len();
        let next_last = response.tags.last().cloned();
        tags.extend(response.tags);

        // A registry that ignores `last` would loop forever; stop if we make
        // no forward progress.
        if page_len < TAG_PAGE_SIZE || next_last.is_none() || next_last == last {
            break;
        }
        last = next_last;
    }

    tags.sort();
    tags.dedup();
    Ok(tags)
}
