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
use std::collections::{HashMap, HashSet};
use std::env;
use std::sync::Arc;

use chrono::{DateTime, Duration, TimeZone, Utc};
use pierre_core::civil_time::resolve_zone;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::refresh::DataFreshness;
use pierre_core::models::{Activity, ConnectionStatus, TenantId};
use pierre_database::repositories::BackfillCoverage;
use pierre_providers::core::ActivityQueryParams;
use pierre_providers::deduplication::{merge_duplicates, DedupConfig, FragmentReport};
use pierre_providers::CoreFitnessProvider;
use tracing::{info, warn};
use uuid::Uuid;

use crate::capture_sweep::FLAG_REASON;
use crate::context::ToolExecutionContext;
use crate::protocol::auth::AuthService;
use crate::protocol::reauth_notice::flag_needs_reauth;
use crate::protocol::types::{auth_required_provider, UniversalResponse};
use crate::runtime::ToolRuntime;
use pierre_providers::ai_scope;
use pierre_providers::backend_resolver;
use serde_json::Value;

/// The session a cache write lands, and the `per_session` persona digest it sends
#[cfg(feature = "client-notifications")]
mod session_landing;
/// Whether a live list read counts as a sync, and what a failed one records
pub mod sync_verdict;
/// The activity cache's write-through, and what a write says about freshness
pub mod write_through;

use sync_verdict::{
    judge_live_read, read_provider_head, record_head_outcome, sync_backoff_until, HeadRead,
    HeadVerdict, LiveRead,
};
use write_through::{write_through_activity_cache, write_through_served_window, WriteThrough};

/// Read limit for the single deterministic durable-cache read on the historical
/// backfill path. Defined beside the training-history read, which reads the
/// same windows; `pub` so the cap boundary is exercisable by the coverage-note
/// tests (re-exported through `implementations::data`, its original home).
pub use pierre_services::training_history_read::HISTORICAL_WINDOW_READ_LIMIT;

/// Cache fallback window when the request carries no `after` lower bound.
const STALE_FALLBACK_WINDOW_DAYS: i64 = 90;

/// Cap on stale rows served from cache when a live fetch fails.
const STALE_FALLBACK_LIMIT: i64 = 500;

/// Default retention + read window (days) for the provider-agnostic activity
/// cache when `PIERRE_ACTIVITY_CACHE_RETENTION_DAYS` is unset.
///
/// Sized so a cached read covers the training-load default window *and its
/// CTL warm-up*: `DEFAULT_BACKFILL_DAYS` (90) plus `warmup_days(42)` (72) is
/// 162 days, and `AthleteMetrics` asks for a 180-day lookback. At the previous
/// 90 both were silently truncated — the compute got a series warmed from a
/// zero seed and `recent_tsb` was read off a half-warmed curve.
///
/// This covers the default 90-day training-history window plus its 72-day CTL
/// warm-up, not every window `compute_and_persist_history` accepts —
/// `MAX_BACKFILL_DAYS` is 365, which would need 437 days retained. A deeper ask
/// is answered as partial coverage and re-requests a capture.
///
/// The prune in `write_through_activity_cache` is keyed per `(user, tenant)`
/// across all providers, so a later narrow writer reclaims a deep backfill's
/// rows back to this floor. `prune_and_realign_coverage` raises the coverage
/// floors in the same breath, so the historical gate re-fetches instead of
/// serving a cache the prune has emptied.
const DEFAULT_ACTIVITY_CACHE_RETENTION_DAYS: i64 = 180;

/// Resolve the activity-cache retention window (days) from the environment,
/// falling back to [`DEFAULT_ACTIVITY_CACHE_RETENTION_DAYS`].
///
/// This is both the
/// prune cutoff applied after a write-through and the lookback used when
/// reading cached rows, so widening it deepens the cache into a historical
/// store (e.g. to keep a backfilled season) at the cost of more rows retained.
/// Non-positive or unparseable values fall back to the default.
#[must_use]
pub fn activity_cache_retention_days() -> i64 {
    env::var("PIERRE_ACTIVITY_CACHE_RETENTION_DAYS")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .filter(|days| *days > 0)
        .unwrap_or(DEFAULT_ACTIVITY_CACHE_RETENTION_DAYS)
}

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
            &context.resources.repos().auth_repos(),
            context.user_id,
            tenant,
            &conn.provider,
        )
        .await;
        if canonical == primary_backend {
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

/// A window the athlete's other connections served after the elected primary
/// failed to authenticate, dead or only unreachable just now.
pub struct FallbackServe {
    /// The activities those connections produced, every recording of one
    /// workout merged into a single session.
    pub activities: Vec<Activity>,
    /// The groups that merge collapsed and the fields each session gained.
    pub merge_report: FragmentReport,
    /// User-facing names of the connections that contributed at least one row,
    /// so the response names its real sources instead of the dead provider.
    pub served_by: Vec<String>,
}

/// Serve the requested window from the athlete's OTHER connections when the
/// elected primary cannot authenticate.
///
/// A multi-source aggregator answers with what it holds: an athlete whose WHOOP
/// token died, or whose Strava refresh is rate limited, still has years of
/// history behind a healthy connection, and why the primary is missing belongs
/// BESIDE that answer rather than instead of it. The caller keeps that signal
/// (a reconnect prompt, or a "could not be reached" note) and attaches it as a
/// caveat; only when this returns `None` — no other connection produced a
/// single row — does the turn become the primary's failure alone.
///
/// Health-aware: a sibling already flagged `needs_reauth` is not fetched into the
/// same failure; its durable cache answers instead, exactly as
/// [`fetch_provider_activities`] answers the refusal that flagged it — so the
/// second read of a turn serves the rows the first one did, rather than losing
/// them to the flag the first read raised. A `revoked` sibling is skipped: the
/// athlete withdrew that access. Cross-tenant like the peer path, so
/// a provider connected under the athlete's own tenant answers from a group
/// conversation. A deep historical window on a scrape-backed mirror reads that
/// sibling's durable cache rather than scraping inline — the historical branch
/// exists precisely to keep a multi-year page out of the turn. The union is
/// merged the same way [`maybe_merge_other_connections`] merges it, so a
/// watch's misclassified twin of a GPS session collapses into it.
pub async fn serve_without_primary(
    context: &ToolExecutionContext,
    primary_backend: &str,
    is_historical: bool,
    params: &ActivityQueryParams,
) -> Option<FallbackServe> {
    let tenant = context.tenant_id.map(TenantId::from_uuid);
    let connections = context
        .resources
        .repos()
        .provider_connections
        .get_for_user(context.user_id, None)
        .await
        .ok()?;

    let mut served: Vec<Activity> = Vec::new();
    let mut served_by: Vec<String> = Vec::new();
    // Backends already asked, so two connection rows that resolve to the SAME
    // backend (an athlete holding both a `strava` row and the `sciotte` mirror
    // it resolves to) are fetched once instead of returning every session twice.
    let mut asked: Vec<String> = vec![primary_backend.to_owned()];
    for conn in &connections {
        if conn.status == ConnectionStatus::Revoked {
            continue;
        }
        let canonical = backend_resolver::resolve_backend(
            &context.resources.repos().auth_repos(),
            context.user_id,
            tenant,
            &conn.provider,
        )
        .await;
        if asked.contains(&canonical) {
            continue;
        }
        asked.push(canonical.clone());
        let fetched = if conn.status.requires_reauth() {
            let Ok(conn_tenant) = TenantId::parse_str(&conn.tenant_id) else {
                continue;
            };
            serve_stale_activities(
                &context.resources,
                &canonical,
                context.user_id,
                conn_tenant,
                params,
            )
            .await
        } else if is_historical && backend_resolver::is_mirror_backend(&canonical) {
            let Ok(conn_tenant) = TenantId::parse_str(&conn.tenant_id) else {
                continue;
            };
            read_cached_window(
                &context.resources,
                &canonical,
                context.user_id,
                conn_tenant,
                params,
            )
            .await
        } else {
            fetch_provider_activities(
                &context.resources,
                &conn.provider,
                context.user_id,
                &conn.tenant_id,
                params,
            )
            .await
        };
        if let Some(rows) = fetched {
            if !rows.is_empty() {
                served_by.push(backend_resolver::user_facing_name(&canonical).to_owned());
                served.extend(rows);
            }
        }
    }

    if served.is_empty() {
        return None;
    }
    let (served, merge_report) = merge_duplicates(served, &DedupConfig::from_env());
    warn!(
        user_id = %context.user_id,
        primary = %primary_backend,
        count = served.len(),
        served_by = %served_by.join(", "),
        "elected provider could not authenticate; serving the window from the athlete's other connections"
    );
    Some(FallbackServe {
        activities: served,
        merge_report,
        served_by,
    })
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

/// Whether a cached historical window is deep enough to serve as-is.
///
/// Cached rows alone are not enough — a prior limit-capped backfill leaves only
/// the recent slice of a deep window. The window is covered when a backfill
/// reached at least as far back as `after_ts`, OR exhausted the provider feed
/// (`hit_feed_end`, so no older data exists). No coverage record ⇒ not covered.
///
/// Depth is necessary, not sufficient: the record must also have been written
/// by the provider's current capture (`required_capture_version`, from
/// `pierre_core::constants::provider_capture`). A record below it vouches for
/// rows an older capture wrote — deep enough, and short of whatever the fix
/// started reading — so the window is re-captured instead of served. Neither
/// escape hatch outranks that: a feed the old capture exhausted was still read
/// by the old capture.
/// `pub` so the gate decision is exercisable by the integration test suite.
///
/// The record is kept honest at the source rather than second-guessed here: a
/// prune raises every coverage floor it just falsified
/// (`prune_and_realign_coverage`), so a claim can no longer outlive the rows it
/// vouches for. Comparing against the retention default instead would misjudge
/// exactly the caches worth trusting — a deep backfill widens its own retention
/// via `backfill_retention_days`, so its rows legitimately sit below the
/// default floor.
#[must_use]
pub fn historical_depth_covered(
    coverage: Option<BackfillCoverage>,
    after_ts: i64,
    required_capture_version: u32,
) -> bool {
    coverage.is_some_and(|c| {
        c.capture_version >= required_capture_version
            && (c.hit_feed_end || c.oldest_reached_ts <= after_ts)
    })
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

/// Top up a cache-served window with a live read when the head has gone stale.
///
/// The covered-historical path serves the durable cache with no freshness test:
/// `read_cached_window` consults neither `synced_at` nor `activity_fetch_freshness`,
/// and `historical_depth_covered` asks only whether the window is DEEP enough,
/// never whether it is CURRENT. An agent whose window declares sixteen weeks takes
/// that path on every single turn, so its grounding block was as old as whatever
/// last happened to write through — while the block itself instructs the model to
/// "base your analysis on these specific activities" and not to answer from memory.
///
/// The freshness mark was already written and already read, but only by the
/// `get_data_freshness` REPORTING tool. Nothing acted on it. Now the same
/// [`DataFreshness`] bands the agent is TOLD about also decide whether to look
/// again, so the report and the behaviour cannot disagree.
///
/// Bounded windows are exempt: a closed `before` names a period that is over, so
/// there is no live head it could be missing, and refreshing one would re-scrape
/// history that cannot have changed.
///
/// A head whose last refreshes failed is read again only once
/// [`sync_backoff_until`](sync_verdict::sync_backoff_until) allows it: a provider that keeps failing is not
/// scraped on every turn and every Home load.
///
/// Best-effort. A failed refresh leaves the cached window exactly as it was —
/// slightly old data beats no data, which is the same posture
/// [`fetch_provider_activities`] already takes on a provider blip.
pub async fn refresh_stale_head(
    runtime: &Arc<dyn ToolRuntime>,
    provider_slug: &str,
    user_id: Uuid,
    tenant_id: TenantId,
    before: Option<i64>,
    served: &mut Vec<Activity>,
) {
    if before_bounds_a_closed_window(before, Utc::now().timestamp()) {
        return;
    }

    let last_sync = runtime
        .repos()
        .activity_cache
        .latest_activity_sync(user_id, &tenant_id, provider_slug)
        .await
        .unwrap_or(None);

    let freshness = DataFreshness::from_last_sync(last_sync);
    if matches!(freshness, DataFreshness::Fresh | DataFreshness::Recent) {
        return;
    }
    let failure = runtime
        .repos()
        .activity_cache
        .latest_activity_fetch_failure(user_id, &tenant_id, provider_slug)
        .await
        .unwrap_or(None);
    if let Some(retry_at) = sync_backoff_until(failure, last_sync, Utc::now()) {
        info!(
            user_id = %user_id,
            provider = %provider_slug,
            %retry_at,
            "activity head is stale, but its last refreshes failed; backing off before reading it again"
        );
        return;
    }

    info!(
        user_id = %user_id,
        provider = %provider_slug,
        ?last_sync,
        freshness = freshness.label(),
        "activity head is stale; re-reading the recent window before grounding"
    );
    let Some(live) = refresh_head(runtime, provider_slug, user_id, tenant_id).await else {
        return;
    };

    merge_live_head(served, live);
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
    let params = ActivityQueryParams {
        after: Some(Utc::now().timestamp() - STALE_HEAD_REFRESH_DAYS * 86_400),
        before: None,
        limit: Some(STALE_HEAD_REFRESH_LIMIT),
        offset: None,
    };
    fetch_provider_activities(
        runtime,
        provider_slug,
        user_id,
        &tenant_id.to_string(),
        &params,
    )
    .await
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
/// [`historical_depth_covered`] and [`historical_head_slice`].
#[must_use]
pub fn before_bounds_a_closed_window(before: Option<i64>, now_ts: i64) -> bool {
    before.is_some_and(|b| b < now_ts - HEAD_OPEN_TOLERANCE_SECS)
}

/// Assemble the served list for a covered historical window.
///
/// Appends the disjoint open-`before` head slice above the coverage clip (see
/// [`historical_head_slice`] — disjoint windows can't reintroduce the
/// probe/serve divergence the single bounded read prevents; boundary rows can
/// land in both inclusive reads, so the id filter keeps the union exact), then
/// tops up a stale head from the provider via [`refresh_stale_head`]. Shared
/// by the coverage-gated serve and the inline-backfill (task handle) serve so
/// the two cannot drift. `pub` (not `pub(crate)`) because its only caller
/// lives behind the `tools-data` feature and a featureless build must not
/// read it as dead code.
pub async fn serve_historical_window(
    runtime: &Arc<dyn ToolRuntime>,
    provider_slug: &str,
    user_id: Uuid,
    tenant_id: TenantId,
    before: Option<i64>,
    coverage_before: Option<i64>,
    mut served: Vec<Activity>,
) -> Vec<Activity> {
    if let Some(head_after) = historical_head_slice(before, coverage_before, Utc::now().timestamp())
    {
        let head_params = ActivityQueryParams {
            after: Some(head_after),
            before: None,
            limit: Some(HISTORICAL_WINDOW_READ_LIMIT),
            offset: None,
        };
        if let Some(head) =
            read_cached_window(runtime, provider_slug, user_id, tenant_id, &head_params).await
        {
            let seen: HashSet<String> = served.iter().map(|a| a.id().to_owned()).collect();
            served.extend(head.into_iter().filter(|a| !seen.contains(a.id())));
        }
    }
    // The covered gate proves the window is deep enough; it says nothing about
    // whether it is current. Top up the head when the provider may be holding
    // something we have never seen.
    refresh_stale_head(
        runtime,
        provider_slug,
        user_id,
        tenant_id,
        before,
        &mut served,
    )
    .await;
    served
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
/// the athlete's reconnect notice instead ([`flag_needs_reauth`]). Without
/// that, the Home refresh that met a dead session said nothing, recorded no
/// pause, and scraped the dead session again on every Home load.
pub async fn fetch_provider_activities(
    runtime: &Arc<dyn ToolRuntime>,
    provider_slug: &str,
    user_id: Uuid,
    tenant_id: &str,
    params: &ActivityQueryParams,
) -> Option<Vec<Activity>> {
    let attempt_started_at = Utc::now();
    match fetch_provider_head(runtime, provider_slug, user_id, tenant_id, params).await {
        Ok(activities) => Some(activities),
        Err(e) => {
            let auth_required = e.provider_auth_required_provider().is_some();
            warn!(
                user_id = %user_id,
                provider = %provider_slug,
                error = %e,
                auth_required,
                "fetch_provider_activities: live fetch failed; serving cache"
            );
            // Live fetch failed — serve the user's cached activities for this
            // window rather than returning nothing.
            let tenant = TenantId::parse_str(tenant_id).ok()?;
            if auth_required {
                if let Err(flag_error) = flag_needs_reauth(
                    runtime,
                    user_id,
                    tenant,
                    provider_slug,
                    FLAG_REASON,
                    attempt_started_at,
                )
                .await
                {
                    warn!(
                        user_id = %user_id,
                        provider = %provider_slug,
                        error = %flag_error,
                        "fetch_provider_activities: could not flag the refused connection"
                    );
                }
            }
            serve_stale_activities(runtime, provider_slug, user_id, tenant, params).await
        }
    }
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
    // The verdict and the write-through see the provider's full answer — the
    // cache is the athlete's — and only what is handed back is filtered for a
    // model (a no-op outside one).
    let head = ai_scope::unfiltered(fetch_and_persist_head(
        runtime,
        provider_slug,
        user_id,
        tenant_id,
        params,
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
    let read = ai_scope::unfiltered(judge_live_read(runtime, live, provider)).await;
    if let Some(tenant) = tenant {
        record_head_outcome(runtime, user_id, tenant, provider_slug, params, &read).await;
    }
    let mut head = read?;
    if let (HeadVerdict::Complete, Some(tenant)) = (&head.verdict, tenant) {
        write_through_served_window(
            runtime,
            user_id,
            &tenant,
            provider_slug,
            params,
            &head.activities,
        )
        .await;
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
) -> AppResult<Vec<Activity>> {
    let tenant = TenantId::parse_str(tenant_id).ok();
    let read = read_provider_head(runtime, provider_slug, user_id, tenant_id, params).await;
    if let Some(tenant) = tenant {
        record_head_outcome(runtime, user_id, tenant, provider_slug, params, &read).await;
    }
    let head = read?;
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
    if let Some(tenant) = tenant {
        let auth_service = AuthService::new(Arc::clone(runtime));
        write_through_activity_cache(
            &auth_service,
            user_id,
            tenant,
            provider_slug,
            &head.activities,
            WriteThrough {
                retention_days: activity_cache_retention_days(),
                read: params,
            },
        )
        .await;
    }
    Ok(head.activities)
}

/// Type a failed `create_authenticated_provider`, preserving the auth shape.
///
/// That call tags a non-recoverably dead connection with
/// the auth-required provider slug — the one signal that says the athlete must
/// reconnect. Everything else it can fail with (an unsupported provider, a
/// tenant missing OAuth credentials, a transport blip) is not a dead session
/// and must never flag a connection, so it degrades to a plain external-service
/// error the sweep treats as transient.
pub(crate) fn provider_auth_failure(provider_slug: &str, response: &UniversalResponse) -> AppError {
    if auth_required_provider(response).is_some() {
        return AppError::provider_auth_required(provider_slug);
    }
    AppError::external_service(
        provider_slug,
        response
            .error
            .clone()
            .unwrap_or_else(|| "provider authentication failed".to_owned()),
    )
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

    match runtime
        .repos()
        .activity_cache
        .get_cached_activities(user_id, &tenant, Some(provider_slug), start, end, limit)
        .await
    {
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
/// failing the whole call.
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
/// `activity_coverage_note` interpolates it into "spanning …". A bare date has no
/// offset to disambiguate it, so rendering the UTC day moves an evening session
/// to the next one — the same defect that had a 22:59 hike reported as "ce
/// matin" (2026-08-28). An absent or unparseable zone falls back to UTC.
#[must_use]
pub fn activity_date_span(
    activities: &[Activity],
    user_timezone: Option<&str>,
) -> Option<(String, String)> {
    let zone = resolve_zone(user_timezone);
    let oldest = activities.iter().map(Activity::start_date).min()?;
    let newest = activities.iter().map(Activity::start_date).max()?;
    Some((
        oldest.with_timezone(&zone).format("%Y-%m-%d").to_string(),
        newest.with_timezone(&zone).format("%Y-%m-%d").to_string(),
    ))
}
