// ABOUTME: Shared helper to fetch a user's recent activities from their connected providers
// ABOUTME: One auth+fetch path reused by group snapshots and agent recommendations — no duplication
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Provider activity fetching shared across route crates.
//!
//! Authenticating a provider (refreshing OAuth tokens, resolving sciotte
//! mirrors) and pulling activities is identical whether the caller is the
//! group analytics snapshot builder or the agent recommender. This module
//! owns that single path so neither re-implements it.

use std::cmp::{Ordering, Reverse};
use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Duration, NaiveDate, TimeZone, Utc};
use pierre_core::civil_time::resolve_zone;
use pierre_core::constants::oauth_providers;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{Activity, TenantId};
use pierre_providers::core::ActivityQueryParams;
use pierre_providers::deduplication::{merge_duplicates, DedupConfig, FragmentReport};
use pierre_providers::CoreFitnessProvider;
use tracing::{info, warn};
use uuid::Uuid;

use crate::capture_sweep::FLAG_REASON;
use crate::context::ToolExecutionContext;
use crate::protocol::auth::AuthService;
use crate::protocol::reauth_notice;
use crate::reauth_retry;
use crate::runtime::ToolRuntime;
use pierre_providers::ai_scope;
use pierre_providers::backend_resolver;
use pierre_services::training_history_read::history_store_keys;
use serde_json::Value;

/// A live list read that answers its turn inside the refresh budget; it runs
/// as an `activity_backfill` job, so it is gated with that module.
#[cfg(feature = "tools-data")]
pub mod budgeted_read;
/// The athlete's other connections serving a turn its elected one cannot
mod fallback;
/// A covered historical window's serve and its stale-head top-up, which
/// reads through [`budgeted_read`]; gated with it.
#[cfg(feature = "tools-data")]
mod historical;
/// A push-only provider's reads: its cached pushes, never a live read
pub mod pushed;
/// The session a cache write lands, and the `per_session` persona digest it sends
#[cfg(feature = "client-notifications")]
mod session_landing;
/// Whether a live list read counts as a sync, and what a failed one records
pub mod sync_verdict;
/// The athlete's uploaded `.fit` activities folded into the agent's reads
pub mod uploads;
/// The activity cache's write-through, and what a write says about freshness
pub mod write_through;

pub(crate) use fallback::flag_refusal;
pub(crate) use fallback::grant_window;
pub use fallback::{serve_without_primary, FallbackServe};
#[cfg(feature = "tools-data")]
pub use historical::{
    refresh_stale_head, serve_covered_history, serve_historical_window, HistoricalServe,
};
pub(crate) use pushed::connection_grant_beside_session;
use pushed::{pushed_head, pushed_window, reads_pushed_store, serving_slug};
use sync_verdict::{
    judge_live_read, read_provider_head, record_head_outcome, HeadRead, HeadVerdict, LiveRead,
};
use write_through::{write_through_activity_cache, write_through_served_window, WriteThrough};

/// Read limit for the single deterministic durable-cache read on the historical
/// backfill path. Defined beside the training-history read, which reads the
/// same windows; `pub` so the cap boundary is exercisable by the coverage-note
/// tests (re-exported through `implementations::data`, its original home).
pub use pierre_core::models::coverage::WindowTotals;
pub use pierre_services::training_history_read::HISTORICAL_WINDOW_READ_LIMIT;

/// Cache fallback window when the request carries no `after` lower bound.
const STALE_FALLBACK_WINDOW_DAYS: i64 = 90;

/// Cap on stale rows served from cache when a live fetch fails.
const STALE_FALLBACK_LIMIT: i64 = 500;

/// How far back a read of the cached activities looks, in days: the
/// lookback of the stale fallback, the group snapshot and Home's views.
///
/// A reading window, not retention: rows are kept by the activity store's
/// one retention policy (carnet#1482), and nothing deletes a row for being
/// older than this.
pub const CACHED_ACTIVITY_LOOKBACK_DAYS: i64 = 180;

/// Fold the athlete's OTHER connected providers into a primary fetch, then
/// merge every recording of one workout into a single session.
///
/// Folding is skipped when the caller pinned an explicit `provider` argument
/// or the ask is the coverage-gated historical branch; the merge always runs,
/// because one connection alone can hold two recordings of a workout (a watch
/// and a bike computer uploading the same ride). The report names every group
/// merged and every field a session took from another recording.
///
/// `resolve_provider_for_tool` picks ONE provider (arg, env, or most recently
/// used connection), which shows a multi-provider athlete only a slice of
/// their training: on 2026-08-22 a most-recently-used WHOOP connection hid a
/// 200km Strava ride behind WHOOP's distance-less, misclassified "run" record
/// of the same session. This reuses the peer tool's cache-degrading
/// [`fetch_provider_activities`] per remaining connection (cross-tenant, like
/// the peer path, so a provider connected under the athlete's own tenant
/// resolves from a group conversation) and the session merger, which keeps the
/// GPS row, pairs a watch's misclassified sport with the GPS provider's record
/// of the same workout, and fills the GPS row's gaps from the watch's record.
///
/// Best-effort by design: a secondary connection that fails to fetch is
/// skipped (the helper already logs it) — the primary path alone decides
/// auth errors and reconnect handoffs.
///
/// The activities the athlete brought in themselves ([`uploads`]: uploaded
/// `.fit` files, imported archives) join every read that is not pinned to one
/// provider, the historical branch included: they are a local read with no
/// connection to fold, and the merge pairs one with a provider's copy of the
/// same workout. A read pinned to a provider also serves the athlete's own
/// copies of that provider's history (an imported Strava export for
/// `strava`), merged with the live rows.
pub async fn maybe_merge_other_connections(
    context: &ToolExecutionContext,
    args: &Value,
    is_historical: bool,
    primary_backend: &str,
    params: &ActivityQueryParams,
    mut activities: Vec<Activity>,
) -> (Vec<Activity>, FragmentReport) {
    let explicit_provider_arg = args
        .get("provider")
        .and_then(Value::as_str)
        .is_some_and(|s| !s.is_empty());
    if !explicit_provider_arg && !is_historical {
        fold_other_connections(context, primary_backend, params, &mut activities).await;
    }
    if explicit_provider_arg {
        let provider = backend_resolver::user_facing_name(primary_backend);
        activities.extend(uploads::companion_window(context, provider, params).await);
    } else if !oauth_providers::is_user_provided(primary_backend) {
        activities.extend(uploads::every_user_provided_window(context, params).await);
    }
    merge_duplicates(activities, &DedupConfig::from_env())
}

/// Append every non-primary connection's rows for `params` to `activities`.
async fn fold_other_connections(
    context: &ToolExecutionContext,
    primary_backend: &str,
    params: &ActivityQueryParams,
    activities: &mut Vec<Activity>,
) {
    let tenant = context.tenant_id.map(TenantId::from_uuid);
    let Ok(connections) = context
        .resources
        .repos()
        .provider_connections
        .get_for_user(context.user_id, None)
        .await
    else {
        return;
    };

    for conn in &connections {
        let canonical = backend_resolver::resolve_backend(
            context.resources.repos(),
            context.user_id,
            tenant,
            &conn.provider,
        )
        .await;
        if canonical == primary_backend
            || connection_grant_beside_session(&context.resources, context.user_id, conn).await
        {
            continue;
        }
        if let Some(fetched) = fetch_provider_activities(
            &context.resources,
            &conn.provider,
            context.user_id,
            &conn.tenant_id,
            params,
        )
        .await
        {
            activities.extend(fetched);
        }
    }
}

/// Record that `provider` just served this athlete's data.
///
/// The election orders on `last_used_at` ahead of `connected_at`, so
/// this write is what makes the resolver mean "the backend the athlete is
/// actually training on" instead of "the connection added last". Called at the
/// serve chokepoint for the ELECTED provider only — touching every connection a
/// merge folded in would reduce the ordering to whichever fetch finished last.
/// Best-effort: a failed touch costs one election, never the answer.
pub async fn touch_connection_used(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant_id: TenantId,
    provider: &str,
) {
    if let Err(e) = runtime
        .repos()
        .provider_connections
        .touch_last_used(user_id, tenant_id, provider)
        .await
    {
        info!(
            user_id = %user_id,
            provider = %provider,
            error = %e,
            "provider connection touch failed; resolver ordering falls back to connected_at"
        );
    }
}

/// Lower bound of the disjoint head slice `(coverage_bound, now]` an
/// open-`before` historical serve must append to the coverage read.
///
/// The coverage read is clipped at `after + 1 year` so recent rows can't mask
/// a missing season — but rows above the clip are still inside the requested
/// window. Served without this slice, the list tops out a year above `after`
/// and the agent falsely reports "nothing newer" while newer rows sit in the
/// durable cache. `None` when the caller bounded `before` (nothing was
/// clipped) or the clip already reaches `now`.
/// `pub` so the slice decision is exercisable by the integration test suite.
#[must_use]
pub fn historical_head_slice(
    before: Option<i64>,
    coverage_bound: Option<i64>,
    now_ts: i64,
) -> Option<i64> {
    if before.is_some() {
        return None;
    }
    let bound = coverage_bound?;
    (bound < now_ts).then_some(bound)
}

/// How far back a stale-head refresh re-reads from the provider.
///
/// Thirty days, not the served window. The covered gate exists so a sixteen-week
/// ask is not re-scraped on every turn — a sciotte scrape is a headless browser
/// session costing tens of seconds — and the only thing a stale head can be
/// missing is recent. Deep history is immutable once backfilled.
const STALE_HEAD_REFRESH_DAYS: i64 = 30;

/// Why a provider's window was served from its cache instead of a live read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StaleReason {
    /// The live read outlived the turn's refresh budget; it finishes in the
    /// background and writes through for the next turn.
    Timeout,
    /// Its recent reads failed, so it is not read again until the backoff ends.
    Backoff,
    /// The live read failed.
    Error,
}

impl StaleReason {
    /// The machine-readable `reason` a caveat carries.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Timeout => "timeout",
            Self::Backoff => "backoff",
            Self::Error => "error",
        }
    }
}

/// Re-read a provider's recent head — the last [`STALE_HEAD_REFRESH_DAYS`] —
/// live, whatever its freshness, and write it through.
///
/// The read under [`refresh_stale_head`], for a caller that has already
/// decided the head needs it: the Home page refreshes a head whose last
/// refresh failed while it is still inside the freshness bands, and the
/// athlete's retry reads it inside the failure's pause. What the read
/// answers counts as a sync, or is recorded as a failed one, exactly as for
/// every other head read ([`fetch_provider_head`]); a failed read serves the
/// cached head, `None` when that is empty too.
pub async fn refresh_head(
    runtime: &Arc<dyn ToolRuntime>,
    provider_slug: &str,
    user_id: Uuid,
    tenant_id: TenantId,
) -> Option<Vec<Activity>> {
    let params = stale_head_params();
    fetch_provider_activities(
        runtime,
        provider_slug,
        user_id,
        &tenant_id.to_string(),
        &params,
    )
    .await
}

/// The window a stale head's re-read covers: the last
/// [`STALE_HEAD_REFRESH_DAYS`], up to [`STALE_HEAD_REFRESH_LIMIT`] rows.
pub(crate) fn stale_head_params() -> ActivityQueryParams {
    ActivityQueryParams {
        after: Some(Utc::now().timestamp() - STALE_HEAD_REFRESH_DAYS * 86_400),
        before: None,
        limit: Some(STALE_HEAD_REFRESH_LIMIT),
        offset: None,
    }
}

/// Fold a live provider read into a cache-served window: the live row wins.
///
/// A row the provider just returned is strictly newer than the cached copy of
/// the same activity, so it replaces that copy rather than being dropped as a
/// duplicate. Keeping the cached copy meant a turn that paid for a scrape
/// answered from the rows the scrape had just superseded: on 2026-09-18 a live
/// read returned 811 m of climb for a 26 km trail run, the cached copy carried
/// none, and the coach told the athlete elevation was "non dispo" for every
/// outing but the one activity the cache had never seen. The write-through had
/// already stored the fresh rows, so only that turn was wrong — which is what
/// made it look like the capture itself was still broken.
///
/// Ids the window has never seen are appended, as before. `pub` so the merge
/// is exercisable by the integration test suite, like
/// [`before_bounds_a_closed_window`].
pub fn merge_live_head(served: &mut Vec<Activity>, live: Vec<Activity>) {
    let position: HashMap<String, usize> = served
        .iter()
        .enumerate()
        .map(|(index, activity)| (activity.id().to_owned(), index))
        .collect();
    for fresh in live {
        match position.get(fresh.id()) {
            Some(&index) => served[index] = fresh,
            None => served.push(fresh),
        }
    }
    served.sort_by_key(|a| Reverse(a.start_date()));
}

/// Cap on rows a stale-head refresh reads back.
const STALE_HEAD_REFRESH_LIMIT: usize = 200;

/// How far behind `now` a `before` bound may sit and still count as an open head.
///
/// The model is told to pass `before` = now for any window question (today /
/// hier / cette semaine / ce mois), and the clock it reads is floored to a 300 s
/// quantum for prompt-cache stability, so "now" reaches this function already a
/// few minutes stale. An hour absorbs that without admitting a window the
/// athlete meant as closed.
const HEAD_OPEN_TOLERANCE_SECS: i64 = 3_600;

/// Whether `before` bounds a window that is genuinely closed — one no activity
/// recorded from here on can fall inside.
///
/// A closed window is the case [`refresh_stale_head`] exists to skip: topping up
/// the head cannot change an answer about 2022. A window ending at the *present*
/// is the opposite — it is the athlete asking what they have just done, and it is
/// exactly the shape the model produces for "what did I do this week".
///
/// The distinction was missing, and `before.is_some()` alone stood in for it. Since
/// the prompt instructs `before` = now for every window question, that guard made
/// the head top-up unreachable on the most common ask in the product: on
/// 2026-08-31 a "cette semaine" turn served 109 rows from a cache whose newest
/// activity was three days old, with `before` set to the turn's own timestamp, and
/// returned here before it could read the freshness it would have acted on.
///
/// `pub` so the decision is exercisable by the integration test suite, like
/// [`historical_head_slice`].
#[must_use]
pub fn before_bounds_a_closed_window(before: Option<i64>, now_ts: i64) -> bool {
    before.is_some_and(|b| b < now_ts - HEAD_OPEN_TOLERANCE_SECS)
}

/// Fetch activities for a single provider connection, authenticating (and
/// refreshing tokens) as needed.
///
/// A successful fetch is written through to the activity cache so a later
/// failure can be served from it. When the live fetch fails (provider needs
/// re-auth, transient scrape error, timeout), the user's cached activities for
/// the same window are served stale instead of an empty result — a provider
/// blip degrades to "slightly old data" rather than "no data". Returns `None`
/// only when the live fetch fails *and* the cache is empty.
///
/// A fetch the provider refused for authentication — a dead scrape session, a
/// revoked token — is not recorded as a failed sync (see
/// [`fetch_provider_head`]); it flags the connection `needs_reauth` and sends
/// the athlete's reconnect notice instead
/// ([`flag_needs_reauth`](reauth_notice::flag_needs_reauth)). Without
/// that, the Home refresh that met a dead session said nothing, recorded no
/// pause, and scraped the dead session again on every Home load. The flag
/// names the connection that refused (`reauth_notice::refused_backend`), and a
/// session already flagged is not read ([`reauth_retry::flagged_session`])
/// until a reconnect or a claimed retry: the live grant behind it serves in
/// its place (`fallback::grant_window`, carnet#1494), and only without one
/// does the cache.
///
/// A grant named beside the athlete's session is read as that session
/// (`pushed::serving_slug`, carnet#1494): its flag, cache and verdict are the
/// session's, never the grant's.
pub async fn fetch_provider_activities(
    runtime: &Arc<dyn ToolRuntime>,
    provider_slug: &str,
    user_id: Uuid,
    tenant_id: &str,
    params: &ActivityQueryParams,
) -> Option<Vec<Activity>> {
    let attempt_started_at = Utc::now();
    let tenant = TenantId::parse_str(tenant_id).ok();
    let provider_slug = &serving_slug(runtime, provider_slug, user_id, tenant).await;
    if let Some(served) =
        serve_flagged_session(runtime, provider_slug, user_id, tenant, params).await
    {
        return served;
    }
    match Box::pin(fetch_provider_head(
        runtime,
        provider_slug,
        user_id,
        tenant_id,
        params,
    ))
    .await
    {
        Ok(activities) => Some(activities),
        Err(e) => {
            let failed = FailedRead {
                provider_slug,
                user_id,
                tenant,
                params,
                attempt_started_at,
            };
            serve_after_failed_read(runtime, failed, &e).await
        }
    }
}

/// The window a read of a flagged scrape session is answered with, without
/// calling the scraper ([`reauth_retry::flagged_session`]): the live grant
/// behind it (`fallback::grant_window`, carnet#1494), else the cache; `None`
/// when the read may go ahead.
pub(crate) async fn serve_flagged_session(
    runtime: &Arc<dyn ToolRuntime>,
    provider_slug: &str,
    user_id: Uuid,
    tenant: Option<TenantId>,
    params: &ActivityQueryParams,
) -> Option<Option<Vec<Activity>>> {
    let tenant = tenant?;
    reauth_retry::flagged_session(runtime, user_id, tenant, provider_slug).await?;
    // The grant behind a flagged session serves; only without one does the cache.
    if let Some((_, rows)) = Box::pin(grant_window(
        runtime,
        provider_slug,
        user_id,
        tenant,
        params,
    ))
    .await
    {
        return Some(Some(rows));
    }
    Some(serve_stale_activities(runtime, provider_slug, user_id, tenant, params).await)
}

/// A live read that failed, for [`serve_after_failed_read`].
pub(crate) struct FailedRead<'a> {
    pub(crate) provider_slug: &'a str,
    pub(crate) user_id: Uuid,
    pub(crate) tenant: Option<TenantId>,
    pub(crate) params: &'a ActivityQueryParams,
    pub(crate) attempt_started_at: DateTime<Utc>,
}

/// Answer a live read that failed with `e` from the cache: the connection
/// that refused is flagged, then the athlete's cached window for the read is
/// served rather than nothing.
pub(crate) async fn serve_after_failed_read(
    runtime: &Arc<dyn ToolRuntime>,
    failed: FailedRead<'_>,
    e: &AppError,
) -> Option<Vec<Activity>> {
    let FailedRead {
        provider_slug,
        user_id,
        tenant,
        params,
        attempt_started_at,
    } = failed;
    // The connection that refused is the backend the error names, so a
    // dead username-and-password session is flagged, not the grant.
    let refused = reauth_notice::refused_backend(e, provider_slug);
    warn!(
        user_id = %user_id,
        provider = %provider_slug,
        refused = refused.as_deref(),
        error = %e,
        "fetch_provider_activities: live fetch failed; serving cache"
    );
    let tenant = tenant?;
    if let Some(refused) = refused {
        reauth_notice::flag_refused(
            runtime,
            user_id,
            tenant,
            &refused,
            FLAG_REASON,
            attempt_started_at,
        )
        .await;
    }
    serve_stale_activities(runtime, provider_slug, user_id, tenant, params).await
}

/// Authenticate the provider, fetch `params` live, and write the result through
/// to the durable cache — no cache fallback, no swallowed error.
///
/// [`fetch_provider_activities`] is this plus the read path's fallback. Serving
/// stale rows is right for an athlete waiting on an answer and wrong for a
/// caller whose whole job is to notice that the fetch failed: the nightly
/// capture sweep must tell an auth-shaped failure (a lapsed scrape session or a
/// 401, both surfacing as [`AppError::provider_auth_required`]) from a transient
/// one, because only the first may flag the athlete's connection.
///
/// The write-through and its fetch-freshness mark are the shared ones, so a
/// sweep fetch and a chat turn's fetch can never disagree about upsert, prune
/// or freshness.
///
/// Only an answer the provider vouched for counts as a sync. A read that
/// failed (other than for authentication, which the reconnect path reports),
/// a capture missing its list head, and an empty answer over a window the
/// cache holds activities in ([`implausibly_empty`](sync_verdict::implausibly_empty)) each leave freshness
/// where the last good fetch put it, and a read of the list head records the
/// attempt instead ([`record_sync_failure`](sync_verdict::record_sync_failure)): the head then still reads
/// stale, so a later Home load or turn refreshes it again once the failure's
/// pause is over ([`sync_backoff_until`](sync_verdict::sync_backoff_until)), and Home can say the sync failed.
/// On 2026-09-29 a scrape that failed answered `count=0` as a success, was
/// stamped fresh, and nothing looked again for four hours while the day's
/// activities sat unseen.
///
/// A read the provider answered with a list it did not reject also re-arms a
/// connection flagged `needs_reauth` ([`rearm_after_live_read`](sync_verdict::rearm_after_live_read)), for every
/// caller alike: the credential that just served is the one the flag doubted.
/// A grant named beside the athlete's session is read as that session
/// (`pushed::serving_slug`), so it never re-arms one backend from the other's
/// read or writes under the other's key. A read that must reach the named
/// backend exactly is [`read_exact_backend`].
///
/// # Errors
///
/// Returns [`AppError::provider_auth_required`] when the connection is
/// non-recoverably dead, an external-service error when the provider answered
/// with nothing over cached activities, and the provider's own error
/// otherwise.
pub async fn fetch_provider_head(
    runtime: &Arc<dyn ToolRuntime>,
    provider_slug: &str,
    user_id: Uuid,
    tenant_id: &str,
    params: &ActivityQueryParams,
) -> AppResult<Vec<Activity>> {
    let provider_slug = &serving_slug(
        runtime,
        provider_slug,
        user_id,
        TenantId::parse_str(tenant_id).ok(),
    )
    .await;
    // The verdict and the write-through see the provider's full answer — the
    // cache is the athlete's — and only what is handed back is filtered for a
    // model (a no-op outside one).
    let head = ai_scope::unfiltered(fetch_and_persist_head(
        runtime,
        provider_slug,
        user_id,
        tenant_id,
        params,
        Routing::Resolved,
    ))
    .await?;
    Ok(ai_scope::filter_activities(
        runtime.provider_registry().as_ref(),
        head,
    ))
}

/// How a head read picks the backend it authenticates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Routing {
    /// The backend the athlete's read plan serves the named slug by
    /// (`backend_resolver::resolve_backend`).
    Resolved,
    /// Exactly the named backend, with its own token row.
    Exact,
}

/// [`fetch_provider_head`] of exactly `backend`, never the backend the
/// athlete's read plan serves first: judged, recorded and written through
/// under `backend` alone.
///
/// For a read a provider's own event asks of the credential it names: a
/// Strava webhook and the first read of a connection just made reach the
/// grant, so a pushed event never starts a scrape of the session beside it
/// (carnet#1494). A push-only backend answers with its pushed window, as it
/// does on every read.
///
/// # Errors
///
/// As [`fetch_provider_head`].
pub async fn read_exact_backend(
    runtime: &Arc<dyn ToolRuntime>,
    backend: &str,
    user_id: Uuid,
    tenant_id: &str,
    params: &ActivityQueryParams,
) -> AppResult<Vec<Activity>> {
    let head = ai_scope::unfiltered(fetch_and_persist_head(
        runtime,
        backend,
        user_id,
        tenant_id,
        params,
        Routing::Exact,
    ))
    .await?;
    Ok(ai_scope::filter_activities(
        runtime.provider_registry().as_ref(),
        head,
    ))
}

/// The elected provider's live list read for `get_activities`: judged,
/// recorded and written through on the provider's full answer, then handed
/// back as a model may see it.
///
/// Only a `Complete` read of a tenant's connection is written: an incomplete
/// capture is served but never persisted (carnet#149, carnet#151), and the
/// rows are the provider's own, so `write_through_served_window` applies.
///
/// # Errors
///
/// The provider's own read error, unchanged.
pub async fn read_live_window(
    runtime: &Arc<dyn ToolRuntime>,
    provider: &dyn CoreFitnessProvider,
    live: LiveRead<'_>,
    tenant: Option<TenantId>,
) -> AppResult<HeadRead> {
    let LiveRead {
        provider_slug,
        user_id,
        params,
        ..
    } = live;
    // A push-only provider has no live list: its window is what it pushed.
    if runtime.provider_registry().is_push_only(provider.name()) {
        return pushed_head(runtime, provider.name(), user_id, tenant, params).await;
    }
    let read = ai_scope::unfiltered(judge_live_read(runtime, live, provider)).await;
    let Some(tenant) = tenant else {
        let mut head = read?;
        head.activities =
            ai_scope::filter_activities(runtime.provider_registry().as_ref(), head.activities);
        return Ok(head);
    };
    // A session read that falls back is answered by its grant (carnet#1494).
    let mut head = Box::pin(fallback::serve_head(runtime, live, tenant, read)).await?;
    let key = fallback::write_key(runtime, provider_slug, &head);
    if let (HeadVerdict::Complete, Some(key)) = (&head.verdict, key) {
        write_through_served_window(runtime, user_id, &tenant, key, params, &head.activities).await;
    }
    head.activities =
        ai_scope::filter_activities(runtime.provider_registry().as_ref(), head.activities);
    Ok(head)
}

async fn fetch_and_persist_head(
    runtime: &Arc<dyn ToolRuntime>,
    provider_slug: &str,
    user_id: Uuid,
    tenant_id: &str,
    params: &ActivityQueryParams,
    routing: Routing,
) -> AppResult<Vec<Activity>> {
    let tenant = TenantId::parse_str(tenant_id).ok();
    let pushed = match routing {
        Routing::Resolved => reads_pushed_store(runtime, provider_slug, user_id, tenant).await,
        Routing::Exact => runtime.provider_registry().is_push_only(provider_slug),
    };
    if pushed {
        return pushed_window(runtime, provider_slug, user_id, tenant, params).await;
    }
    let live = LiveRead {
        provider_slug,
        user_id,
        tenant_id: Some(tenant_id),
        params,
        attempt_started_at: Utc::now(),
    };
    let read = read_provider_head(runtime, live, routing).await;
    // A session read that falls back is answered by its grant (carnet#1494).
    let head = match (tenant, routing) {
        (Some(tenant), Routing::Resolved) => {
            Box::pin(fallback::serve_head(runtime, live, tenant, read)).await
        }
        (Some(tenant), Routing::Exact) => {
            record_head_outcome(runtime, user_id, tenant, provider_slug, params, &read).await;
            read
        }
        (None, _) => read,
    }?;
    if let HeadVerdict::EmptyOverCached { cached } = head.verdict {
        return Err(AppError::external_service(
            provider_slug,
            format!(
                "answered with no activities over a window the cache holds {cached} of its \
                 activities in; not counted as a sync"
            ),
        ));
    }
    if head.verdict == HeadVerdict::Incomplete {
        // A capture whose head the provider never saw is served, not persisted.
        // The write-through moves every row's `synced_at` and the fetch mark to
        // now, and `DataFreshness` then reads the days it missed as a quiet week
        // — the masking carnet#149 paid for. Leaving the cache untouched keeps
        // the next ask a live fetch, which is the only thing that can fill the
        // head.
        warn!(
            user_id = %user_id,
            provider = %provider_slug,
            count = head.activities.len(),
            "fetch_provider_head: capture is missing the list head; served without write-through"
        );
        return Ok(head.activities);
    }

    // Warm the stale-while-revalidate cache so the next outage serves these.
    if let (Some(tenant), Some(key)) = (tenant, fallback::write_key(runtime, provider_slug, &head))
    {
        let auth_service = AuthService::new(Arc::clone(runtime));
        write_through_activity_cache(
            &auth_service,
            user_id,
            tenant,
            key,
            &head.activities,
            WriteThrough {
                read: params,
                announces_sessions: true,
            },
        )
        .await;
    }
    Ok(head.activities)
}

/// The `[start, end]` of cached rows a read of `params` covers.
///
/// Honors the request's `before` upper bound. A historical query like "2022
/// races" (after=2022, before=2023) must read the bounded [after, before]
/// window — reading [after, now] would return recent rows that fall inside
/// the open window and mask whether the deep history is actually cached.
pub(crate) fn cached_window_bounds(
    params: &ActivityQueryParams,
    now: DateTime<Utc>,
) -> (DateTime<Utc>, DateTime<Utc>) {
    let end = params
        .before
        .and_then(|ts| Utc.timestamp_opt(ts, 0).single())
        .unwrap_or(now);
    let start = params
        .after
        .and_then(|ts| Utc.timestamp_opt(ts, 0).single())
        .unwrap_or_else(|| now - Duration::days(STALE_FALLBACK_WINDOW_DAYS));
    (start, end)
}

/// Read a provider's cached activities for the request window from the durable
/// activity cache, newest first.
///
/// Returns `None` when the cache is empty or the read fails — the caller then
/// treats the provider as "no activities". Shared by the stale-fallback path
/// and the historical-backfill gate (which serves a deep window straight from
/// cache once a prior backfill has populated it).
pub(crate) async fn read_cached_window(
    runtime: &Arc<dyn ToolRuntime>,
    provider_slug: &str,
    user_id: Uuid,
    tenant: TenantId,
    params: &ActivityQueryParams,
) -> Option<Vec<Activity>> {
    let (start, end) = cached_window_bounds(params, Utc::now());
    let limit = params
        .limit
        .and_then(|l| i64::try_from(l).ok())
        .unwrap_or(STALE_FALLBACK_LIMIT);

    match cached_history(runtime, provider_slug, user_id, tenant, (start, end), limit).await {
        // Emptiness is judged on the athlete's rows, so a window that holds
        // only withheld rows still reads as cached and starts no live fetch;
        // what is handed back is filtered for a model (a no-op outside one).
        Ok(cached) if !cached.is_empty() => Some(ai_scope::filter_activities(
            runtime.provider_registry().as_ref(),
            cached,
        )),
        Ok(_) => None,
        Err(e) => {
            warn!(
                user_id = %user_id,
                provider = %provider_slug,
                error = %e,
                "activity cache window read failed"
            );
            None
        }
    }
}

/// Every stored row a read of `provider_slug`'s window serves, newest first,
/// at most `limit`.
///
/// Its own key, and the other half of its pair's too
/// ([`history_store_keys`]): what a usr/pwd session walked
/// beside a grant, and what the grant stored beside a session, every
/// recording of one workout merged into one session. A dead Garmin grant
/// keeps serving what the session stored, as a live one does (carnet#1386).
pub(super) async fn cached_history(
    runtime: &Arc<dyn ToolRuntime>,
    provider_slug: &str,
    user_id: Uuid,
    tenant: TenantId,
    (start, end): (DateTime<Utc>, DateTime<Utc>),
    limit: i64,
) -> AppResult<Vec<Activity>> {
    let cache = &runtime.repos().activity_cache;
    let keys = history_store_keys(provider_slug);
    if let [key] = keys.as_slice() {
        return cache
            .get_cached_activities(user_id, &tenant, Some(key), start, end, limit)
            .await;
    }
    let mut rows = Vec::new();
    for key in keys {
        rows.extend(
            cache
                .get_cached_activities(user_id, &tenant, Some(key), start, end, limit)
                .await?,
        );
    }
    let (mut sessions, _) = merge_duplicates(rows, &DedupConfig::from_env());
    sessions.sort_by_key(|activity| Reverse(activity.start_date()));
    sessions.truncate(usize::try_from(limit).unwrap_or(usize::MAX));
    Ok(sessions)
}

/// The page of a provider's cached window `params` names: the window read by
/// [`read_cached_window`] with `params.offset` rows skipped, so a later page
/// served from the cache is that page and never the first one again. `None`
/// when the page holds nothing.
async fn read_cached_page(
    runtime: &Arc<dyn ToolRuntime>,
    provider_slug: &str,
    user_id: Uuid,
    tenant: TenantId,
    params: &ActivityQueryParams,
) -> Option<Vec<Activity>> {
    let skip = params.offset.unwrap_or(0);
    if skip == 0 {
        return read_cached_window(runtime, provider_slug, user_id, tenant, params).await;
    }
    let through_page = ActivityQueryParams {
        limit: Some(
            params
                .limit
                .unwrap_or_else(|| usize::try_from(STALE_FALLBACK_LIMIT).unwrap_or(usize::MAX))
                .saturating_add(skip),
        ),
        offset: None,
        ..params.clone()
    };
    let page: Vec<Activity> =
        read_cached_window(runtime, provider_slug, user_id, tenant, &through_page)
            .await?
            .into_iter()
            .skip(skip)
            .collect();
    (!page.is_empty()).then_some(page)
}

/// Read a provider's cached activities after a failed live fetch, newest first.
///
/// Thin wrapper over [`read_cached_page`] that logs the stale-serve. Only
/// invoked after a live fetch failure, so any non-empty result is strictly
/// better than the empty fallback. `pub` for the chat tool's primary read,
/// which lives behind the `tools-data` feature.
pub async fn serve_stale_activities(
    runtime: &Arc<dyn ToolRuntime>,
    provider_slug: &str,
    user_id: Uuid,
    tenant: TenantId,
    params: &ActivityQueryParams,
) -> Option<Vec<Activity>> {
    let cached = read_cached_page(runtime, provider_slug, user_id, tenant, params).await;
    if let Some(ref activities) = cached {
        warn!(
            user_id = %user_id,
            provider = %provider_slug,
            count = activities.len(),
            "fetch_provider_activities: live fetch failed, serving stale cached activities"
        );
    }
    cached
}

/// Fetch recent activities across all of a user's connected providers and
/// merge them into a single list.
///
/// `after_ts` is a Unix timestamp (seconds) lower bound; `limit_per_provider`
/// caps how many activities are pulled from each provider. Providers that
/// fail to authenticate or fetch are skipped (logged at `warn`) rather than
/// failing the whole call, and so is a grant beside a usr/pwd session of its
/// provider (`pushed::connection_grant_beside_session`): the session is read.
pub async fn fetch_recent_activities_all_providers(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant_id: &str,
    after_ts: i64,
    limit_per_provider: usize,
) -> Vec<Activity> {
    let connections = runtime
        .repos()
        .provider_connections
        .get_for_user(user_id, None)
        .await
        .unwrap_or_default();

    let params = ActivityQueryParams {
        limit: Some(limit_per_provider),
        offset: None,
        before: None,
        after: Some(after_ts),
    };

    let mut all = Vec::new();
    for connection in &connections {
        // A grant beside a session is the session's fallback: the session's
        // own connection reads the window, the grant's stored rows included
        // (`history_store_keys`).
        if connection_grant_beside_session(runtime, user_id, connection).await {
            continue;
        }
        if let Some(activities) =
            fetch_provider_activities(runtime, &connection.provider, user_id, tenant_id, &params)
                .await
        {
            all.extend(activities);
        }
    }
    all
}

/// Order a fetched activity list in place by the requested key.
///
/// Applied BEFORE the display limit so a "longest to shortest" ask keeps the
/// longest activities rather than the most recent. Recognized keys:
/// `date_desc` (default, newest first), `date_asc`, `distance_desc`,
/// `distance_asc`, `duration_desc`, `duration_asc`. An unknown value falls back
/// to `date_desc`. A missing distance sorts as 0 m, so an activity with no
/// recorded distance lands last on a `distance_desc` sort. `pub` so the ordering
/// is exercisable by the integration test suite.
pub fn sort_activities(activities: &mut [Activity], sort_by: &str) {
    // f64 distances have no total order (NaN), so the distance arms keep
    // `sort_by` with `partial_cmp`; the Ord-keyed arms (date, duration) use
    // `sort_by_key` to satisfy clippy::unnecessary_sort_by.
    let distance = |a: &Activity| a.distance_meters().unwrap_or(0.0);
    match sort_by {
        "date_asc" => activities.sort_by_key(Activity::start_date),
        "distance_desc" => activities.sort_by(|a, b| {
            distance(b)
                .partial_cmp(&distance(a))
                .unwrap_or(Ordering::Equal)
        }),
        "distance_asc" => activities.sort_by(|a, b| {
            distance(a)
                .partial_cmp(&distance(b))
                .unwrap_or(Ordering::Equal)
        }),
        "duration_desc" => activities.sort_by_key(|a| Reverse(a.duration_seconds())),
        "duration_asc" => activities.sort_by_key(Activity::duration_seconds),
        // "date_desc" and any unrecognized value: newest first (the historical
        // default that cached responses also produce).
        _ => activities.sort_by_key(|a| Reverse(a.start_date())),
    }
}

/// Oldest/newest activity date (`"YYYY-MM-DD"`) across `activities`, or `None`
/// when the slice is empty.
///
/// Captured over the FULL post-filter set BEFORE the display-limit truncation so
/// the response can frame the served window's true span — otherwise the LLM
/// anchors on the oldest activity in the truncated slice (e.g. "depuis le 21
/// août") instead of the window's real start.
///
/// Rendered in the athlete's timezone, because this span reaches them as prose:
/// the coverage note interpolates it into "spanning …". A bare date has no
/// offset to disambiguate it, so rendering the UTC day moves an evening session
/// to the next one — the same defect that had a 22:59 hike reported as "ce
/// matin" (2026-08-28). An absent or unparseable zone falls back to UTC.
#[must_use]
pub fn activity_date_span(
    activities: &[Activity],
    user_timezone: Option<&str>,
) -> Option<(NaiveDate, NaiveDate)> {
    let zone = resolve_zone(user_timezone);
    let oldest = activities.iter().map(Activity::start_date).min()?;
    let newest = activities.iter().map(Activity::start_date).max()?;
    Some((
        oldest.with_timezone(&zone).date_naive(),
        newest.with_timezone(&zone).date_naive(),
    ))
}

/// Sum a window's count, distance, duration and elevation gain.
#[must_use]
pub fn activity_window_totals(activities: &[Activity]) -> WindowTotals {
    let distance_m: f64 = activities
        .iter()
        .filter_map(Activity::distance_meters)
        .sum();
    let seconds: u64 = activities.iter().map(Activity::duration_seconds).sum();
    let elevation_m: f64 = activities.iter().filter_map(Activity::elevation_gain).sum();
    WindowTotals {
        count: u64::try_from(activities.len()).unwrap_or(u64::MAX),
        distance_km: distance_m / 1000.0,
        duration_seconds: seconds,
        elevation_gain_m: elevation_m,
    }
}
