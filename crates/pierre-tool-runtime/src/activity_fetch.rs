// ABOUTME: Shared helper to fetch a user's recent activities from their connected providers
// ABOUTME: One auth+fetch path reused by group snapshots and coach recommendations — no duplication
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Provider activity fetching shared across route crates.
//!
//! Authenticating a provider (refreshing OAuth tokens, resolving sciotte
//! mirrors) and pulling activities is identical whether the caller is the
//! group analytics snapshot builder or the coach recommender. This module
//! owns that single path so neither re-implements it.

use std::cmp::{Ordering, Reverse};
use std::collections::HashSet;
use std::env;
use std::sync::Arc;

use chrono::{Duration, TimeZone, Utc};
use pierre_core::models::refresh::DataFreshness;
use pierre_core::models::{Activity, TenantId};
use pierre_database::repositories::{ActivityCacheRepository, BackfillCoverage};
use pierre_providers::core::ActivityQueryParams;
use tracing::{info, warn};
use uuid::Uuid;

use crate::activity_dedup::{ActivityDeduplicator, TimeWindowDeduplicator};
use crate::context::ToolExecutionContext;
use crate::protocol::auth::AuthService;
use crate::runtime::ToolRuntime;
use pierre_providers::backend_resolver;
use serde_json::Value;

/// Cache fallback window when the request carries no `after` lower bound.
const STALE_FALLBACK_WINDOW_DAYS: i64 = 90;

/// Cap on stale rows served from cache when a live fetch fails.
const STALE_FALLBACK_LIMIT: i64 = 500;

/// Default retention + read window (days) for the provider-agnostic activity
/// cache when `PIERRE_ACTIVITY_CACHE_RETENTION_DAYS` is unset. Exceeds the
/// training-load lookback so cached reads always cover CTL/ATL/TSB.
const DEFAULT_ACTIVITY_CACHE_RETENTION_DAYS: i64 = 90;

/// Resolve the activity-cache retention window (days) from the environment,
/// falling back to [`DEFAULT_ACTIVITY_CACHE_RETENTION_DAYS`]. This is both the
/// prune cutoff applied after a write-through and the lookback used when
/// reading cached rows, so widening it deepens the cache into a historical
/// store (e.g. to keep a backfilled season) at the cost of more rows retained.
/// Non-positive or unparseable values fall back to the default.
#[must_use]
pub(crate) fn activity_cache_retention_days() -> i64 {
    env::var("PIERRE_ACTIVITY_CACHE_RETENTION_DAYS")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .filter(|days| *days > 0)
        .unwrap_or(DEFAULT_ACTIVITY_CACHE_RETENTION_DAYS)
}

/// Fold the athlete's OTHER connected providers into a primary fetch.
///
/// No-op when the caller pinned an explicit `provider` argument or the ask
/// is the coverage-gated historical branch; otherwise every remaining
/// connection is fetched and the union deduplicated.
///
/// `resolve_provider_for_tool` picks ONE provider (arg, env, or most recently
/// used connection), which shows a multi-provider athlete only a slice of
/// their training: on 2026-08-22 a most-recently-used WHOOP connection hid a
/// 200km Strava ride behind WHOOP's distance-less, misclassified "run" record
/// of the same session. This reuses the peer tool's cache-degrading
/// [`fetch_provider_activities`] per remaining connection (cross-tenant, like
/// the peer path, so a provider connected under the athlete's own tenant
/// resolves from a group conversation) and the snapshot's deduplicator, whose
/// `pick_best` keeps the GPS row and whose cross-sport rule pairs a watch's
/// misclassified sport with the GPS provider's record of the same workout.
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
) -> Vec<Activity> {
    let explicit_provider_arg = args
        .get("provider")
        .and_then(Value::as_str)
        .is_some_and(|s| !s.is_empty());
    if explicit_provider_arg || is_historical {
        return activities;
    }

    let tenant = context.tenant_id.map(TenantId::from_uuid);
    let Ok(connections) = context
        .resources
        .repos()
        .provider_connections
        .get_for_user(context.user_id, None)
        .await
    else {
        return activities;
    };

    let mut merged_any = false;
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
            if !fetched.is_empty() {
                merged_any = true;
            }
            activities.extend(fetched);
        }
    }

    if merged_any {
        activities = TimeWindowDeduplicator::from_env().deduplicate(activities);
    }
    activities
}

/// A window the athlete's other connections served after the elected primary
/// failed to authenticate.
pub struct FallbackServe {
    /// The merged (and, when more than one connection contributed, deduplicated)
    /// activities those connections produced.
    pub activities: Vec<Activity>,
    /// User-facing names of the connections that contributed at least one row,
    /// so the response names its real sources instead of the dead provider.
    pub served_by: Vec<String>,
}

/// Serve the requested window from the athlete's OTHER connections when the
/// elected primary cannot authenticate.
///
/// A multi-source aggregator answers with what it holds: an athlete whose WHOOP
/// token died still has years of Strava behind a healthy connection, and the
/// reconnect prompt belongs BESIDE that answer rather than instead of it. The
/// caller keeps the reconnect signal and attaches it as a caveat; only when this
/// returns `None` — no other connection produced a single row — does the turn
/// become the reconnect message alone.
///
/// Health-aware: a sibling already flagged `needs_reauth`/`revoked` is skipped
/// instead of fetched into the same failure. Cross-tenant like the peer path, so
/// a provider connected under the athlete's own tenant answers from a group
/// conversation. A deep historical window on a scrape-backed mirror reads that
/// sibling's durable cache rather than scraping inline — the historical branch
/// exists precisely to keep a multi-year page out of the turn. The union is
/// deduplicated whenever more than one connection contributed, so a watch's
/// misclassified twin of a GPS session collapses the same way the merge path
/// collapses it.
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
        if conn.status.requires_reauth() {
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
        let fetched = if is_historical && backend_resolver::is_mirror_backend(&canonical) {
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
    if served_by.len() > 1 {
        served = TimeWindowDeduplicator::from_env().deduplicate(served);
    }
    warn!(
        user_id = %context.user_id,
        dead_provider = %primary_backend,
        count = served.len(),
        served_by = %served_by.join(", "),
        "elected provider needs re-auth; serving the window from the athlete's other connections"
    );
    Some(FallbackServe {
        activities: served,
        served_by,
    })
}

/// Record that `provider` just served this athlete's data.
///
/// `resolve_most_recent` orders on `last_used_at` ahead of `connected_at`, so
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
/// `pub` so the gate decision is exercisable by the integration test suite.
#[must_use]
pub fn historical_depth_covered(coverage: Option<BackfillCoverage>, after_ts: i64) -> bool {
    coverage.is_some_and(|c| c.hit_feed_end || c.oldest_reached_ts <= after_ts)
}

/// Lower bound of the disjoint head slice `(coverage_bound, now]` an
/// open-`before` historical serve must append to the coverage read.
///
/// The coverage read is clipped at `after + 1 year` so recent rows can't mask
/// a missing season — but rows above the clip are still inside the requested
/// window. Served without this slice, the list tops out a year above `after`
/// and the coach falsely reports "nothing newer" while newer rows sit in the
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
/// never whether it is CURRENT. A coach whose window declares sixteen weeks takes
/// that path on every single turn, so its grounding block was as old as whatever
/// last happened to write through — while the block itself instructs the model to
/// "base your analysis on these specific activities" and not to answer from memory.
///
/// The freshness mark was already written and already read, but only by the
/// `get_data_freshness` REPORTING tool. Nothing acted on it. Now the same
/// [`DataFreshness`] bands the coach is TOLD about also decide whether to look
/// again, so the report and the behaviour cannot disagree.
///
/// Bounded windows are exempt: a closed `before` names a period that is over, so
/// there is no live head it could be missing, and refreshing one would re-scrape
/// history that cannot have changed.
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

    let head_after = Utc::now().timestamp() - STALE_HEAD_REFRESH_DAYS * 86_400;
    info!(
        user_id = %user_id,
        provider = %provider_slug,
        ?last_sync,
        freshness = freshness.label(),
        "activity head is stale; re-reading the recent window before grounding"
    );

    let params = ActivityQueryParams {
        after: Some(head_after),
        before: None,
        limit: Some(STALE_HEAD_REFRESH_LIMIT),
        offset: None,
    };
    let Some(live) = fetch_provider_activities(
        runtime,
        provider_slug,
        user_id,
        &tenant_id.to_string(),
        &params,
    )
    .await
    else {
        return;
    };

    let seen: HashSet<String> = served.iter().map(|a| a.id().to_owned()).collect();
    let added = live.into_iter().filter(|a| !seen.contains(a.id()));
    served.extend(added);
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

/// Fetch activities for a single provider connection, authenticating (and
/// refreshing tokens) as needed.
///
/// A successful fetch is written through to the activity cache so a later
/// failure can be served from it. When the live fetch fails (provider needs
/// re-auth, transient scrape error, timeout), the user's cached activities for
/// the same window are served stale instead of an empty result — a provider
/// blip degrades to "slightly old data" rather than "no data". Returns `None`
/// only when the live fetch fails *and* the cache is empty.
pub async fn fetch_provider_activities(
    runtime: &Arc<dyn ToolRuntime>,
    provider_slug: &str,
    user_id: Uuid,
    tenant_id: &str,
    params: &ActivityQueryParams,
) -> Option<Vec<Activity>> {
    let auth_service = AuthService::new(Arc::clone(runtime));
    let tenant = TenantId::parse_str(tenant_id).ok();

    let live = match auth_service
        .create_authenticated_provider(provider_slug, user_id, Some(tenant_id))
        .await
    {
        Ok(provider) => provider
            .get_activities_with_params(params)
            .await
            .map_err(|e| {
                warn!(
                    user_id = %user_id,
                    provider = %provider_slug,
                    error = %e,
                    "fetch_provider_activities: failed to fetch activities"
                );
            })
            .ok(),
        Err(e) => {
            warn!(
                user_id = %user_id,
                provider = %provider_slug,
                error = ?e.error,
                "fetch_provider_activities: failed to create authenticated provider"
            );
            None
        }
    };

    if let Some(activities) = live {
        // Warm the stale-while-revalidate cache so the next outage serves these.
        if let Some(tenant) = tenant {
            write_through_activity_cache(
                &auth_service,
                user_id,
                tenant,
                provider_slug,
                &activities,
                activity_cache_retention_days(),
            )
            .await;
        }
        return Some(activities);
    }

    // Live fetch failed — serve the user's cached activities for this window
    // rather than returning nothing.
    serve_stale_activities(runtime, provider_slug, user_id, tenant?, params).await
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
    let now = Utc::now();
    // Honor the request's `before` upper bound. A historical query like
    // "2022 races" (after=2022, before=2023) must read the bounded [after,
    // before] window — reading [after, now] would return recent rows that fall
    // inside the open window and mask whether the deep history is actually
    // cached.
    let end = params
        .before
        .and_then(|ts| Utc.timestamp_opt(ts, 0).single())
        .unwrap_or(now);
    let start = params
        .after
        .and_then(|ts| Utc.timestamp_opt(ts, 0).single())
        .unwrap_or_else(|| now - Duration::days(STALE_FALLBACK_WINDOW_DAYS));
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
        Ok(cached) if !cached.is_empty() => Some(cached),
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

/// Read a provider's cached activities after a failed live fetch, newest first.
///
/// Thin wrapper over [`read_cached_window`] that logs the stale-serve. Only
/// invoked after a live fetch failure, so any non-empty result is strictly
/// better than the empty fallback.
async fn serve_stale_activities(
    runtime: &Arc<dyn ToolRuntime>,
    provider_slug: &str,
    user_id: Uuid,
    tenant: TenantId,
    params: &ActivityQueryParams,
) -> Option<Vec<Activity>> {
    let cached = read_cached_window(runtime, provider_slug, user_id, tenant, params).await;
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

/// Warm the provider-agnostic activity cache after a successful live fetch so
/// the next outage serves these rows stale-while-revalidate. Shared with the
/// group snapshot builder through the `activity_cache` repo.
///
/// `retention_days` is the per-transaction prune window: after the upsert,
/// rows older than `now - retention_days` are garbage-collected. Recent-fetch
/// callers pass [`activity_cache_retention_days`] (the deployment default);
/// a historical backfill passes a deeper window so the season it just wrote is
/// not immediately pruned. Pruning is keyed by `(user_id, tenant_id)` across
/// all providers, so the retention floor for durable history is whatever the
/// *widest-window* writer uses.
/// Returns the count of net distinct rows persisted (deduped by `activity_id`),
/// or `None` when the upsert itself failed. The historical backfill surfaces
/// this honest figure in its completion notice; recent-fetch callers ignore it.
pub(crate) async fn write_through_activity_cache(
    auth_service: &AuthService,
    user_id: Uuid,
    tenant_id: TenantId,
    provider: &str,
    activities: &[Activity],
    retention_days: i64,
) -> Option<u64> {
    let data = auth_service.runtime().data();
    let cache = data.repos().activity_cache.clone();
    let persisted = match cache
        .upsert_activities(user_id, &tenant_id, provider, activities)
        .await
    {
        Ok(count) => count,
        Err(e) => {
            info!(user_id = %user_id, provider = %provider, error = %e, "Activity cache: write-through failed");
            return None;
        }
    };
    let cutoff = Utc::now() - Duration::days(retention_days);
    if let Err(e) = cache
        .prune_activities_before(user_id, &tenant_id, cutoff)
        .await
    {
        info!(user_id = %user_id, provider = %provider, error = %e, "Activity cache: prune failed");
    }
    stamp_fetch_freshness(cache.as_ref(), user_id, tenant_id, provider).await;
    Some(persisted)
}

/// Record that this fetch happened, independent of how many rows it returned.
///
/// The fetch itself is the freshness signal: a provider that answered with
/// zero activities is exactly as current as one that answered with ten, and
/// the upserted rows cannot say so. Best-effort — a failed mark costs one
/// deferred freshness read, never the fetch.
async fn stamp_fetch_freshness(
    cache: &dyn ActivityCacheRepository,
    user_id: Uuid,
    tenant_id: TenantId,
    provider: &str,
) {
    if let Err(e) = cache
        .record_activity_fetch(user_id, &tenant_id, provider, Utc::now())
        .await
    {
        info!(user_id = %user_id, provider = %provider, error = %e, "Activity cache: fetch freshness mark failed");
    }
}

/// Persist a window `get_activities` just served.
///
/// So the stale-while-revalidate path and later turns can answer it without
/// re-fetching. Best-effort: a cache failure never blocks the response. An empty
/// window writes nothing.
///
/// The caller decides provenance, and only rows a PROVIDER produced belong here.
/// Two kinds never do:
///
/// * Rows a sibling connection produced while the elected provider was auth-dead.
///   They belong to the providers that produced them — each already wrote its own
///   through — and filing them under the dead provider's key would have a
///   reconnect restore history it never recorded.
/// * Rows the historical branch read out of this very table. Writing them back
///   changes no data; it only moves their `synced_at` to now.
///   [`ActivityCacheRepository::latest_activity_sync`] takes the max of that
///   column, [`DataFreshness`] reads the result as `Fresh`, and
///   [`refresh_stale_head`] returns early on `Fresh` — so the one path that would
///   top the head up is disarmed by the act of serving the stale window, and every
///   later ask re-arms the disarming. A capture that stops then reports itself
///   current forever: jf@dravr.ai's sciotte capture froze at 2026-08-28 02:59Z and
///   two days later all 109 cached rows carried one identical `synced_at`, while
///   `activity_fetch_freshness` still held the last real fetch, five days older.
///   A stale-head top-up that does reach the provider is written through by
///   [`fetch_provider_activities`], which is what should move freshness.
///
/// `pub` like its siblings here: the only caller is `implementations::data`, which is
/// behind the `tools-data` feature, so a narrower visibility reads as dead code in a
/// `--no-default-features` build.
pub async fn write_through_served_window(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant_id: &TenantId,
    provider_slug: &str,
    activities: &[Activity],
) {
    if activities.is_empty() {
        return;
    }
    if let Err(e) = runtime
        .repos()
        .activity_cache
        .upsert_activities(user_id, tenant_id, provider_slug, activities)
        .await
    {
        warn!(
            user_id = %user_id,
            provider = %provider_slug,
            error = %e,
            "Activity cache: write-through from get_activities failed"
        );
    }
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
    let zone = user_timezone
        .and_then(|tz| tz.parse::<chrono_tz::Tz>().ok())
        .unwrap_or(chrono_tz::UTC);
    let oldest = activities.iter().map(Activity::start_date).min()?;
    let newest = activities.iter().map(Activity::start_date).max()?;
    Some((
        oldest.with_timezone(&zone).format("%Y-%m-%d").to_string(),
        newest.with_timezone(&zone).format("%Y-%m-%d").to_string(),
    ))
}
