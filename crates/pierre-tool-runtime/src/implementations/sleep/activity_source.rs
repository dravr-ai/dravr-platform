// ABOUTME: Where the recovery and sleep tools read training load from: the athlete's activity provider
// ABOUTME: Elects the provider the way every activity tool does, fetches through the shared path, merges sessions
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::models::Activity;
use pierre_providers::core::ActivityQueryParams;
use pierre_providers::deduplication::{merge_duplicates, DedupConfig};
use serde_json::Value;
use uuid::Uuid;

use crate::activity_fetch::fetch_provider_activities;
use crate::protocol::provider_helpers::resolve_provider_for_request;
use crate::protocol::{UniversalResponse, UniversalToolExecutor};

/// The provider the recovery and sleep tools compute training load from, and
/// its activities, oldest first.
///
/// The provider is the `activity_provider` argument when the caller names one,
/// else the one [`resolve_provider_for_request`] elects for every activity tool
/// (the `provider` argument, the deployment override, then the athlete's
/// most-recently-used connection) — so an Intervals.icu, TrainingPeaks or
/// scrape-connected Garmin athlete is read like any other, and an athlete with
/// no connection gets the canonical reconnect refusal instead of a Strava
/// fetch they never set up. The rows come through the shared
/// [`fetch_provider_activities`], which writes a live fetch through to the
/// activity cache and serves that cache when the provider is unreachable, and
/// every recording of one workout is merged into a single session before any
/// load is computed, as the training-load tool does.
///
/// # Errors
/// Returns a boxed `UniversalResponse` when no provider can be elected, when
/// the request carries no tenant, or when the provider can neither be read nor
/// served from cache — the provider's own authentication refusal (with its
/// reconnect tag) when that is why.
pub(super) async fn training_load_activities(
    executor: &UniversalToolExecutor,
    parameters: &Value,
    user_uuid: Uuid,
    tenant_id: Option<&str>,
) -> Result<(String, Vec<Activity>), Box<UniversalResponse>> {
    let provider = match parameters
        .get("activity_provider")
        .and_then(Value::as_str)
        .filter(|p| !p.is_empty())
    {
        Some(named) => named.to_owned(),
        None => resolve_provider_for_request(parameters, executor, user_uuid, tenant_id).await?,
    };
    let Some(tenant) = tenant_id else {
        return Err(Box::new(failure(format!(
            "Reading activities from '{provider}' needs the request's tenant"
        ))));
    };

    let params = ActivityQueryParams {
        limit: Some(executor.resources.config().sleep_tool_params.activity_limit as usize),
        offset: None,
        before: None,
        after: None,
    };
    let Some(raw) =
        fetch_provider_activities(&executor.resources, &provider, user_uuid, tenant, &params).await
    else {
        // Nothing live and nothing cached. When the connection cannot
        // authenticate, answer with its refusal, which carries the reconnect
        // tag the tool loop turns into a login link.
        executor
            .auth_service
            .create_authenticated_provider(&provider, user_uuid, Some(tenant))
            .await?;
        return Err(Box::new(failure(format!(
            "Failed to fetch activities from '{provider}'"
        ))));
    };

    let (mut activities, _report) = merge_duplicates(raw, &DedupConfig::from_env());
    // Oldest first: the EMA in `TrainingLoadCalculator` reads the series in
    // chronological order.
    activities.sort_by_key(Activity::start_date);
    Ok((provider, activities))
}

fn failure(error: String) -> UniversalResponse {
    UniversalResponse {
        success: false,
        result: None,
        error: Some(error),
        metadata: None,
    }
}
