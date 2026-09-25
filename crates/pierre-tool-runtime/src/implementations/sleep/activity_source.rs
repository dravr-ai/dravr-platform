// ABOUTME: Where the recovery and sleep tools read training load from: the athlete's activity provider
// ABOUTME: Picks the provider the athlete can read activities from, through its serving backends
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::models::Activity;
use pierre_providers::backend_resolver::serving_backends;
use uuid::Uuid;

use crate::protocol::{UniversalResponse, UniversalToolExecutor};

/// Provider-agnostic activity fetcher
///
/// Fetches activities from any supported fitness provider based on the provider name.
/// Uses `AuthService` for tenant-aware credential lookup and provider instantiation,
/// falling back to environment credentials when tenant-specific ones are not configured.
///
/// # Arguments
/// * `executor` - The tool executor with access to auth service and provider registry
/// * `user_uuid` - The user's UUID for token lookup
/// * `tenant_id` - Optional tenant ID for multi-tenant deployments
/// * `provider_name` - Name of the provider to fetch from (e.g., "strava", "garmin", "whoop")
///
/// # Errors
/// Returns a boxed `UniversalResponse` if authentication fails or activities cannot be fetched
pub(super) async fn fetch_provider_activities(
    executor: &UniversalToolExecutor,
    user_uuid: Uuid,
    tenant_id: Option<&str>,
    provider_name: &str,
) -> Result<Vec<Activity>, Box<UniversalResponse>> {
    // Use AuthService for tenant-aware authenticated provider creation
    let provider = executor
        .auth_service
        .create_authenticated_provider(provider_name, user_uuid, tenant_id)
        .await?;

    #[allow(clippy::cast_possible_truncation)]
    let mut activities = provider
        .get_activities(
            Some(executor.resources.config().sleep_tool_params.activity_limit as usize),
            None,
        )
        .await
        .map_err(|e| UniversalResponse {
            success: false,
            result: None,
            error: Some(format!(
                "Failed to fetch activities from '{provider_name}': {e}"
            )),
            metadata: None,
        })?;

    // Sort oldest-first — EMA calculation in TrainingLoadCalculator requires chronological order
    activities.sort_by_key(Activity::start_date);
    Ok(activities)
}

/// Select the best available activity provider for a user
///
/// Returns the first provider, in priority order, that the user can read
/// activities from: one of its serving backends advertises activities and the
/// user holds a valid token for that backend. A scrape-connected Garmin or
/// COROS athlete holds the mirror's session (`sciotte_garmin`,
/// `sciotte_coros`), not a `garmin` or `coros` token, so the check runs per
/// backend and the user-facing name is returned for provider creation to
/// resolve. Priority order: strava > garmin > coros > whoop > intervals.icu >
/// terra.
pub(super) async fn select_activity_provider(
    executor: &UniversalToolExecutor,
    user_uuid: Uuid,
    tenant_id: Option<&str>,
) -> Option<String> {
    // Activity provider priority (Strava is best for activities)
    let priority = [
        "strava",
        "garmin",
        "coros",
        "whoop",
        "intervals_icu",
        "terra",
    ];

    for provider in priority {
        for backend in serving_backends(provider) {
            let serves_activities = executor
                .resources
                .provider_registry()
                .get_capabilities(&backend)
                .is_some_and(|caps| caps.supports_activities());
            if serves_activities
                && matches!(
                    executor
                        .auth_service
                        .get_valid_token(user_uuid, &backend, tenant_id)
                        .await,
                    Ok(Some(_))
                )
            {
                return Some(provider.to_owned());
            }
        }
    }
    None
}
