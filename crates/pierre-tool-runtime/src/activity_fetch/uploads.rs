// ABOUTME: Activities the athlete brought in (uploaded .fit files, imported archives) folded into every activity read
// ABOUTME: A local read of the athlete's own rows: no connection, no sync to record, nothing written back

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! User-provided activities in the agent's reads (carnet#818, carnet#1218).
//!
//! An uploaded workout is cached under the `upload` key, an imported Strava
//! history under `strava_archive` (`USER_PROVIDED_PROVIDERS`), and neither has
//! a connection behind it, so neither the provider election nor the fold of
//! the athlete's other connections ever reaches them. This module is the one
//! place those reads add them: beside the elected provider's window, beside
//! the connections that stood in for a missing one, and on their own for an
//! athlete whose only activities they are. It reads through
//! [`UserProvidedProvider`], the same reader the authentication chokepoint
//! hands out for those keys, and never through the live-fetch path: an upload
//! or an import is not a sync, so nothing here records freshness or writes
//! the cache.

use pierre_core::constants::oauth_providers::{self, USER_PROVIDED_PROVIDERS};
use pierre_core::models::{Activity, TenantId};
use pierre_providers::backend_resolver;
use pierre_providers::core::{ActivityQueryParams, OwnerStores};
use pierre_providers::upload_provider::UserProvidedProvider;
use pierre_providers::CoreFitnessProvider;
use pierre_tools_core::ToolResult;
use serde_json::Value;
use std::sync::Arc;
use tracing::warn;

use crate::context::ToolExecutionContext;
use crate::protocol::provider_helpers;

/// The user-provided keys the athlete holds activities under in the
/// context's tenant, in [`USER_PROVIDED_PROVIDERS`] order; none for a call
/// made outside a tenant, which holds none.
pub async fn held_user_provided(context: &ToolExecutionContext) -> Vec<&'static str> {
    let Some(tenant) = context.tenant_id.map(TenantId::from_uuid) else {
        return Vec::new();
    };
    let held = context
        .resources
        .repos()
        .activity_cache
        .held_user_provided(context.user_id, &tenant)
        .await
        .unwrap_or_else(|e| {
            warn!(user_id = %context.user_id, error = %e, "user-provided sources lookup failed");
            Vec::new()
        });
    USER_PROVIDED_PROVIDERS
        .iter()
        .copied()
        .filter(|key| held.iter().any(|held| held == key))
        .collect()
}

/// The athlete's activities under the user-provided `keys` inside `params`'
/// window, newest first per key.
///
/// Best effort, like every other source folded into a read: a failed read is
/// logged and contributes nothing.
pub async fn user_provided_window(
    context: &ToolExecutionContext,
    keys: &[&'static str],
    params: &ActivityQueryParams,
) -> Vec<Activity> {
    let Some(tenant) = context.tenant_id.map(TenantId::from_uuid) else {
        return Vec::new();
    };
    let repos = context.resources.repos();
    let mut activities = Vec::new();
    for key in keys {
        let reader = UserProvidedProvider::for_key(
            key,
            OwnerStores::from_repos(repos, context.user_id, tenant),
            Arc::clone(context.resources.originals()),
        );
        match reader.get_activities_with_params(params).await {
            Ok(rows) => activities.extend(rows),
            Err(e) => warn!(
                user_id = %context.user_id,
                provider = key,
                error = %e,
                "user-provided activities could not be read; the window is served without them"
            ),
        }
    }
    activities
}

/// Every user-provided activity the athlete holds inside `params`' window.
pub async fn every_user_provided_window(
    context: &ToolExecutionContext,
    params: &ActivityQueryParams,
) -> Vec<Activity> {
    let held = held_user_provided(context).await;
    user_provided_window(context, &held, params).await
}

/// The rows a read resolved to a user-provided key serves.
///
/// That key's alone when the read is pinned to it (`pinned_key`), else every
/// key the athlete holds, since the read stands in for a connection they do
/// not have.
pub async fn user_provided_rows(
    context: &ToolExecutionContext,
    pinned_key: Option<&str>,
    params: &ActivityQueryParams,
) -> Vec<Activity> {
    match USER_PROVIDED_PROVIDERS
        .iter()
        .find(|key| pinned_key == Some(**key))
    {
        Some(key) => user_provided_window(context, &[key], params).await,
        None => every_user_provided_window(context, params).await,
    }
}

/// The athlete's own copies of `provider`'s activities inside `params`'
/// window (an imported Strava history for `strava`): what a read pinned to
/// `provider` also serves.
pub async fn companion_window(
    context: &ToolExecutionContext,
    provider: &str,
    params: &ActivityQueryParams,
) -> Vec<Activity> {
    let companions = oauth_providers::companion_keys(provider);
    if companions.is_empty() {
        return Vec::new();
    }
    let held = held_user_provided(context).await;
    let keys: Vec<&'static str> = companions
        .iter()
        .copied()
        .filter(|key| held.contains(key))
        .collect();
    user_provided_window(context, &keys, params).await
}

/// The provider a read answers from: the one [`resolve_provider_for_tool`]
/// elects, else what the athlete brought in themselves.
///
/// A read pinned to a provider the athlete is not connected to is served from
/// the history they imported from it ([`companion_stand_in`]); a read no
/// connection answers is served from their uploads and imports
/// ([`stand_in_key`]). Neither held, the election's refusal stands.
///
/// # Errors
///
/// The refusal to answer with, when nothing answers.
pub async fn resolve_with_own_copies(
    args: &Value,
    context: &ToolExecutionContext,
) -> Result<String, ToolResult> {
    match provider_helpers::resolve_provider_for_tool(args, context).await {
        Ok(provider) => Ok(companion_stand_in(context, &provider)
            .await
            .map_or(provider, str::to_owned)),
        Err(refusal) => stand_in_key(context, args.get("provider").and_then(Value::as_str))
            .await
            .map(str::to_owned)
            .ok_or(refusal),
    }
}

/// The user-provided key that answers a read whose provider could not be resolved.
///
/// With nothing connected, that is the athlete's own copy of `requested`'s
/// history when they hold one (`strava_archive` for `strava`), the key itself
/// when `requested` names one they hold, else the first key they hold.
pub async fn stand_in_key(
    context: &ToolExecutionContext,
    requested: Option<&str>,
) -> Option<&'static str> {
    let held = held_user_provided(context).await;
    let Some(requested) = requested.filter(|requested| !requested.is_empty()) else {
        return held.first().copied();
    };
    held.iter()
        .copied()
        .find(|key| *key == requested || oauth_providers::companion_keys(requested).contains(key))
}

/// The athlete's own copy of `provider`'s history, for a read pinned to it.
///
/// When they hold no connection to `provider`, their imported copy of its
/// history (an imported Strava export for `strava`) answers, so asking for a
/// provider by name serves what they brought in from it rather than a
/// refusal to connect. `None` when they are connected to `provider`, or hold
/// no copy of its history.
pub async fn companion_stand_in(
    context: &ToolExecutionContext,
    provider: &str,
) -> Option<&'static str> {
    let companions = oauth_providers::companion_keys(provider);
    if companions.is_empty() {
        return None;
    }
    let connected = context
        .resources
        .repos()
        .provider_connections
        .get_for_user(context.user_id, None)
        .await
        .map_or(true, |connections| {
            connections
                .iter()
                .any(|conn| backend_resolver::user_facing_name(&conn.provider) == provider)
        });
    if connected {
        return None;
    }
    let held = held_user_provided(context).await;
    companions.iter().copied().find(|key| held.contains(key))
}

/// Whether the athlete holds any activity they brought in, in the context's
/// tenant: an athlete with no connection still has these to answer from.
pub async fn holds_user_provided(context: &ToolExecutionContext) -> bool {
    !held_user_provided(context).await.is_empty()
}
