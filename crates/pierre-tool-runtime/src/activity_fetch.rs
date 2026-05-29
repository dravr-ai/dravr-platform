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

use std::sync::Arc;

use pierre_core::models::Activity;
use pierre_providers::core::ActivityQueryParams;
use tracing::warn;
use uuid::Uuid;

use crate::protocol::auth::AuthService;
use crate::runtime::ToolRuntime;

/// Fetch activities for a single provider connection, authenticating (and
/// refreshing tokens) as needed.
///
/// Returns `None` when the provider can't be created (no valid token,
/// unsupported provider) or the activity fetch fails — both are logged at
/// `warn` and treated as "no activities" by callers.
pub async fn fetch_provider_activities(
    runtime: &Arc<dyn ToolRuntime>,
    provider_slug: &str,
    user_id: Uuid,
    tenant_id: &str,
    params: &ActivityQueryParams,
) -> Option<Vec<Activity>> {
    let auth_service = AuthService::new(Arc::clone(runtime));

    let provider = auth_service
        .create_authenticated_provider(provider_slug, user_id, Some(tenant_id))
        .await
        .map_err(|e| {
            warn!(
                user_id = %user_id,
                provider = %provider_slug,
                error = ?e.error,
                "fetch_provider_activities: failed to create authenticated provider"
            );
        })
        .ok()?;

    provider
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
        .ok()
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
