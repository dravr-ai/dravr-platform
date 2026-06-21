// ABOUTME: Background historical-activity backfill so a deep `after` never scrapes inline
// ABOUTME: Pages a provider's feed to-date off the request path and writes through to the durable cache
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Historical activity backfill.
//!
//! A coaching turn that asks for an old window (e.g. "my 2022 races") must
//! never trigger an inline provider scrape: paginating a provider's
//! reverse-chronological feed back across years stalls the turn and trips the
//! known sciotte navigation timeout. Instead the `get_activities` handler
//! serves deep windows from the durable activity cache, and — on a cold cache —
//! kicks off one of these bounded background jobs. The job authenticates the
//! provider, pages the feed back to the requested `after` via the date-aware
//! scrape, and writes the results through to the cache so the next ask is a
//! plain cache hit. Jobs are de-duplicated per `(user, provider)` and inherit
//! the provider's global scrape-concurrency permit, so they cannot stampede
//! Chrome.

use std::collections::HashSet;
use std::env;
use std::sync::{Arc, LazyLock, Mutex, PoisonError};

use chrono::{Duration, TimeZone, Utc};
use pierre_core::models::{Activity, TenantId};
use pierre_database::repositories::BackfillCoverage;
use pierre_providers::core::ActivityQueryParams;
use tracing::{info, warn};
use uuid::Uuid;

use crate::activity_fetch::{activity_cache_retention_days, write_through_activity_cache};
use crate::protocol::UniversalExecutor;
use crate::runtime::ToolRuntime;

/// Default age (days) past which an activity request is served via background
/// backfill instead of an inline provider scrape.
const DEFAULT_HISTORICAL_BACKFILL_THRESHOLD_DAYS: i64 = 90;

/// Extra days added to a backfill's prune window so the season it just fetched
/// is not immediately garbage-collected by its own write-through.
const BACKFILL_RETENTION_MARGIN_DAYS: i64 = 7;

/// Threshold (days) past which an `after` lower bound triggers background
/// backfill, from `PIERRE_HISTORICAL_BACKFILL_THRESHOLD_DAYS` (falls back to
/// [`DEFAULT_HISTORICAL_BACKFILL_THRESHOLD_DAYS`]). Non-positive or unparseable
/// values fall back to the default.
fn historical_backfill_threshold_days() -> i64 {
    env::var("PIERRE_HISTORICAL_BACKFILL_THRESHOLD_DAYS")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .filter(|days| *days > 0)
        .unwrap_or(DEFAULT_HISTORICAL_BACKFILL_THRESHOLD_DAYS)
}

/// Whether an `after` lower bound (unix seconds) is deep enough to require
/// background backfill instead of an inline provider scrape.
///
/// Compares the bound against the [`historical_backfill_threshold_days`]
/// window. `pub` so the gate decision can be exercised by the integration
/// test suite.
#[must_use]
pub fn is_historical_backfill_window(after_ts: i64) -> bool {
    Utc.timestamp_opt(after_ts, 0)
        .single()
        .is_some_and(|after| {
            after < Utc::now() - Duration::days(historical_backfill_threshold_days())
        })
}

/// Prune window (days) for a backfill write: deep enough to span from the
/// requested `after` to now (plus a margin) so the fetched season isn't pruned
/// by its own write-through. Falls back to the configured retention when no
/// `after` is set.
fn backfill_retention_days(after: Option<i64>) -> i64 {
    let base = activity_cache_retention_days();
    let Some(after_dt) = after.and_then(|ts| Utc.timestamp_opt(ts, 0).single()) else {
        return base;
    };
    let span = (Utc::now() - after_dt).num_days() + BACKFILL_RETENTION_MARGIN_DAYS;
    base.max(span)
}

/// Infer whether a completed backfill scrape exhausted the provider feed (no
/// older data) rather than stopping because it reached the requested floor or
/// hit the fetch limit.
///
/// Feed-end = the oldest activity fetched is still newer than `after_ts` (it did
/// not reach the requested floor) AND the scrape did not fill `fetch_limit` (so
/// it was not count-capped — it ran out of feed). When it reached `after`
/// (`oldest <= after`) the floor is the request, not the feed end. `pub` so the
/// inference is exercisable by the integration test suite.
#[must_use]
pub fn backfill_hit_feed_end(
    oldest_reached_ts: i64,
    after_ts: i64,
    fetched_count: usize,
    fetch_limit: usize,
) -> bool {
    oldest_reached_ts > after_ts && fetched_count < fetch_limit
}

/// In-flight backfill keys (`user_id:provider`) so repeated asks while a job is
/// already paginating a user's history don't stack duplicate scrapes.
static IN_FLIGHT_BACKFILLS: LazyLock<Mutex<HashSet<String>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

fn in_flight_key(user_id: Uuid, provider: &str) -> String {
    format!("{user_id}:{provider}")
}

/// Inputs for a background activity backfill.
pub(crate) struct ActivityBackfillJob {
    /// Shared runtime used to re-authenticate the provider off the request path.
    pub resources: Arc<dyn ToolRuntime>,
    /// User whose history is being backfilled.
    pub user_id: Uuid,
    /// Tenant that owns the cache rows.
    pub tenant_id: TenantId,
    /// Tenant id as a string for provider authentication (`None` for the nil tenant).
    pub tenant_id_str: Option<String>,
    /// Backend provider slug to scrape (already resolved to the sciotte mirror if any).
    pub provider_name: String,
    /// The original request window — its deep `after` drives the page-to-date scrape.
    pub query_params: ActivityQueryParams,
    /// Originating Pierre conversation id when the backfill was triggered from a
    /// chat turn, so a later phase can push the completion notice back to the
    /// channel that asked. `None` for MCP-direct / A2A / SSE callers with no
    /// conversation. Unused until the backfill-completion push lands.
    pub pierre_conversation_id: Option<String>,
}

/// Spawn a bounded background job that pages a provider's feed back to the
/// requested historical `after` and writes the results through to the durable
/// activity cache. De-duplicated per `(user, provider)`. Returns `true` if a
/// new job was started, `false` if one was already in flight.
pub(crate) fn spawn_activity_backfill(job: ActivityBackfillJob) -> bool {
    let key = in_flight_key(job.user_id, &job.provider_name);
    {
        let mut guard = IN_FLIGHT_BACKFILLS
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if !guard.insert(key.clone()) {
            info!(
                user_id = %job.user_id,
                provider = %job.provider_name,
                "Activity backfill already in flight — skipping duplicate"
            );
            return false;
        }
    }

    tokio::spawn(async move {
        run_activity_backfill(&job).await;
        IN_FLIGHT_BACKFILLS
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&key);
    });
    true
}

/// Authenticate the provider and page its feed to the requested `after`,
/// returning the historical activities. Auth and fetch failures are logged and
/// folded to `None` — this runs detached, so a failure is just a no-op backfill.
async fn fetch_backfill_activities(
    job: &ActivityBackfillJob,
    executor: &UniversalExecutor,
) -> Option<Vec<Activity>> {
    let provider = match executor
        .auth_service
        .create_authenticated_provider(
            &job.provider_name,
            job.user_id,
            job.tenant_id_str.as_deref(),
        )
        .await
    {
        Ok(provider) => provider,
        Err(response) => {
            warn!(
                user_id = %job.user_id,
                provider = %job.provider_name,
                error = ?response.error,
                "Activity backfill: provider authentication failed"
            );
            return None;
        }
    };

    match provider.get_activities_with_params(&job.query_params).await {
        Ok(activities) => Some(activities),
        Err(e) => {
            warn!(
                user_id = %job.user_id,
                provider = %job.provider_name,
                error = %e,
                "Activity backfill: provider fetch failed"
            );
            None
        }
    }
}

/// Record how deep a completed backfill reached for `(tenant, user, provider)`
/// so the historical gate can serve a complete window or self-heal a shallow one.
///
/// Best-effort: a write failure is logged, never propagated. No-op when the job
/// carries no `after` (only the dated gate path records coverage) or fetched
/// nothing. `hit_feed_end` is inferred via [`backfill_hit_feed_end`].
async fn record_backfill_coverage(
    job: &ActivityBackfillJob,
    activities: &[Activity],
    fetched_count: usize,
) {
    let Some(after_ts) = job.query_params.after else {
        return;
    };
    let Some(oldest) = activities.iter().map(Activity::start_date).min() else {
        return;
    };
    let oldest_reached_ts = oldest.timestamp();
    let fetch_limit = job.query_params.limit.unwrap_or(usize::MAX);
    let hit_feed_end =
        backfill_hit_feed_end(oldest_reached_ts, after_ts, fetched_count, fetch_limit);
    let coverage = BackfillCoverage {
        oldest_reached_ts,
        hit_feed_end,
    };
    if let Err(e) = job
        .resources
        .repos()
        .activity_cache
        .upsert_backfill_coverage(job.user_id, &job.tenant_id, &job.provider_name, coverage)
        .await
    {
        warn!(error = %e, "Activity backfill: failed to record coverage");
    }
}

/// Page the provider's feed to the requested `after` and write the historical
/// activities through to the durable cache. All failures are logged and
/// swallowed — this runs detached from any request.
async fn run_activity_backfill(job: &ActivityBackfillJob) {
    let executor = UniversalExecutor::new(job.resources.clone());
    let Some(activities) = fetch_backfill_activities(job, &executor).await else {
        return;
    };

    if activities.is_empty() {
        info!(
            user_id = %job.user_id,
            provider = %job.provider_name,
            "Activity backfill: provider returned no historical activities"
        );
        return;
    }

    let fetched_count = activities.len();
    let retention_days = backfill_retention_days(job.query_params.after);
    // The persisted count is the NET DISTINCT rows actually stored (deduped by
    // activity_id), which can be below `fetched_count` when the provider feed
    // repeats an id within the batch. Surface the persisted figure to the user;
    // fall back to the fetched length only if the write-through failed.
    let activity_count = write_through_activity_cache(
        &executor.auth_service,
        job.user_id,
        job.tenant_id,
        &job.provider_name,
        &activities,
        retention_days,
    )
    .await
    .and_then(|c| usize::try_from(c).ok())
    .unwrap_or(fetched_count);

    // Record how deep this backfill reached so the historical gate can tell a
    // complete window from a shallow recent slice and self-heal a partial one.
    record_backfill_coverage(job, &activities, fetched_count).await;

    info!(
        user_id = %job.user_id,
        provider = %job.provider_name,
        count = activity_count,
        fetched_count,
        retention_days,
        // Whether this backfill was chat-triggered, so the later completion-push
        // phase's behaviour is observable. The conversation id itself is an
        // internal identifier (already logged on the chat dispatch path); here we
        // only surface whether one is attached.
        chat_triggered = job.pierre_conversation_id.is_some(),
        "Activity backfill: wrote historical activities to durable cache"
    );

    // Chat-triggered backfill: push a "your history is ready" notice back to the
    // originating channel. Best-effort — the notifier owns its own error
    // handling and `activity_count` is the net distinct rows persisted (deduped
    // by activity_id), the honest figure the user sees. No-op for MCP-direct /
    // A2A callers (no conversation id) or when no notifier is wired (messaging
    // compiled out / not configured).
    if let (Some(conversation_id), Some(notifier)) = (
        job.pierre_conversation_id.as_deref(),
        job.resources.backfill_notifier(),
    ) {
        // The historical-window `after` (unix seconds) keys the durable
        // cross-replica dedup claim; `0` when the request had no `after`.
        let after_ts = job.query_params.after.unwrap_or(0);
        notifier
            .push_backfill_complete(
                job.user_id,
                job.tenant_id,
                conversation_id,
                &job.provider_name,
                after_ts,
                activity_count,
            )
            .await;
    }
}
