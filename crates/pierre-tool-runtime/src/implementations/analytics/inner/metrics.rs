// ABOUTME: Handler for calculate_metrics tool computing pace, speed, intensity, efficiency
// ABOUTME: Supports both inline activity parameters and provider-fetched activity_id paths
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::implementations::analytics::output::{ActivityMetricsResult, MetricsInputSummary};
use crate::protocol::format::{apply_format_typed, extract_output_format};
use crate::protocol::provider_helpers::{
    create_configured_provider_with_tenant, TenantCredentialContext,
};
use crate::protocol::{UniversalRequest, UniversalResponse, UniversalToolExecutor};
use crate::protocols::ProtocolError;
use pierre_config::constants::limits::{self, METERS_PER_KILOMETER};
use pierre_core::models::TenantId;
use pierre_core::uuid_utils::parse_user_id_for_protocol;
use pierre_formatters::OutputFormat;
use pierre_intelligence::physiological_constants::efficiency_defaults::{
    DEFAULT_EFFICIENCY_SCORE, DEFAULT_EFFICIENCY_WITH_DISTANCE,
};
use pierre_intelligence::physiological_constants::heart_rate::AGE_BASED_MAX_HR_CONSTANT;
use pierre_intelligence::physiological_constants::unit_conversions::MS_TO_KMH_FACTOR;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;

/// Activity parameters extracted from request
struct ActivityParameters {
    distance: f64,
    duration: u64,
    elevation_gain: f64,
    heart_rate: Option<u32>,
    max_hr_provided: Option<f64>,
    user_age: Option<u32>,
}

/// Calculated fitness metrics
struct CalculatedMetrics {
    pace: f64,
    speed: f64,
    intensity_score: f64,
    efficiency_score: f64,
}

/// Parse activity parameters from request
///
/// Extracts activity metrics (distance, duration, elevation, heart rate) and
/// user profile data (max HR, age) from the MCP request parameters.
///
/// # Arguments
/// * `request` - The incoming MCP request with parameters
///
/// # Returns
/// Parsed activity parameters or error if required fields are missing
///
/// # Errors
/// Returns `ProtocolError::InvalidRequest` if activity parameter is missing
fn parse_activity_parameters(
    request: &UniversalRequest,
) -> Result<ActivityParameters, ProtocolError> {
    let activity = request.parameters.get("activity").ok_or_else(|| {
        ProtocolError::InvalidRequest("activity parameter is required".to_owned())
    })?;

    let distance = activity
        .get("distance")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(0.0);

    let duration = activity
        .get("duration")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);

    let elevation_gain = activity
        .get("elevation_gain")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(0.0);

    let heart_rate = activity
        .get("average_heart_rate")
        .and_then(serde_json::Value::as_u64)
        .and_then(|v| u32::try_from(v).ok());

    let max_hr_provided = request
        .parameters
        .get("max_hr")
        .and_then(serde_json::Value::as_f64);

    let user_age = request
        .parameters
        .get("age")
        .and_then(serde_json::Value::as_u64)
        .and_then(|v| u32::try_from(v).ok());

    Ok(ActivityParameters {
        distance,
        duration,
        elevation_gain,
        heart_rate,
        max_hr_provided,
        user_age,
    })
}

/// Determine maximum heart rate
///
/// Determines max HR using priority: 1) explicit value, 2) Fox formula from age
/// (220 - age), 3) default assumed constant. Returns both the calculated value
/// and a source descriptor for transparency.
///
/// # Arguments
/// * `max_hr_provided` - Explicitly provided max HR (highest priority)
/// * `user_age` - User age for Fox formula calculation
///
/// # Returns
/// Tuple of (`max_hr_value`, `source_description`)
fn determine_max_heart_rate(max_hr_provided: Option<f64>, user_age: Option<u32>) -> (f64, String) {
    // Last resort, reached only when the athlete has given neither a measured
    // max HR nor an age. It is the Fox estimate at 40 — a median adult age —
    // and the tuple's source field says `default_assumed` so a caller can tell
    // this apart from a real measurement rather than reading it as one.
    const ASSUMED_MAX_HR: f64 = 180.0;

    match (max_hr_provided, user_age) {
        (Some(hr), _) => (hr, "provided".to_owned()),
        (None, Some(age)) => {
            let max_hr = f64::from(AGE_BASED_MAX_HR_CONSTANT.saturating_sub(age));
            (max_hr, format!("calculated_from_age_{age}"))
        }
        (None, None) => (ASSUMED_MAX_HR, "default_assumed".to_owned()),
    }
}

/// Calculate activity metrics from parameters
///
/// Computes pace (min/km), speed (km/h), intensity score (% of max HR),
/// and efficiency score (distance/elevation ratio). Uses defensive checks
/// to avoid division by zero.
///
/// # Arguments
/// * `params` - Activity parameters (distance, duration, elevation, HR)
/// * `max_hr` - Maximum heart rate for intensity calculation
///
/// # Returns
/// Calculated metrics structure
fn calculate_activity_metrics(params: &ActivityParameters, max_hr: f64) -> CalculatedMetrics {
    let duration_f64 =
        f64::from(u32::try_from(params.duration.min(u64::from(u32::MAX))).unwrap_or(u32::MAX));

    let pace = if params.distance > 0.0 && params.duration > 0 {
        duration_f64 / (params.distance / METERS_PER_KILOMETER)
    } else {
        0.0
    };

    let speed = if params.duration > 0 {
        (params.distance / duration_f64) * MS_TO_KMH_FACTOR
    } else {
        0.0
    };

    let intensity_score = if max_hr > 0.0 {
        params.heart_rate.map_or(DEFAULT_EFFICIENCY_SCORE, |hr| {
            (f64::from(hr) / max_hr) * limits::PERCENTAGE_MULTIPLIER
        })
    } else {
        DEFAULT_EFFICIENCY_SCORE
    };

    let efficiency_score = if params.distance > 0.0 && params.elevation_gain > 0.0 {
        (params.distance / params.elevation_gain).min(100.0)
    } else {
        DEFAULT_EFFICIENCY_WITH_DISTANCE
    };

    CalculatedMetrics {
        pace,
        speed,
        intensity_score,
        efficiency_score,
    }
}

/// Build metrics calculation response
///
/// Constructs the MCP response with calculated metrics, summary data, and metadata.
/// Includes timestamps and personalization flags for transparency.
///
/// # Arguments
/// * `params` - Original activity parameters
/// * `metrics` - Calculated metrics (pace, speed, intensity, efficiency)
/// * `max_hr` - Determined maximum heart rate
/// * `max_hr_source` - Source description for max HR
///
/// # Returns
/// Complete `UniversalResponse` with results and metadata
fn build_metrics_response(
    params: &ActivityParameters,
    metrics: &CalculatedMetrics,
    max_hr: f64,
    max_hr_source: &str,
    output_format: OutputFormat,
) -> Result<UniversalResponse, ProtocolError> {
    use limits;

    let payload = ActivityMetricsResult {
        pace: metrics.pace,
        speed: metrics.speed,
        intensity_score: metrics.intensity_score,
        efficiency_score: metrics.efficiency_score,
        max_hr_used: max_hr,
        max_hr_source: max_hr_source.to_owned(),
        metrics_summary: MetricsInputSummary {
            distance_km: params.distance / METERS_PER_KILOMETER,
            duration_minutes: params.duration / limits::SECONDS_PER_MINUTE,
            elevation_meters: params.elevation_gain,
            average_heart_rate: params.heart_rate,
        },
    };

    let response = UniversalResponse {
        success: true,
        result: None,
        error: None,
        metadata: Some({
            let mut map = HashMap::new();
            map.insert(
                "calculation_timestamp".into(),
                serde_json::Value::String(chrono::Utc::now().to_rfc3339()),
            );
            map.insert(
                "metric_version".into(),
                serde_json::Value::String("2.0".into()),
            );
            map.insert(
                "personalized".into(),
                serde_json::Value::Bool(
                    params.max_hr_provided.is_some() || params.user_age.is_some(),
                ),
            );
            map
        }),
    };

    apply_format_typed(response, payload, output_format)
}

/// Fetch activity from provider and calculate metrics (helper for `activity_id` path)
async fn fetch_and_calculate_metrics(
    executor: &UniversalToolExecutor,
    request: &UniversalRequest,
    activity_id: &str,
    provider_name: &str,
    user_uuid: uuid::Uuid,
    output_format: OutputFormat,
) -> Result<UniversalResponse, ProtocolError> {
    // Get valid token
    let token_data = match executor
        .auth_service
        .get_valid_token(user_uuid, provider_name, request.tenant_id.as_deref())
        .await
    {
        Ok(Some(token)) => token,
        Ok(None) => {
            return Ok(UniversalResponse {
                success: false,
                result: None,
                error: Some(format!(
                    "No valid token for {provider_name}. Please connect using the connect_provider tool first."
                )),
                metadata: None,
            });
        }
        Err(e) => {
            return Ok(UniversalResponse {
                success: false,
                result: None,
                error: Some(format!("Authentication error: {e}")),
                metadata: None,
            });
        }
    };

    // Build tenant credential context for tenant-scoped OAuth resolution
    let tenant_ctx = request
        .tenant_id
        .as_ref()
        .and_then(|tid| TenantId::parse_str(tid).ok())
        .map(|tid| TenantCredentialContext {
            tenant_oauth_client: executor.resources.tenant_oauth_client(),
            tenants: executor.resources.repos().tenants.as_ref(),
            oauth_tokens: executor.resources.repos().oauth_tokens.as_ref(),
            tenant_id: tid,
            user_id: user_uuid,
        });

    // Create configured provider using provider-agnostic helper with tenant credentials
    let provider = create_configured_provider_with_tenant(
        provider_name,
        executor.resources.provider_registry(),
        &token_data,
        tenant_ctx,
    )
    .await
    .map_err(|e| ProtocolError::InternalError(format!("Failed to configure provider: {e}")))?;

    // Fetch activity from provider
    let activity = provider.get_activity(activity_id).await.map_err(|e| {
        ProtocolError::ExecutionFailed(format!("Failed to fetch activity {activity_id}: {e}"))
    })?;

    // Convert Activity model to parameters format
    let mut request_with_activity = request.clone();
    if let Some(params_obj) = request_with_activity.parameters.as_object_mut() {
        params_obj.insert(
            "activity".to_owned(),
            serde_json::json!({
                "distance": activity.distance_meters(),
                "duration": activity.duration_seconds(),
                "elevation_gain": activity.elevation_gain(),
                "average_heart_rate": activity.average_heart_rate(),
            }),
        );
    } else {
        return Err(ProtocolError::InvalidParameters(
            "parameters must be a JSON object".to_owned(),
        ));
    }

    // Parse parameters from converted activity
    let params = parse_activity_parameters(&request_with_activity)?;
    let (max_hr, max_hr_source) = determine_max_heart_rate(params.max_hr_provided, params.user_age);
    let metrics = calculate_activity_metrics(&params, max_hr);

    build_metrics_response(&params, &metrics, max_hr, &max_hr_source, output_format)
}

/// Handle `calculate_metrics` tool - calculate custom fitness metrics (async)
///
/// # Errors
/// Returns `ProtocolError` if activity parameter is missing or calculation fails
#[must_use]
pub fn handle_calculate_metrics(
    executor: &UniversalToolExecutor,
    request: UniversalRequest,
) -> Pin<Box<dyn Future<Output = Result<UniversalResponse, ProtocolError>> + Send + '_>> {
    Box::pin(async move {
        let user_uuid = parse_user_id_for_protocol(&request.user_id)?;

        // Extract output format parameter: "json" (default) or "toon"
        let output_format = extract_output_format(&request);

        // `activity_id` and `provider` are both declared required by this tool's
        // schema, so this is the only way in. A second branch used to accept an
        // `activity` object inline and skip the fetch, but that shape appears
        // nowhere in the schema — no caller following the tool definition could
        // know to send it, and the only thing that ever did was a test. The
        // shared helpers below it stay: `fetch_and_calculate_metrics` builds the
        // same `activity` object from the fetched activity and runs them.
        let activity_id = request
            .parameters
            .get("activity_id")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                ProtocolError::InvalidParameters("activity_id parameter is required".to_owned())
            })?;

        let provider_name = request
            .parameters
            .get("provider")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                ProtocolError::InvalidParameters(
                    "provider parameter required when using activity_id".to_owned(),
                )
            })?;

        fetch_and_calculate_metrics(
            executor,
            &request,
            activity_id,
            provider_name,
            user_uuid,
            output_format,
        )
        .await
    })
}
