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

use pierre_core::permissions::scopes::OAuthScope;
use std::collections::HashSet;
use std::env;
use std::sync::{Arc, LazyLock, Mutex, PoisonError};
use std::time::Duration as StdDuration;

use chrono::{Duration, TimeZone, Utc};
use pierre_core::models::{Activity, TenantId};
use pierre_database::repositories::BackfillCoverage;
use pierre_providers::core::ActivityQueryParams;
use tokio::time::sleep;
use tracing::{info, warn};
use uuid::Uuid;

use crate::activity_fetch::{
    activity_cache_retention_days, read_cached_window, serve_historical_window,
    write_through_activity_cache,
};
use crate::protocol::types::auth_required_provider;
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

/// The depth a completed backfill may claim as covered, in unix seconds.
///
/// When the scrape did NOT fill `fetch_limit` it returned every activity in the
/// requested `[after, before]` window, so it reached the requested floor
/// `after_ts` itself — record that floor. Crucially this is the REQUESTED floor,
/// not the oldest activity fetched: at a clean period boundary (a year query has
/// `after` = Jan 1 00:00:00) the oldest activity sits just above `after_ts`, so
/// recording the activity timestamp would leave the window looking uncovered and
/// re-scrape forever. When the scrape filled the limit it stopped early
/// (count-capped), so the deepest it can honestly claim is the oldest activity
/// fetched. `pub` so the decision is exercisable by the integration test suite.
#[must_use]
pub fn backfill_covered_floor_ts(
    after_ts: i64,
    oldest_reached_ts: i64,
    fetched_count: usize,
    fetch_limit: usize,
) -> i64 {
    if fetched_count < fetch_limit {
        // Not count-capped: the whole window came back, so the requested floor
        // was reached. `min` is defensive — the scrape only returns activities
        // at or after `after_ts`, so `oldest_reached_ts >= after_ts` normally.
        after_ts.min(oldest_reached_ts)
    } else {
        oldest_reached_ts
    }
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

/// How often an inline backfill re-checks whether a concurrent job's in-flight
/// claim has cleared.
const IN_FLIGHT_WAIT_POLL: StdDuration = StdDuration::from_millis(500);

/// Upper bound on waiting for a concurrent job's claim. A full sciotte
/// season scrape sits under its 330s client ceiling, so 8 minutes covers the
/// slowest legitimate holder; past it the key is presumed leaked (a panicked
/// job never removes its claim) and the wait gives up instead of hanging the
/// task forever.
const IN_FLIGHT_WAIT_CAP: StdDuration = StdDuration::from_mins(8);

/// Terminal outcome of one backfill run.
///
/// The detached spawn path ignores it (its notifications happen inside the
/// run); the inline path maps it onto the tool response so a declaring-tasks
/// caller gets an honest auth error or retry hint instead of an empty window.
enum BackfillRunOutcome {
    /// The window was fetched and durably persisted — or the provider holds
    /// nothing in it. Either way the durable cache now holds the answer.
    Completed,
    /// The provider session lapsed mid-run; the user must reconnect.
    AuthRequired,
    /// Transient failure — nothing persisted; a retry re-runs the window.
    Failed,
}

/// Run a backfill to completion on the caller's own future, serializing on
/// the same in-flight key as [`spawn_activity_backfill`] so concurrent asks
/// for one `(user, provider)` never stack scrapes.
///
/// When another job holds the key this waits for it to clear, then runs its
/// own window anyway: the concurrent job may have paged a shallower `after`,
/// and re-running is the only way to guarantee the requested depth without
/// re-deriving the coverage gate here. The redundant scrape costs one
/// serialized provider pass in the rare double-ask race; the wrong answer
/// would cost serving a shallow window as if it were complete.
async fn run_activity_backfill_inline(job: ActivityBackfillJob) -> BackfillRunOutcome {
    let key = in_flight_key(job.user_id, &job.provider_name);
    let mut waited = StdDuration::ZERO;
    loop {
        let claimed = IN_FLIGHT_BACKFILLS
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.clone());
        if claimed {
            break;
        }
        if waited >= IN_FLIGHT_WAIT_CAP {
            warn!(
                user_id = %job.user_id,
                provider = %job.provider_name,
                "Inline backfill gave up waiting on a concurrent claim"
            );
            return BackfillRunOutcome::Failed;
        }
        sleep(IN_FLIGHT_WAIT_POLL).await;
        waited += IN_FLIGHT_WAIT_POLL;
    }
    let outcome = run_activity_backfill(&job).await;
    IN_FLIGHT_BACKFILLS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .remove(&key);
    outcome
}

/// One declaring-tasks historical serve: what the inline backfill + durable
/// read produced for the caller to answer with.
pub(crate) enum InlineHistoricalServe {
    /// The backfill completed; the (possibly empty) served window follows.
    /// Empty is the honest answer when the provider holds nothing that old.
    Served(Vec<Activity>),
    /// The provider session lapsed mid-run; the caller raises the typed
    /// reconnect error.
    AuthRequired,
    /// Transient failure — the caller answers with a retry hint.
    Failed,
}

/// Run the backfill inline (see [`run_activity_backfill_inline`]) and read
/// the served window back out of the durable cache.
///
/// The read uses the same bounded `coverage_params` as the coverage gate and
/// the same head-slice / stale-head assembly as the covered serve
/// (`serve_historical_window`), so the gate, the covered path, and this
/// path cannot drift.
pub(crate) async fn backfill_inline_and_serve(
    job: ActivityBackfillJob,
    coverage_params: &ActivityQueryParams,
    before: Option<i64>,
) -> InlineHistoricalServe {
    let resources = job.resources.clone();
    let provider_name = job.provider_name.clone();
    let user_id = job.user_id;
    let tenant_id = job.tenant_id;
    match run_activity_backfill_inline(job).await {
        BackfillRunOutcome::AuthRequired => return InlineHistoricalServe::AuthRequired,
        BackfillRunOutcome::Failed => return InlineHistoricalServe::Failed,
        BackfillRunOutcome::Completed => {}
    }
    let window = read_cached_window(
        &resources,
        &provider_name,
        user_id,
        tenant_id,
        coverage_params,
    )
    .await
    .unwrap_or_default();
    let served = serve_historical_window(
        &resources,
        &provider_name,
        user_id,
        tenant_id,
        before,
        coverage_params.before,
        window,
    )
    .await;
    InlineHistoricalServe::Served(served)
}

/// Outcome of authenticating + paging the provider for a backfill.
enum BackfillFetch {
    /// Activities fetched (possibly empty).
    Activities(Vec<Activity>),
    /// The provider's session/token lapsed — the caller nudges the user to
    /// reconnect, since this detached path is otherwise silent.
    AuthRequired,
    /// Any other (transient/unsupported) failure — logged, no nudge.
    Failed,
}

/// Authenticate the provider and page its feed to the requested `after`,
/// returning the historical activities. Auth and fetch failures are logged and
/// classified — this runs detached, so a non-auth failure is just a no-op
/// backfill, but a lapsed session is surfaced as [`BackfillFetch::AuthRequired`]
/// so the caller can nudge the user to reconnect (the foreground tool loop
/// already nudges inline; the backfill must do it too or the expiry is silent).
async fn fetch_backfill_activities(
    job: &ActivityBackfillJob,
    executor: &UniversalExecutor,
) -> BackfillFetch {
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
            // A non-recoverable dead connection tags the response with the
            // auth-required provider (see create_authenticated_provider) —
            // distinguish it from an unsupported-provider / other failure so
            // only a real expiry nudges the user.
            let auth_required = auth_required_provider(&response).is_some();
            warn!(
                user_id = %job.user_id,
                provider = %job.provider_name,
                error = ?response.error,
                auth_required,
                "Activity backfill: provider authentication failed"
            );
            return if auth_required {
                BackfillFetch::AuthRequired
            } else {
                BackfillFetch::Failed
            };
        }
    };

    match provider.get_activities_with_params(&job.query_params).await {
        Ok(activities) => BackfillFetch::Activities(activities),
        Err(e) => {
            // sciotte surfaces a lapsed session HERE (cookies existed at auth
            // time but the scrape redirected to sign-in) as
            // `AppError::provider_auth_required`; any other error is transient.
            let auth_required = e.provider_auth_required_provider().is_some();
            warn!(
                user_id = %job.user_id,
                provider = %job.provider_name,
                error = %e,
                auth_required,
                "Activity backfill: provider fetch failed"
            );
            if auth_required {
                BackfillFetch::AuthRequired
            } else {
                BackfillFetch::Failed
            }
        }
    }
}

/// Record how deep a completed backfill reached for `(tenant, user, provider)`
/// so the historical gate can serve a complete window or self-heal a shallow one.
///
/// Best-effort: a write failure is logged, never propagated. No-op when the job
/// carries no `after` (only the dated gate path records coverage) or fetched
/// nothing. The covered depth is computed via [`backfill_covered_floor_ts`].
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
    let coverage = BackfillCoverage {
        oldest_reached_ts: backfill_covered_floor_ts(
            after_ts,
            oldest_reached_ts,
            fetched_count,
            fetch_limit,
        ),
        // The scrape path cannot PROVE the provider feed is exhausted: a
        // date-floored scrape stops at `after`, never paging below it, so
        // "fewer than the limit" only means the requested window was exhausted,
        // not the whole feed. Inferring feed-end from the count falsely marked
        // every deeper query covered, serving an empty slice from a shallow
        // cache (a 2024-bounded scrape masking real 2022/2023 data). Reaching
        // the requested floor is already captured by oldest_reached_ts above;
        // hit_feed_end stays reserved for an explicit provider feed-exhaustion
        // signal, which the gate still honors when set.
        hit_feed_end: false,
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
/// activities through to the durable cache, reporting the terminal outcome.
/// All failures are logged and mapped — the detached spawn path drops the
/// outcome, the inline path surfaces it to the caller.
async fn run_activity_backfill(job: &ActivityBackfillJob) -> BackfillRunOutcome {
    // Bound explicitly, not inherited: this path is reached from a detached
    // `tokio::spawn`, which a task-local does not cross. The job runs the
    // athlete's own backfill, so it carries the self grant.
    let executor =
        UniversalExecutor::new(job.resources.clone()).with_scopes(OAuthScope::self_grant());
    let activities = match fetch_backfill_activities(job, &executor).await {
        BackfillFetch::Activities(activities) => activities,
        BackfillFetch::AuthRequired => {
            // The provider session expired mid-backfill. This detached path is
            // otherwise silent, so nudge the user to reconnect on the channel
            // that asked instead of leaving them on a perpetual "ask again".
            notify_backfill_reauth(job).await;
            return BackfillRunOutcome::AuthRequired;
        }
        BackfillFetch::Failed => return BackfillRunOutcome::Failed,
    };

    if activities.is_empty() {
        info!(
            user_id = %job.user_id,
            provider = %job.provider_name,
            "Activity backfill: provider returned no historical activities"
        );
        return BackfillRunOutcome::Completed;
    }

    let fetched_count = activities.len();
    let retention_days = backfill_retention_days(job.query_params.after);
    // Persist BEFORE recording coverage, and only proceed when the write lands.
    // A failed/partial write that still stamped coverage would leave the gate
    // believing the window is complete while the rows are missing — it would then
    // serve the stale slice and never self-heal (the "2022 stuck at 46" class).
    let Some(activity_count) =
        persist_backfill_activities(job, &executor, &activities, fetched_count, retention_days)
            .await
    else {
        return BackfillRunOutcome::Failed;
    };

    // Record how deep this backfill reached — now that the rows are durably
    // persisted — so the historical gate can tell a complete window from a
    // shallow recent slice and self-heal a partial one, and its "covered"
    // verdict can never outrun the data actually in the cache.
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

    notify_backfill_complete(job, activity_count).await;
    BackfillRunOutcome::Completed
}

/// Write the scraped historical activities through to the durable cache and
/// return the net distinct rows persisted (deduped by `activity_id`), or `None`
/// when the write-through fails. A failed/partial write must not let the caller
/// record coverage — recording it would leave the gate believing the window is
/// complete while the rows are missing (the "2022 stuck at 46" class), so the
/// caller bails on `None` and the next ask re-backfills the window.
async fn persist_backfill_activities(
    job: &ActivityBackfillJob,
    executor: &UniversalExecutor,
    activities: &[Activity],
    fetched_count: usize,
    retention_days: i64,
) -> Option<usize> {
    let Some(persisted) = write_through_activity_cache(
        &executor.auth_service,
        job.user_id,
        job.tenant_id,
        &job.provider_name,
        activities,
        retention_days,
    )
    .await
    else {
        warn!(
            user_id = %job.user_id,
            provider = %job.provider_name,
            fetched_count,
            "Activity backfill: durable write-through failed; not recording coverage so the gate re-backfills"
        );
        return None;
    };
    // The persisted count is the NET DISTINCT rows actually stored (deduped by
    // `activity_id`), below `fetched_count` when the feed repeats an id in the
    // batch — the honest figure the user sees.
    Some(usize::try_from(persisted).unwrap_or(fetched_count))
}

/// Push a reconnect nudge to the originating channel when a chat-triggered
/// backfill hit a lapsed provider session — a localized message + a one-time
/// hosted-login link, routed cross-channel via the shared notifier rail.
/// Mirrors [`notify_backfill_complete`]'s guards: no conversation id (MCP / A2A
/// / SSE) or no notifier wired → no-op (the connection-status surface still
/// reflects the expiry). By design the nudge is NOT deduped: it re-sends every
/// expired-session turn until a real reconnect clears the flag, so a user whose
/// first link was broken or never clicked is not permanently silenced.
async fn notify_backfill_reauth(job: &ActivityBackfillJob) {
    let (Some(conversation_id), Some(notifier)) = (
        job.pierre_conversation_id.as_deref(),
        job.resources.backfill_notifier(),
    ) else {
        return;
    };
    notifier
        .push_provider_reauth(
            job.user_id,
            job.tenant_id,
            conversation_id,
            &job.provider_name,
        )
        .await;
}

/// Push a "your history is ready" notice back to the originating channel for a
/// chat-triggered backfill. Best-effort — the notifier owns its own error
/// handling and `activity_count` is the net distinct rows persisted (deduped by
/// `activity_id`), the honest figure the user sees. No-op for direct MCP / A2A
/// callers (no conversation id) or when no notifier is wired (messaging compiled
/// out / not configured).
async fn notify_backfill_complete(job: &ActivityBackfillJob, activity_count: usize) {
    let (Some(conversation_id), Some(notifier)) = (
        job.pierre_conversation_id.as_deref(),
        job.resources.backfill_notifier(),
    ) else {
        return;
    };
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
