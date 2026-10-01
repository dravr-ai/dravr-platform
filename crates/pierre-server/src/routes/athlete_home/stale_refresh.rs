// ABOUTME: Home's background refresh of stale provider heads: which providers to refresh, the refresh itself
// ABOUTME: and the failure recorded when it outlives its bound
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::{
    claim_scrape_session_retry, debug, into_runtime, record_sync_failure, refresh_head,
    retries_flagged_session, revalidation_timeout, sync_backoff_until, timeout, user_facing_name,
    warn, ActivityFetchFailure, AppResult, Arc, ConnectionStatus, DataFreshness, DateTime,
    ProviderConnection, RevalidationRegistry, ServerContext, SyncFailure, TenantId, ToolRuntime,
    Utc, Uuid,
};
use pierre_providers::backend_resolver::brand_name;

/// Whether a last successful fetch is past the bands the chat path refreshes
/// at: the same test `refresh_stale_head` applies per provider.
fn is_stale(as_of: Option<DateTime<Utc>>) -> bool {
    !matches!(
        DataFreshness::from_last_sync(as_of),
        DataFreshness::Fresh | DataFreshness::Recent
    )
}

/// The providers a Home load would refresh, each judged by its own last
/// successful fetch.
#[derive(Debug, Default)]
pub(super) struct StaleRefreshPlan {
    /// Active connections whose own head needs a refresh.
    active: Vec<String>,
    /// Scrape sessions flagged `needs_reauth` whose own head needs a refresh:
    /// each is refreshed only when its throttled retry can be claimed.
    flagged: Vec<String>,
    /// The newest failed refresh among the judged providers that no good
    /// sync of the same provider has superseded.
    pub(super) failure: Option<SyncFailure>,
}

/// Judge each of the athlete's connections by its own last successful fetch,
/// and find the newest refresh of theirs that failed since.
///
/// Per provider, not across them: the newest fetch of any provider says
/// nothing about another's, and judging by it let a fresh connection hide a
/// stale one for as long as the fresh one kept syncing. An active connection
/// is refreshed when its head is stale, or when its last refresh failed —
/// whoever made that refresh, a Home load, a chat turn or the webhook, the
/// head it did not bring in is missing — unless that failure's pause is not
/// over ([`sync_backoff_until`]) and this is not the athlete's own `retry`.
/// A connection flagged `needs_reauth` is refreshed on the same terms only
/// when it is a scrape session ([`retries_flagged_session`]), since one
/// failed read can be the scraper's and not the session's; a flagged OAuth
/// grant is dead until the athlete reconnects, and a revoked connection is
/// theirs to restore.
pub(super) async fn stale_refresh_plan(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant_id: TenantId,
    connections: Vec<ProviderConnection>,
    retry: bool,
) -> AppResult<StaleRefreshPlan> {
    let cache = &resources.repos().activity_cache;
    let now = Utc::now();
    let mut plan = StaleRefreshPlan::default();
    for connection in connections {
        let bucket = match connection.status {
            ConnectionStatus::Active => &mut plan.active,
            ConnectionStatus::NeedsReauth if retries_flagged_session(&connection.provider) => {
                &mut plan.flagged
            }
            ConnectionStatus::NeedsReauth | ConnectionStatus::Revoked => continue,
        };
        let last_sync = cache
            .latest_activity_sync(user_id, &tenant_id, &connection.provider)
            .await?;
        let recorded = cache
            .latest_activity_fetch_failure(user_id, &tenant_id, &connection.provider)
            .await?;
        let failed = recorded
            .filter(|failure| last_sync.is_none_or(|synced| failure.failed_at > synced))
            .map(|failure| failure.failed_at);
        if let Some(failed_at) = failed {
            if plan
                .failure
                .as_ref()
                .is_none_or(|newest| failed_at > newest.failed_at)
            {
                let provider = user_facing_name(&connection.provider);
                // The brand, read the way the connect card reads it: a mirror
                // slug (`trainingpeaks`) has no descriptor of its own, and a
                // lookup by slug printed the slug itself.
                plan.failure = Some(SyncFailure {
                    provider: provider.to_owned(),
                    provider_name: brand_name(&resources.fitness.provider_registry, provider)
                        .unwrap_or(provider)
                        .to_owned(),
                    failed_at,
                    last_synced_at: last_sync,
                });
            }
        }
        let due = is_stale(last_sync) || failed.is_some();
        let paused = !retry && sync_backoff_until(recorded, last_sync, now).is_some();
        if due && !paused {
            bucket.push(connection.provider);
        }
    }
    Ok(plan)
}

/// Start the background refresh `plan` calls for, and report whether the page
/// is stale.
///
/// Stale means a provider this load refreshes was past the freshness bands:
/// an active connection with a stale head, or a flagged scrape session whose
/// retry this load claimed. The client then asks once more a little later.
///
/// One refresh per `(user, tenant)` at a time, shared with every other
/// stale-cache revalidation: a Home page reloaded ten times while a two-minute
/// scrape runs starts it once, and every one of those loads still says stale.
/// A flagged session's retry is claimed only by the load that starts the
/// refresh, so the claim is never spent on a refresh that does not run; it is
/// throttled across every caller and replica
/// ([`claim_scrape_session_retry`]). Tracked on the server's drain tracker so
/// shutdown waits for it, and capped at the shared revalidation timeout so a
/// hung scrape frees the slot.
pub(super) async fn start_stale_refresh(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant_id: TenantId,
    plan: StaleRefreshPlan,
) -> bool {
    let StaleRefreshPlan {
        active: mut providers,
        flagged,
        ..
    } = plan;
    if providers.is_empty() && flagged.is_empty() {
        return false;
    }
    let Some(slot) = RevalidationRegistry::global().try_claim((user_id, tenant_id)) else {
        debug!(%user_id, "home: activity refresh already in flight; not starting another");
        return !providers.is_empty();
    };
    let runtime = into_runtime(resources);
    for provider in flagged {
        if claim_scrape_session_retry(&runtime, user_id, tenant_id, &provider).await {
            providers.push(provider);
        }
    }
    if providers.is_empty() {
        return false;
    }
    resources.common.turns.spawn(async move {
        let started_at = Utc::now();
        let bound = revalidation_timeout();
        let mut checked = Vec::with_capacity(providers.len());
        let refresh = refresh_providers(&runtime, user_id, tenant_id, &providers, &mut checked);
        if timeout(bound, refresh).await.is_err() {
            warn!(
                %user_id,
                timeout_secs = bound.as_secs(),
                "home: activity refresh timed out; releasing its slot"
            );
            let unanswered = TimedOut {
                providers: &providers,
                checked: &checked,
                started_at,
            };
            record_timed_out(&runtime, user_id, tenant_id, unanswered).await;
        }
        drop(slot);
    });
    true
}

/// Re-read each provider's recent head, one after the other, naming each in
/// `checked` once its read has finished — succeeded or failed, it has
/// recorded its own outcome. The plan has already judged each one due, so
/// the head is read whatever its freshness ([`refresh_head`]).
async fn refresh_providers(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant_id: TenantId,
    providers: &[String],
    checked: &mut Vec<String>,
) {
    for provider in providers {
        // The refresh writes through to the cache; the rows it returns are
        // what the next request reads, so nothing is kept here.
        let refreshed = refresh_head(runtime, provider, user_id, tenant_id).await;
        debug!(
            %user_id,
            provider = %provider,
            fetched = refreshed.map_or(0, |rows| rows.len()),
            "home: provider head checked"
        );
        checked.push(provider.clone());
    }
}

/// The providers a bounded refresh was asked to read, those it finished, and
/// when it began.
struct TimedOut<'a> {
    providers: &'a [String],
    checked: &'a [String],
    started_at: DateTime<Utc>,
}

/// Record a failed sync for every provider the refresh did not finish before
/// its timeout: the read in flight was dropped, so nothing else records that
/// it never answered, and the ones after it were never asked — the sync this
/// load started did not happen for them either. A provider whose sync has
/// landed since the refresh began is left alone: the read that was cut off
/// had already written its rows and its mark, and a failure recorded after
/// them would report a sync that worked as one that did not. Best-effort,
/// like every failure record.
async fn record_timed_out(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant_id: TenantId,
    unanswered: TimedOut<'_>,
) {
    let cache = &runtime.repos().activity_cache;
    for provider in unanswered
        .providers
        .iter()
        .filter(|p| !unanswered.checked.contains(p))
    {
        let synced = cache
            .latest_activity_sync(user_id, &tenant_id, provider)
            .await
            .unwrap_or(None);
        if synced.is_some_and(|synced| synced >= unanswered.started_at) {
            continue;
        }
        record_sync_failure(
            runtime,
            user_id,
            tenant_id,
            provider,
            ActivityFetchFailure::FetchError,
        )
        .await;
    }
}
