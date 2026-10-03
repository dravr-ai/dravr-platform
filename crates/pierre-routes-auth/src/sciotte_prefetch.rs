// ABOUTME: The activity pre-fetch a fresh sciotte session starts, and the registry every reader waits on
// ABOUTME: One scrape per connection: later readers and the health backfill wait on it (carnet#736)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;
use std::time::Duration;

use pierre_cache::{Cache, CacheKey, CacheResource};
use pierre_config::agent_recommendations::AgentRecommendationConfig;
use pierre_core::models::{Activity, TenantId};
use pierre_providers::connect_prefetch::{
    ConnectPrefetches, PrefetchKey, PrefetchTicket, PrefetchWait, PrefetchedWindow,
};
use pierre_providers::core::{ActivityQueryParams, CredentialKind, OAuth2Credentials};
use pierre_providers::registry::ProviderRegistry;
use tracing::{info, warn};
use uuid::Uuid;

use crate::AuthRoutesContext;

/// How many of the newest activities a connect pre-fetches.
///
/// At least the first page the activity list caches, and as many as the
/// onboarding proposal reads per provider, so the proposal that follows a
/// connect is answered whole from the pre-fetch: the newest N either reach
/// below the proposal's window or fill its limit ([`PrefetchedWindow::serve`]).
fn prefetch_limit() -> usize {
    AgentRecommendationConfig::from_env()
        .activity_limit_per_provider
        .max(cache_page_len())
}

/// The page size the pre-fetch's activity-list cache entry is written under,
/// the first page the activity list reads.
const PREFETCH_CACHE_PAGE: u32 = 30;

/// [`PREFETCH_CACHE_PAGE`] as a length.
fn cache_page_len() -> usize {
    usize::try_from(PREFETCH_CACHE_PAGE).unwrap_or(usize::MAX)
}

/// Register the activity pre-fetch of a fresh sciotte session and spawn it in
/// the background, so the agent has warm data on the first chat.
///
/// Returns the wait on it, for the health backfill to start after. When a
/// pre-fetch of the connection is already running, none is started and the
/// wait is on that one, which every reader then waits on; `None` only when it
/// finished in between. Backpressure lives on the dedicated service (ADR-021
/// Phase 4 cutover); the registry keeps this platform from pressing one
/// session with several reads at once.
pub fn spawn_activity_prefetch(
    resources: &AuthRoutesContext,
    user_id: Uuid,
    tenant_id: Uuid,
    provider_name: &str,
    session_json: &str,
) -> Option<PrefetchWait> {
    let Some(ticket) = begin_prefetch(user_id, tenant_id, provider_name) else {
        return prefetch_in_flight(user_id, tenant_id, provider_name);
    };
    let wait = ticket.wait();
    tokio::spawn(prefetch_activities(
        Arc::clone(&resources.provider_registry),
        Arc::clone(&resources.cache),
        PrefetchTarget {
            user_id,
            tenant_id,
            provider_name: provider_name.to_owned(),
            session_json: session_json.to_owned(),
        },
        ticket,
    ));
    Some(wait)
}

/// Register the pre-fetch of `user_id`'s `provider_name` connection, or `None`
/// (logged) when one is already in flight.
pub fn begin_prefetch(
    user_id: Uuid,
    tenant_id: Uuid,
    provider_name: &str,
) -> Option<PrefetchTicket> {
    let key = PrefetchKey::new(TenantId::from_uuid(tenant_id), user_id, provider_name);
    let ticket = ConnectPrefetches::global().begin(key);
    if ticket.is_none() {
        info!(
            user_id = %user_id,
            provider = %provider_name,
            "Activity pre-fetch already in flight for this connection — not starting another"
        );
    }
    ticket
}

/// The pre-fetch of `user_id`'s `provider_name` connection still running, to
/// wait on; `None` when none is.
pub fn prefetch_in_flight(
    user_id: Uuid,
    tenant_id: Uuid,
    provider_name: &str,
) -> Option<PrefetchWait> {
    let key = PrefetchKey::new(TenantId::from_uuid(tenant_id), user_id, provider_name);
    ConnectPrefetches::global().in_flight(&key)
}

/// The connection a pre-fetch reads, and the session it reads through.
pub struct PrefetchTarget {
    /// The athlete.
    pub user_id: Uuid,
    /// The tenant the connection lives in.
    pub tenant_id: Uuid,
    /// The backend the connection is stored under (`sciotte_coros`).
    pub provider_name: String,
    /// The session the login just stored, as JSON.
    pub session_json: String,
}

/// Pre-fetch and cache the newest activities of the account the target's
/// session signed in to, then publish them on `ticket` to every reader
/// waiting on the pre-fetch. Every failure is logged: the cache is only
/// warmed, and a waiting reader then reads live.
pub async fn prefetch_activities(
    registry: Arc<ProviderRegistry>,
    cache: Arc<Cache>,
    target: PrefetchTarget,
    ticket: PrefetchTicket,
) {
    let PrefetchTarget {
        user_id,
        tenant_id,
        provider_name,
        session_json,
    } = target;
    info!(
        user_id = %user_id,
        provider = %provider_name,
        "Starting background activity pre-fetch (remote scrape)"
    );
    let limit = prefetch_limit();
    let Some((activities, head_complete)) =
        fetch_prefetch_window(&registry, user_id, &provider_name, session_json, limit).await
    else {
        ticket.finish(None);
        return;
    };
    let count = activities.len();
    let first_page: Vec<Activity> = activities.iter().take(cache_page_len()).cloned().collect();
    ticket.finish(Some(PrefetchedWindow::new(
        activities,
        limit,
        head_complete,
    )));
    let cache_key = CacheKey::new(
        TenantId::from_uuid(tenant_id),
        user_id,
        provider_name.clone(),
        CacheResource::ActivityList {
            page: 1,
            per_page: PREFETCH_CACHE_PAGE,
            before: None,
            after: None,
            sport_type: None,
        },
    );
    let ttl = Duration::from_mins(15);
    if let Err(e) = cache.set(&cache_key, &first_page, ttl).await {
        warn!(error = %e, "Background pre-fetch: failed to cache activities");
    } else {
        info!(user_id = %user_id, provider = %provider_name, count, "Background activity pre-fetch complete — cache warm");
    }
}

/// The newest `limit` activities of the account `session_json` signed in to,
/// read through a fresh `provider_name` provider, and whether the capture
/// reached the list head; `None`, logged, when the provider cannot be built,
/// take the session, or answer.
async fn fetch_prefetch_window(
    registry: &ProviderRegistry,
    user_id: Uuid,
    provider_name: &str,
    session_json: String,
    limit: usize,
) -> Option<(Vec<Activity>, bool)> {
    let provider = registry
        .create_provider(provider_name)
        .inspect_err(|e| warn!(error = %e, "Background pre-fetch: failed to create provider"))
        .ok()?;
    let credentials = OAuth2Credentials {
        client_id: String::new(),
        client_secret: String::new(),
        access_token: Some(session_json),
        refresh_token: None,
        expires_at: None,
        scopes: vec![],
        kind: CredentialKind::OAuthBearer,
    };
    provider
        .set_credentials(credentials)
        .await
        .inspect_err(|e| warn!(error = %e, "Background pre-fetch: failed to set credentials"))
        .ok()?;
    let params = ActivityQueryParams {
        limit: Some(limit),
        offset: None,
        before: None,
        after: None,
    };
    let activities = provider
        .get_activities_with_params(&params)
        .await
        .inspect_err(|e| {
            warn!(user_id = %user_id, error = %e, "Background pre-fetch: failed to fetch activities");
        })
        .ok()?;
    Some((activities, provider.head_complete()))
}
