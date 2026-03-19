// ABOUTME: Handler for get_activity_intelligence tool with AI-powered analysis
// ABOUTME: Fetches activity data from provider and generates insights via MCP sampling or static analysis
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::config::environment::default_provider;
use crate::constants::limits::METERS_PER_KILOMETER;
use crate::errors::{AppResult, ErrorCode};
use crate::intelligence::physiological_constants::business_thresholds::{
    ACHIEVEMENT_DISTANCE_THRESHOLD_KM, ACHIEVEMENT_ELEVATION_THRESHOLD_M,
};
use crate::intelligence::physiological_constants::heart_rate::HIGH_INTENSITY_HR_THRESHOLD;
use crate::mcp::sampling_peer::SamplingPeer;
use crate::mcp::schema::{Content, CreateMessageRequest, ModelPreferences, PromptMessage};
use pierre_llm::prompts::{ACTIVITY_ANALYSIS_PROMPT, ACTIVITY_ANALYSIS_SYSTEM_PROMPT};

const ACTIVITY_SUMMARY_PLACEHOLDER: &str = "{activity_summary}";
use crate::models::Activity;
use crate::protocols::universal::handlers::{apply_format_to_response, extract_output_format};
use crate::protocols::universal::{UniversalRequest, UniversalResponse, UniversalToolExecutor};
use crate::protocols::ProtocolError;
use crate::providers::core::FitnessProvider;
use crate::utils::uuid::parse_user_id_for_protocol;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tracing::{info, warn};

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

/// Create intelligence analysis JSON response with optional MCP sampling
async fn create_intelligence_response(
    activity: &Activity,
    activity_id: &str,
    user_uuid: uuid::Uuid,
    tenant_id: Option<String>,
    sampling_peer: Option<&Arc<SamplingPeer>>,
) -> UniversalResponse {
    // Try MCP sampling first if available (uses client's LLM)
    if let Some(peer) = sampling_peer {
        match generate_activity_intelligence_via_sampling(peer, activity).await {
            Ok(llm_analysis) => {
                info!("Generated activity intelligence using MCP sampling");
                return UniversalResponse {
                    success: true,
                    result: Some(llm_analysis),
                    error: None,
                    metadata: Some({
                        let mut map = HashMap::new();
                        map.insert(
                            "activity_id".to_owned(),
                            serde_json::Value::String(activity_id.to_owned()),
                        );
                        map.insert(
                            "user_id".to_owned(),
                            serde_json::Value::String(user_uuid.to_string()),
                        );
                        if let Some(tid) = tenant_id.clone() {
                            map.insert("tenant_id".to_owned(), serde_json::Value::String(tid));
                        }
                        map.insert(
                            "analysis_source".to_owned(),
                            serde_json::Value::String("mcp_sampling".to_owned()),
                        );
                        map
                    }),
                };
            }
            Err(e) => {
                warn!(
                    "MCP sampling failed, falling back to static analysis: {}",
                    e
                );
            }
        }
    }

    // Fall back to static analysis
    let (insights, recommendations) = generate_activity_insights(activity);

    let summary = format!(
        "{:?} activity completed. {} insights generated.",
        activity.sport_type(),
        insights.len()
    );

    let duration_minutes = f64::from(
        u32::try_from(activity.duration_seconds().min(u64::from(u32::MAX))).unwrap_or(u32::MAX),
    ) / 60.0;

    let analysis = serde_json::json!({
        "activity_id": activity_id,
        "activity_type": format!("{:?}", activity.sport_type()),
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "intelligence": {
            "summary": summary,
            "insights": insights,
            "recommendations": recommendations,
            "performance_metrics": {
                "distance_km": activity.distance_meters().map(|d| d / METERS_PER_KILOMETER),
                "duration_minutes": Some(duration_minutes),
                "elevation_meters": activity.elevation_gain(),
                "average_heart_rate": activity.average_heart_rate(),
                "max_heart_rate": activity.max_heart_rate(),
                "calories": activity.calories()
            }
        }
    });

    let metadata = build_intelligence_metadata(activity_id, user_uuid, tenant_id);

    UniversalResponse {
        success: true,
        result: Some(analysis),
        error: None,
        metadata: Some(metadata),
    }
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
    sampling_peer: Option<&Arc<SamplingPeer>>,
) -> UniversalResponse {
    match provider.get_activity(activity_id).await {
        Ok(activity) => {
            create_intelligence_response(
                &activity,
                activity_id,
                user_uuid,
                tenant_id,
                sampling_peer,
            )
            .await
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
                                    a.name(),
                                    a.sport_type()
                                )
                            })
                            .collect();

                        let most_recent = &activities[0];

                        // Analyze the most recent activity automatically
                        let mut response = create_intelligence_response(
                            most_recent,
                            most_recent.id(),
                            user_uuid,
                            tenant_id,
                            None, // No sampling in fallback path
                        )
                        .await;

                        // Add auto-selection note to the result
                        if let Some(result) = response.result.as_mut() {
                            result["auto_selected"] = serde_json::json!({
                                "reason": format!("Activity '{activity_id}' not found"),
                                "selected_activity": most_recent.id(),
                                "selected_activity_name": most_recent.name(),
                                "selected_activity_date": most_recent.start_date().format("%Y-%m-%d").to_string(),
                                "available_activities": activity_list
                            });
                        }

                        return response;
                    }
                    Ok(_) => {
                        return UniversalResponse {
                            success: false,
                            result: None,
                            error: Some(format!("Activity '{activity_id}' not found and no activities available in your account.")),
                            metadata: None,
                        };
                    }
                    Err(fetch_err) => {
                        return UniversalResponse {
                            success: false,
                            result: None,
                            error: Some(format!("Activity '{activity_id}' not found. Failed to fetch available activities: {fetch_err}")),
                            metadata: None,
                        };
                    }
                }
            }

            // Other errors - generic message
            UniversalResponse {
                success: false,
                result: None,
                error: Some(format!("Failed to fetch activity {activity_id}: {e}")),
                metadata: None,
            }
        }
    }
}

/// Generate activity intelligence via MCP sampling
///
/// Analyzes a single activity using the client's LLM via MCP sampling for AI-powered insights.
///
/// # Arguments
/// * `sampling_peer` - MCP sampling peer for LLM requests
/// * `activity` - Activity to analyze
///
/// # Returns
/// JSON response with LLM-generated activity analysis
///
/// # Errors
/// Returns error if sampling request fails or response is invalid
async fn generate_activity_intelligence_via_sampling(
    sampling_peer: &Arc<SamplingPeer>,
    activity: &Activity,
) -> AppResult<serde_json::Value> {
    use {Content, CreateMessageRequest, ModelPreferences, PromptMessage};

    // Prepare activity data for LLM analysis
    #[allow(clippy::cast_precision_loss)]
    let duration_min = activity.duration_seconds() as f64 / 60.0;
    let distance_km = activity.distance_meters().map(|d| d / 1000.0);
    let avg_pace = activity
        .average_speed()
        .map(|s| if s > 0.0 { 1000.0 / (s * 60.0) } else { 0.0 });

    let activity_summary = format!(
        "Activity Type: {:?}\n\
         Duration: {duration_min:.1} minutes\n\
         Distance: {}\n\
         Average Pace: {}\n\
         Average Heart Rate: {}\n\
         Calories: {}",
        activity.sport_type(),
        distance_km.map_or_else(|| "N/A".to_owned(), |d| format!("{d:.2} km")),
        avg_pace.map_or_else(|| "N/A".to_owned(), |p| format!("{p:.2} min/km")),
        activity
            .average_heart_rate()
            .map_or_else(|| "N/A".to_owned(), |hr| format!("{hr} bpm")),
        activity
            .calories()
            .map_or_else(|| "N/A".to_owned(), |c| c.to_string())
    );

    // Create prompt for LLM from template
    let prompt = ACTIVITY_ANALYSIS_PROMPT.replace(ACTIVITY_SUMMARY_PLACEHOLDER, &activity_summary);

    // Send sampling request to client's LLM
    let request = CreateMessageRequest {
        messages: vec![PromptMessage::user(Content::Text { text: prompt })],
        model_preferences: Some(ModelPreferences {
            // Hint for high-quality model - client decides actual model
            hints: None,
            intelligence_priority: Some(0.9),
            cost_priority: None,
            speed_priority: None,
        }),
        max_tokens: 800,
        temperature: Some(0.7),
        system_prompt: Some(ACTIVITY_ANALYSIS_SYSTEM_PROMPT.trim().to_owned()),
        include_context: None,
        stop_sequences: None,
        metadata: None,
    };

    let result = sampling_peer.create_message(request).await?;

    // Parse LLM response
    serde_json::from_str::<serde_json::Value>(&result.content.text).or_else(|_| {
        // Wrap non-JSON response
        Ok(serde_json::json!({
            "summary": result.content.text,
            "insights": [result.content.text],
            "recommendations": [],
            "analysis_type": "ai_powered",
            "source": "mcp_sampling"
        }))
    })
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

        // Check cancellation at start
        if let Some(token) = &request.cancellation_token {
            if token.is_cancelled().await {
                return Err(ProtocolError::OperationCancelled(
                    "get_activity_intelligence cancelled by user".to_owned(),
                ));
            }
        }

        let activity_id = request
            .parameters
            .get("activity_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ProtocolError::InvalidRequest("Missing required parameter: activity_id".to_owned())
            })?;

        let provider_name = request
            .parameters
            .get("provider")
            .and_then(|v| v.as_str())
            .map_or_else(default_provider, String::from);
        let user_uuid = parse_user_id_for_protocol(&request.user_id)?;

        // Extract output format parameter: "json" (default) or "toon"
        let output_format = extract_output_format(&request);

        // Report progress - starting authentication
        if let Some(reporter) = &request.progress_reporter {
            reporter.report(
                25.0,
                Some(100.0),
                Some("Checking authentication...".to_owned()),
            );
        }

        // Check cancellation before auth
        if let Some(token) = &request.cancellation_token {
            if token.is_cancelled().await {
                return Err(ProtocolError::OperationCancelled(
                    "get_activity_intelligence cancelled before authentication".to_owned(),
                ));
            }
        }

        match executor
            .auth_service
            .create_authenticated_provider(&provider_name, user_uuid, request.tenant_id.as_deref())
            .await
        {
            Ok(provider) => {
                // Report progress after auth
                if let Some(reporter) = &request.progress_reporter {
                    reporter.report(
                        50.0,
                        Some(100.0),
                        Some("Authenticated - analyzing activity...".to_owned()),
                    );
                }

                // Check cancellation before analysis
                if let Some(token) = &request.cancellation_token {
                    if token.is_cancelled().await {
                        return Err(ProtocolError::OperationCancelled(
                            "get_activity_intelligence cancelled before analysis".to_owned(),
                        ));
                    }
                }

                let result = fetch_and_analyze_activity(
                    provider,
                    activity_id,
                    user_uuid,
                    request.tenant_id,
                    executor.resources.sampling_peer.as_ref(),
                )
                .await;

                // Report completion on success
                if result.success {
                    if let Some(reporter) = &request.progress_reporter {
                        reporter.report(
                            100.0,
                            Some(100.0),
                            Some("Activity intelligence retrieved".to_owned()),
                        );
                    }
                }

                // Apply format transformation
                Ok(apply_format_to_response(
                    result,
                    "intelligence",
                    output_format,
                ))
            }
            Err(response) => Ok(response),
        }
    })
}
