// ABOUTME: Shared helper functions for provider-agnostic handler operations
// ABOUTME: Consolidates provider extraction and resolution, and the fetch helpers built on the auth chokepoint
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::activity_fetch::provider_auth_failure;
use crate::context::ToolExecutionContext;
use crate::protocol::auth::AuthService;
use crate::protocol::types::{
    UniversalResponse, UniversalToolExecutor, META_AUTH_REQUIRED_PROVIDER,
};
use crate::runtime::ToolRuntime;
use pierre_config::environment::{default_provider, get_oauth_config, OAuthProviderConfig};
use pierre_core::errors::{AppError, ErrorCode};
use pierre_core::models::{Activity, TenantId};
use pierre_providers::core::FitnessProvider;
use pierre_tools_core::ToolResult;
use serde_json::{json, Value as JsonValue};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, info, warn};
use uuid::Uuid;

/// Extract the literal `provider` argument from request parameters, when present.
///
/// Returns `None` when the caller didn't pass `provider` (or passed an empty string).
/// **Does not fall back to a synthetic-or-strava-default;** that historical behavior
/// silently bound untargeted tool calls to seed data in production. Use
/// [`resolve_provider_for_request`] to do the full priority chain (explicit arg →
/// env override → user's most-recently-used connection → reconnect signal).
#[must_use]
pub fn extract_provider(parameters: &serde_json::Map<String, JsonValue>) -> Option<String> {
    parameters
        .get("provider")
        .and_then(JsonValue::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// The canonical no-provider refusal, minted in exactly one place.
///
/// Three sites refuse for the same reason — both resolvers here and the
/// dispatch chokepoint in `executor.rs` — and the tool loop detects this shape
/// (`success: false` + [`META_AUTH_REQUIRED_PROVIDER`]) to hand the turn to the
/// `auth_recovery` stage, which mints a hosted-login URL and localized copy.
/// One constructor keeps the three from drifting apart; drift here means one
/// path silently stops triggering the recovery flow.
#[must_use]
pub fn no_provider_refusal() -> UniversalResponse {
    let mut metadata: HashMap<String, JsonValue> = HashMap::new();
    metadata.insert(
        META_AUTH_REQUIRED_PROVIDER.to_owned(),
        JsonValue::String("sciotte".to_owned()),
    );
    UniversalResponse {
        success: false,
        result: None,
        error: Some(
            "No fitness provider connected. Connect Strava, Garmin, or another \
             provider before asking for activity data."
                .to_owned(),
        ),
        metadata: Some(metadata),
    }
}

/// Resolve which fitness provider to serve a tool call from.
///
/// Priority chain — matches the user mental model of "what data are we looking at":
/// 1. Explicit `provider` argument in request parameters.
/// 2. Deployment-wide `PIERRE_DEFAULT_PROVIDER` env override.
/// 3. The user's most-recently-used connection from `provider_connections`
///    (ordered by `last_used_at DESC NULLS LAST, connected_at DESC`).
/// 4. Returns a `UniversalResponse` carrying `META_AUTH_REQUIRED_PROVIDER` so the
///    tool loop short-circuits and the chat-pipeline `auth_recovery` stage mints
///    a hosted-login URL and renders the localized reconnect copy.
///
/// Replaces the historical `.and_then(|v| v.as_str()).map_or_else(default_provider, String::from)`
/// pattern duplicated across every fitness-API handler, which silently bound
/// untargeted calls to the synthetic seed provider in production.
///
/// # Errors
///
/// Returns a boxed `UniversalResponse` when the user has no provider connections at
/// all. The returned response carries the canonical reconnect-required metadata
/// the tool loop scans for.
pub async fn resolve_provider_for_request(
    parameters: &JsonValue,
    executor: &UniversalToolExecutor,
    user_uuid: Uuid,
    tenant_id: Option<&str>,
) -> Result<String, Box<UniversalResponse>> {
    // 1. Explicit arg
    if let Some(p) = parameters
        .get("provider")
        .and_then(JsonValue::as_str)
        .filter(|s| !s.is_empty())
    {
        return Ok(p.to_owned());
    }

    // 2. Env override
    if let Some(env_p) = default_provider() {
        return Ok(env_p);
    }

    // 3. User's most-recently-used connection
    let tenant = tenant_id.and_then(|t| TenantId::parse_str(t).ok());
    match executor
        .resources
        .repos()
        .provider_connections
        .resolve_most_recent(user_uuid, tenant)
        .await
    {
        Ok(Some(conn)) => {
            info!(
                target: "notify",
                user_id = %user_uuid,
                provider = %conn.provider,
                "resolved provider from user's most-recent connection"
            );
            Ok(conn.provider)
        }
        Ok(None) => {
            warn!(
                user_id = %user_uuid,
                "no fitness provider connected for user — surfacing reconnect signal"
            );
            // The race-window backstop: the dispatch chokepoint already refused
            // providerless users for REQUIRES_PROVIDER tools, so reaching this
            // branch means either a tool that calls a resolver without declaring
            // the capability, or a disconnect between the check and this read.
            Err(Box::new(no_provider_refusal()))
        }
        Err(e) => {
            warn!(
                user_id = %user_uuid,
                error = %e,
                "provider_connections lookup failed during resolution"
            );
            Err(Box::new(UniversalResponse {
                success: false,
                result: None,
                error: Some(format!(
                    "Failed to resolve fitness provider for this request: {e}"
                )),
                metadata: None,
            }))
        }
    }
}

/// `McpTool::execute`-shaped variant of [`resolve_provider_for_request`].
///
/// Same priority chain (explicit arg → env override → most-recent connection)
/// but reads from a [`crate::context::ToolExecutionContext`] (which carries
/// `Arc<dyn ToolRuntime>` + parsed `user_id`/`tenant_id`) instead of a
/// `UniversalRequest`. Returns a JSON `ToolResult::error` payload on no-provider
/// rather than minting a `UniversalResponse`, because `McpTool` callers consume
/// `ToolResult` directly.
///
/// # Errors
///
/// Returns `Err(ToolResult)` carrying the canonical `auth_required_provider`
/// payload when the user has no provider connections at all. Callers should
/// propagate this with `?` or an early return so the chat tool-loop's
/// `auth_recovery` stage renders the localized reconnect copy.
pub async fn resolve_provider_for_tool(
    args: &JsonValue,
    context: &ToolExecutionContext,
) -> Result<String, ToolResult> {
    // 1. Explicit arg
    if let Some(p) = args
        .get("provider")
        .and_then(JsonValue::as_str)
        .filter(|s| !s.is_empty())
    {
        return Ok(p.to_owned());
    }

    // 2. Env override
    if let Some(env_p) = default_provider() {
        return Ok(env_p);
    }

    // 3. User's most-recently-used connection
    let tenant = context.tenant_id.map(TenantId::from_uuid);
    match context
        .resources
        .repos()
        .provider_connections
        .resolve_most_recent(context.user_id, tenant)
        .await
    {
        Ok(Some(conn)) => Ok(conn.provider),
        Ok(None) | Err(_) => Err(ToolResult::error(json!({
            "error": "No fitness provider connected. Connect Strava, Garmin, or another provider before asking for activity data.",
            "auth_required_provider": "sciotte"
        }))),
    }
}

/// Create a standard auth error response
#[must_use]
pub fn create_auth_error_response(provider_name: &str, error: &str) -> UniversalResponse {
    UniversalResponse {
        success: true,
        result: Some(json!({
            "activities": [],
            "message": format!("Authentication error for {provider_name}: {error}"),
            "error": format!("Authentication error: {error}"),
            "provider": provider_name
        })),
        error: None,
        metadata: Some({
            let mut map = HashMap::new();
            map.insert("authentication_error".to_owned(), JsonValue::Bool(true));
            map.insert(
                "provider".to_owned(),
                JsonValue::String(provider_name.to_owned()),
            );
            map
        }),
    }
}

/// Get OAuth config for a provider, with logging
pub fn get_provider_oauth_config(provider_name: &str) -> OAuthProviderConfig {
    let config = get_oauth_config(provider_name);
    debug!(
        provider = provider_name,
        has_client_id = config.client_id.is_some(),
        "Loaded OAuth config for provider"
    );
    config
}

/// Fetch activities from the user's connected provider
///
/// Authenticates through [`AuthService::create_authenticated_provider`], the
/// chokepoint every provider read goes through, and fetches recent activities.
///
/// # Errors
///
/// Returns [`AppError`] when:
/// - the connection needs reconnecting ([`AppError::provider_auth_required`])
///   or the provider cannot be authenticated for another reason
/// - The provider's `get_activities` call fails (network, auth refresh, etc.)
pub async fn fetch_activities_from_provider(
    resources: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    provider_name: &str,
    tenant_id: Option<&str>,
    limit: Option<usize>,
) -> Result<Vec<Activity>, AppError> {
    let provider = configured_provider(resources, user_id, provider_name, tenant_id).await?;

    // Fetch activities. Preserve a `ProviderAuthRequired` error verbatim so it
    // keeps its code (and provider slug in `details`) across the `?` boundary:
    // the tool executor short-circuits on it and the chat-pipeline
    // `auth_recovery` stage mints a hosted-login link and renders the
    // reconnect copy. Wrapping it in `internal` here erased the code and the
    // user got a generic "internal error" instead of a reconnect prompt.
    provider.get_activities(limit, None).await.map_err(|e| {
        if e.code == ErrorCode::ProviderAuthRequired {
            e
        } else {
            AppError::internal(format!("Failed to fetch activities: {e}"))
        }
    })
}

/// Fetch ONE activity by id from the user's connected provider.
///
/// The single-activity sibling of [`fetch_activities_from_provider`]: the
/// same authentication, but one
/// `get_activity_with_streams` round trip instead of a paged list scan — 200× cheaper for the callers
/// that previously pulled a whole window to find one id, able to reach
/// activities older than any recent window, and on providers with a real
/// detail endpoint (Strava, Garmin) the returned activity carries the
/// laps/splits list rows never had.
///
/// # Errors
///
/// Returns [`AppError`] when authentication fails, and preserves a
/// `ProviderAuthRequired` error verbatim (same reconnect contract as the list
/// sibling); any other fetch failure reads as the activity being unreachable
/// by this id.
pub async fn fetch_activity_from_provider(
    resources: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    provider_name: &str,
    tenant_id: Option<&str>,
    activity_id: &str,
) -> Result<Activity, AppError> {
    let provider = configured_provider(resources, user_id, provider_name, tenant_id).await?;

    provider
        .get_activity_with_streams(activity_id)
        .await
        .map_err(|e| {
            if e.code == ErrorCode::ProviderAuthRequired {
                e
            } else {
                AppError::not_found(format!(
                    "activity {activity_id} not found on {provider_name}: {e}"
                ))
            }
        })
}

/// Build the authenticated provider both fetch helpers above read through:
/// the one chokepoint every provider read authenticates at
/// ([`AuthService::create_authenticated_provider`]), so a mirror backend, a
/// delegated `TrainingPeaks` link and a refreshed token serve these reads as
/// they serve every other.
///
/// A refusal that says the connection needs reconnecting keeps that shape as
/// [`AppError::provider_auth_required`]; any other refusal is an
/// external-service error carrying the refusal's words.
async fn configured_provider(
    resources: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    provider_name: &str,
    tenant_id: Option<&str>,
) -> Result<Box<dyn FitnessProvider>, AppError> {
    AuthService::new(Arc::clone(resources))
        .create_authenticated_provider(provider_name, user_id, tenant_id)
        .await
        .map_err(|response| provider_auth_failure(provider_name, &response))
}

/// Infer workout intensity from recent activities
///
/// Analyzes recent training data to determine current workout intensity:
/// - High: Average > 2 hours/day or high heart rate zones
/// - Moderate: Average 1-2 hours/day
/// - Low: Average < 1 hour/day
///
/// # Arguments
/// * `activities` - List of recent activities to analyze
/// * `days_back` - Number of days the activities span
///
/// # Returns
/// Inferred intensity as "low", "moderate", or "high"
#[must_use]
pub fn infer_workout_intensity(activities: &[Activity], days_back: u32) -> String {
    if activities.is_empty() || days_back == 0 {
        return "moderate".to_owned(); // Default when no data
    }

    // Calculate total training hours
    // Safe: total_seconds is sum of activity durations (typically < 10^9 seconds), well within f64 precision
    let total_seconds: u64 = activities.iter().map(Activity::duration_seconds).sum();
    #[allow(clippy::cast_precision_loss)]
    let total_hours = total_seconds as f64 / 3600.0;
    let avg_hours_per_day = total_hours / f64::from(days_back);

    // Calculate average heart rate if available
    let hr_activities: Vec<_> = activities
        .iter()
        .filter_map(Activity::average_heart_rate)
        .collect();
    let avg_hr = if hr_activities.is_empty() {
        None
    } else {
        // Safe: activity count is bounded by fetch limit (typically 50), well within u32 range
        #[allow(clippy::cast_possible_truncation)]
        let count = hr_activities.len() as u32;
        Some(hr_activities.iter().sum::<u32>() / count)
    };

    // Intensity inference logic
    // High intensity: > 2 hours/day OR high avg HR (> 150 bpm)
    // Moderate: 1-2 hours/day OR moderate avg HR (130-150 bpm)
    // Low: < 1 hour/day AND low avg HR (< 130 bpm)
    if avg_hours_per_day > 2.0 || avg_hr.is_some_and(|hr| hr > 150) {
        "high".to_owned()
    } else if avg_hours_per_day >= 1.0 || avg_hr.is_some_and(|hr| hr >= 130) {
        "moderate".to_owned()
    } else {
        "low".to_owned()
    }
}
