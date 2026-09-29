// ABOUTME: Handler for get_activity_intelligence tool with AI-powered analysis
// ABOUTME: Fetches activity data from provider and generates deterministic insights from its metrics
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::implementations::analytics::output::{
    ActivityIntelligence, ActivityIntelligenceResult, ActivityPerformanceMetrics,
    AutoSelectedActivity,
};
use crate::protocol::format::{apply_format_typed, extract_output_format};
use crate::protocol::provider_helpers::resolve_provider_for_request;
use crate::protocol::{
    auth_required_provider, UniversalRequest, UniversalResponse, UniversalToolExecutor,
};
use crate::protocols::ProtocolError;
use pierre_config::constants::limits::METERS_PER_KILOMETER;
use pierre_core::errors::ErrorCode;
use pierre_core::models::Activity;
use pierre_core::untrusted::{display_line, ACTIVITY_NAME_MAX_CHARS};
use pierre_core::uuid_utils::parse_user_id_for_protocol;
use pierre_formatters::OutputFormat;
use pierre_intelligence::physiological_constants::business_thresholds::{
    ACHIEVEMENT_DISTANCE_THRESHOLD_KM, ACHIEVEMENT_ELEVATION_THRESHOLD_M,
};
use pierre_intelligence::physiological_constants::heart_rate::HIGH_INTENSITY_HR_THRESHOLD;
use pierre_providers::core::FitnessProvider;
use serde_json::{json, Value};
use uuid::Uuid;

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;

/// Generate insights and recommendations from activity data
fn generate_activity_insights(activity: &Activity) -> (Vec<String>, Vec<&'static str>) {
    let mut insights = Vec::new();
    let mut recommendations = Vec::new();

    // Analyze distance
    if let Some(distance) = activity.distance_meters() {
        let km = distance / METERS_PER_KILOMETER;
        insights.push(format!("Activity covered {km:.2} km"));
        if km > ACHIEVEMENT_DISTANCE_THRESHOLD_KM {
            recommendations.push("Great long-distance effort! Ensure proper recovery time");
        }
    }

    // Analyze elevation
    if let Some(elevation) = activity.elevation_gain() {
        insights.push(format!("Total elevation gain: {elevation:.0} meters"));
        if elevation > ACHIEVEMENT_ELEVATION_THRESHOLD_M {
            recommendations.push("Significant elevation - consider targeted hill training");
        }
    }

    // Analyze heart rate
    if let Some(avg_hr) = activity.average_heart_rate() {
        insights.push(format!("Average heart rate: {avg_hr} bpm"));
        if avg_hr > HIGH_INTENSITY_HR_THRESHOLD {
            recommendations.push("High-intensity effort detected - monitor recovery");
        }
    }

    // Analyze calories
    if let Some(calories) = activity.calories() {
        insights.push(format!("Calories burned: {calories}"));
    }

    (insights, recommendations)
}

/// Build intelligence response metadata
///
/// Creates metadata map with activity ID, user ID, tenant ID, and analysis type
/// for tracking and audit purposes.
///
/// # Arguments
/// * `activity_id` - Activity identifier
/// * `user_uuid` - User UUID
/// * `tenant_id` - Optional tenant identifier
///
/// # Returns
/// `HashMap` with metadata key-value pairs
fn build_intelligence_metadata(
    activity_id: &str,
    user_uuid: uuid::Uuid,
    tenant_id: Option<String>,
) -> HashMap<String, serde_json::Value> {
    let mut metadata = HashMap::new();
    metadata.insert(
        "activity_id".to_owned(),
        serde_json::Value::String(activity_id.to_owned()),
    );
    metadata.insert(
        "user_id".to_owned(),
        serde_json::Value::String(user_uuid.to_string()),
    );
    metadata.insert(
        "tenant_id".to_owned(),
        tenant_id.map_or(serde_json::Value::Null, serde_json::Value::String),
    );
    metadata.insert(
        "analysis_type".to_owned(),
        serde_json::Value::String("intelligence".to_owned()),
    );
    metadata
}

/// Create the intelligence analysis for an activity from its own metrics.
fn create_intelligence_response(
    activity: &Activity,
    activity_id: &str,
    user_uuid: uuid::Uuid,
    tenant_id: Option<String>,
) -> (
    ActivityIntelligenceResult,
    HashMap<String, serde_json::Value>,
) {
    let (insights, recommendations) = generate_activity_insights(activity);

    let summary = format!(
        "{:?} activity completed. {} insights generated.",
        activity.sport_type(),
        insights.len()
    );

    let duration_minutes = f64::from(
        u32::try_from(activity.duration_seconds().min(u64::from(u32::MAX))).unwrap_or(u32::MAX),
    ) / 60.0;

    let intelligence = ActivityIntelligence {
        summary,
        insights,
        recommendations: recommendations.into_iter().map(ToOwned::to_owned).collect(),
        source: "deterministic".to_owned(),
    };

    let analysis = ActivityIntelligenceResult {
        activity_id: activity_id.to_owned(),
        activity_type: format!("{:?}", activity.sport_type()),
        timestamp: chrono::Utc::now().to_rfc3339(),
        intelligence,
        performance_metrics: ActivityPerformanceMetrics {
            distance_km: activity.distance_meters().map(|d| d / METERS_PER_KILOMETER),
            duration_minutes: Some(duration_minutes),
            elevation_meters: activity.elevation_gain(),
            average_heart_rate: activity.average_heart_rate(),
            max_heart_rate: activity.max_heart_rate(),
            calories: activity.calories(),
        },
        auto_selected: None,
    };

    (
        analysis,
        build_intelligence_metadata(activity_id, user_uuid, tenant_id),
    )
}

/// Fetch activity and create intelligence response
///
/// Retrieves activity data from provider and generates intelligence analysis.
/// Returns error response if activity fetch fails.
///
/// # Arguments
/// * `provider` - Configured activity provider
/// * `activity_id` - Activity identifier to fetch
/// * `user_uuid` - User UUID for response metadata
/// * `tenant_id` - Optional tenant identifier
///
/// # Returns
/// `UniversalResponse` with intelligence or error
async fn fetch_and_analyze_activity(
    provider: Box<dyn FitnessProvider>,
    activity_id: &str,
    user_uuid: uuid::Uuid,
    tenant_id: Option<String>,
    output_format: OutputFormat,
) -> Result<UniversalResponse, ProtocolError> {
    match provider.get_activity(activity_id).await {
        Ok(activity) => {
            let (analysis, metadata) =
                create_intelligence_response(&activity, activity_id, user_uuid, tenant_id);
            apply_format_typed(
                UniversalResponse {
                    success: true,
                    result: None,
                    error: None,
                    metadata: Some(metadata),
                },
                analysis,
                output_format,
            )
        }
        Err(e) => {
            // Handle NotFound by auto-fetching recent activities
            if e.code == ErrorCode::ResourceNotFound {
                // Activity not found - fetch recent activities to show valid IDs
                match provider.get_activities(Some(5), None).await {
                    Ok(activities) if !activities.is_empty() => {
                        let activity_list: Vec<String> = activities
                            .iter()
                            .map(|a| {
                                format!(
                                    "- {} (ID: {}): {} - {:?}",
                                    a.start_date().format("%Y-%m-%d"),
                                    a.id(),
                                    display_line(a.name(), ACTIVITY_NAME_MAX_CHARS),
                                    a.sport_type()
                                )
                            })
                            .collect();

                        let most_recent = &activities[0];

                        // Analyze the most recent activity automatically
                        let (mut analysis, metadata) = create_intelligence_response(
                            most_recent,
                            most_recent.id(),
                            user_uuid,
                            tenant_id,
                        );

                        // Say so on the payload rather than beside it: a client
                        // that reports "your activity" about a substitute is
                        // the failure this field exists to prevent.
                        analysis.auto_selected = Some(AutoSelectedActivity {
                            reason: format!("Activity '{activity_id}' not found"),
                            selected_activity: most_recent.id().to_owned(),
                            selected_activity_name: display_line(
                                most_recent.name(),
                                ACTIVITY_NAME_MAX_CHARS,
                            ),
                            selected_activity_date: most_recent
                                .start_date()
                                .format("%Y-%m-%d")
                                .to_string(),
                            available_activities: activity_list,
                        });

                        return apply_format_typed(
                            UniversalResponse {
                                success: true,
                                result: None,
                                error: None,
                                metadata: Some(metadata),
                            },
                            analysis,
                            output_format,
                        );
                    }
                    Ok(_) => {
                        return Ok(UniversalResponse {
                            success: false,
                            result: None,
                            error: Some(format!("Activity '{activity_id}' not found and no activities available in your account.")),
                            metadata: None,
                        });
                    }
                    Err(fetch_err) => {
                        return Ok(UniversalResponse {
                            success: false,
                            result: None,
                            error: Some(format!("Activity '{activity_id}' not found. Failed to fetch available activities: {fetch_err}")),
                            metadata: None,
                        });
                    }
                }
            }

            // Other errors - generic message
            Ok(UniversalResponse {
                success: false,
                result: None,
                error: Some(format!("Failed to fetch activity {activity_id}: {e}")),
                metadata: None,
            })
        }
    }
}

/// Handle `get_activity_intelligence` tool - get AI analysis for activity (async)
///
/// # Errors
/// Returns `ProtocolError` if `activity_id` parameter is missing or validation fails
#[must_use]
pub fn handle_get_activity_intelligence(
    executor: &UniversalToolExecutor,
    request: UniversalRequest,
) -> Pin<Box<dyn Future<Output = Result<UniversalResponse, ProtocolError>> + Send + '_>> {
    Box::pin(async move {
        use parse_user_id_for_protocol;

        let activity_id = request
            .parameters
            .get("activity_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ProtocolError::InvalidRequest("Missing required parameter: activity_id".to_owned())
            })?;

        let user_uuid = parse_user_id_for_protocol(&request.user_id)?;
        let provider_name = match resolve_provider_for_request(
            &request.parameters,
            executor,
            user_uuid,
            request.tenant_id.as_deref(),
        )
        .await
        {
            Ok(p) => p,
            Err(response) => return Ok(*response),
        };

        // Extract output format parameter: "json" (default) or "toon"
        let output_format = extract_output_format(&request);

        match executor
            .auth_service
            .create_authenticated_provider(&provider_name, user_uuid, request.tenant_id.as_deref())
            .await
        {
            Ok(provider) => {
                let result = fetch_and_analyze_activity(
                    provider,
                    activity_id,
                    user_uuid,
                    request.tenant_id,
                    output_format,
                )
                .await?;

                Ok(result)
            }
            Err(response) => {
                // A lapsed or missing token must surface as the TYPED auth
                // error so the tool loop hands back the localized reconnect
                // link this turn — an in-band error payload strands the
                // athlete with a generic failure the model can only
                // apologise about (same rule as get_activities; live
                // incident 2026-08-11). Metadata is dropped at the
                // ToolResponse boundary, so the typed re-raise here is the
                // only way the signal survives the bridge.
                auth_required_provider(&response).map_or_else(
                    || Ok(*response),
                    |provider| Err(ProtocolError::ProviderAuthRequired { provider }),
                )
            }
        }
    })
}

/// Create metadata for activity analysis responses
fn create_activity_metadata(
    activity_id: &str,
    user_uuid: Uuid,
    tenant_id: Option<&String>,
) -> HashMap<String, Value> {
    let mut map = HashMap::new();
    map.insert(
        "activity_id".to_owned(),
        Value::String(activity_id.to_owned()),
    );
    map.insert("user_id".to_owned(), Value::String(user_uuid.to_string()));
    map.insert(
        "tenant_id".to_owned(),
        tenant_id.map_or(Value::Null, |id| {
            Value::String(id.clone()) // Safe: String ownership for JSON value
        }),
    );
    map
}

/// Process activity analysis when activity is found
///
/// Dispatches into the `get_activity_intelligence` tool through the shared
/// registry (`UniversalToolExecutor::execute_tool`) rather than calling the
/// analytics handler function directly, so `analyze_activity` reaches the
/// handler through whatever `McpTool` impl is registered for it.
pub async fn process_activity_analysis(
    executor: &UniversalToolExecutor,
    mut request: UniversalRequest,
    activity_id: &str,
    user_uuid: Uuid,
) -> Result<UniversalResponse, ProtocolError> {
    "get_activity_intelligence".clone_into(&mut request.tool_name);
    let analysis_response = executor.execute_tool(request).await?;
    let metadata = Some(create_activity_metadata(
        activity_id,
        user_uuid,
        analysis_response
            .metadata
            .as_ref()
            .and_then(|m| m.get("tenant_id").and_then(Value::as_str).map(String::from))
            .as_ref(),
    ));

    // Propagate the inner verdict — hardcoding `success: true` here reported
    // every downstream failure as a successful call with an error payload.
    Ok(UniversalResponse {
        success: analysis_response.success,
        result: analysis_response.result.or_else(|| Some(json!({}))),
        error: analysis_response.error,
        metadata,
    })
}
