// ABOUTME: Commitment-scoped background activity refresh — warms the cache for athletes with a due promise
// ABOUTME: Fetches every connected provider through the shared auth+fetch path so the sweep can label a verdict
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Commitment-driven activity refresh.
//!
//! The commitment sweep counts an athlete's promise against the durable
//! activity cache, which is warmed write-through by chat and tool fetches. An
//! athlete who promised something and then never opened a conversation has a
//! cold window the sweep refuses to read as a miss — so instead of waiting for
//! a chat that may never come, the sweep requests one of these bounded
//! refreshes. Each refresh fetches the commitment window from every provider
//! the athlete has connected, through the same authenticate-and-fetch path a
//! chat turn uses (so every provider — OAuth API or scrape — is covered by the
//! one seam), and the write-through plus its fetch-freshness mark is what lets
//! the next sweep tick trust the count. De-duplicated per `(tenant, user)` so
//! hourly ticks cannot stack fetches for the same athlete.

use std::collections::HashSet;
use std::sync::{Arc, LazyLock, Mutex, PoisonError};

use pierre_core::models::TenantId;
use pierre_providers::core::ActivityQueryParams;
use tracing::{info, warn};
use uuid::Uuid;

use crate::activity_fetch::fetch_provider_activities;
use crate::runtime::ToolRuntime;

/// Cap on activities pulled per provider for one refresh. A commitment window
/// is at most 30 days, so this never truncates a real working set (it mirrors
/// the sweep's own activity scan limit).
const REFRESH_FETCH_LIMIT: usize = 200;

/// In-flight refresh keys (`tenant:user`) so a sweep tick that defers several
/// of one athlete's commitments — or two ticks racing a slow scrape — cannot
/// stack duplicate provider fetches.
static IN_FLIGHT_REFRESHES: LazyLock<Mutex<HashSet<String>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

fn in_flight_key(tenant_id: TenantId, user_id: Uuid) -> String {
    format!("{tenant_id}:{user_id}")
}

/// Spawn a bounded background job that fetches the window starting at
/// `window_start_ts` (unix seconds) from every provider the athlete has
/// connected, writing through to the durable activity cache.
///
/// Returns `true` if a new job was started, `false` when one was already in
/// flight for this athlete.
pub fn spawn_commitment_activity_refresh(
    runtime: Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant_id: TenantId,
    window_start_ts: i64,
) -> bool {
    let key = in_flight_key(tenant_id, user_id);
    {
        let mut guard = IN_FLIGHT_REFRESHES
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if !guard.insert(key.clone()) {
            return false;
        }
    }

    tokio::spawn(async move {
        run_commitment_refresh(&runtime, user_id, tenant_id, window_start_ts).await;
        IN_FLIGHT_REFRESHES
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&key);
    });
    true
}

/// Fetch the window from each of the athlete's connected providers. Per-provider
/// failures are logged by the shared fetch path and skipped — a dead provider
/// must not stop a live one from producing the count.
async fn run_commitment_refresh(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant_id: TenantId,
    window_start_ts: i64,
) {
    let connections = match runtime
        .repos()
        .provider_connections
        .get_for_user(user_id, Some(tenant_id))
        .await
    {
        Ok(connections) => connections,
        Err(e) => {
            warn!(
                user_id = %user_id,
                error = %e,
                "Commitment refresh: failed to list connected providers"
            );
            return;
        }
    };
    if connections.is_empty() {
        info!(
            user_id = %user_id,
            "Commitment refresh: athlete has no connected providers; nothing to fetch"
        );
        return;
    }

    let providers: Vec<&str> = connections.iter().map(|c| c.provider.as_str()).collect();
    let fetched =
        fetch_window_across_providers(runtime, user_id, tenant_id, window_start_ts, &providers)
            .await;

    info!(
        user_id = %user_id,
        providers = providers.len(),
        fetched,
        "Commitment refresh: fetched the commitment window across connected providers"
    );
}

/// Fetch `[window_start_ts, now]` from each provider through the shared
/// authenticate-and-fetch path, returning the total activities fetched.
async fn fetch_window_across_providers(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant_id: TenantId,
    window_start_ts: i64,
    providers: &[&str],
) -> usize {
    let params = ActivityQueryParams {
        limit: Some(REFRESH_FETCH_LIMIT),
        offset: None,
        before: None,
        after: Some(window_start_ts),
    };
    let tenant_str = tenant_id.to_string();
    let mut fetched = 0_usize;
    for provider in providers {
        if let Some(activities) =
            fetch_provider_activities(runtime, provider, user_id, &tenant_str, &params).await
        {
            fetched += activities.len();
        }
    }
    fetched
}
