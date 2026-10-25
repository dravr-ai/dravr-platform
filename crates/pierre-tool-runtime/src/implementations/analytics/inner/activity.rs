// ABOUTME: Handler for get_activity_intelligence tool with AI-powered analysis
// ABOUTME: Fetches activity data from provider and generates deterministic insights from its metrics
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::activity_fetch::StaleReason;
use crate::implementations::analytics::output::{
    ActivityIntelligence, ActivityIntelligenceResult, ActivityPerformanceMetrics,
    AutoSelectedActivity,
};
use crate::implementations::last_known::{self, LiveRead};
use crate::implementations::stored_physiology::stored_hr_zones;
use crate::protocol::format::{apply_format_typed, extract_output_format};
use crate::protocol::provider_helpers::resolve_provider_for_request;
use crate::protocol::{
    auth_required_provider, UniversalRequest, UniversalResponse, UniversalToolExecutor,
};
use crate::protocols::ProtocolError;
use dravr_cageux::physiological_constants::business_thresholds::{
    ACHIEVEMENT_DISTANCE_THRESHOLD_KM, ACHIEVEMENT_ELEVATION_THRESHOLD_M,
};
use dravr_cageux::zones::{HrIntensity, HrZones};
use pierre_config::constants::limits::METERS_PER_KILOMETER;
use pierre_core::errors::ErrorCode;
use pierre_core::models::zones::HrZonesUnavailable;
use pierre_core::models::{Activity, TenantId};
use pierre_core::untrusted::{display_line, ACTIVITY_NAME_MAX_CHARS};
use pierre_core::uuid_utils::parse_user_id_for_protocol;
use pierre_formatters::OutputFormat;
use pierre_providers::core::FitnessProvider;
use pierre_providers::{ai_scope, backend_resolver};
use serde_json::{json, Value};
use uuid::Uuid;

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;

/// The athlete's heart-rate zones as this tool reads them: the zones, or why
/// there are none.
type AthleteHrZones = Result<HrZones, HrZonesUnavailable>;

/// Generate insights and recommendations from activity data
///
/// The session's average heart rate is read against the athlete's own zones:
/// zone 4 or above is a high-intensity effort. Without the zones the heart
/// rate is reported but not graded.
fn generate_activity_insights(
    activity: &Activity,
    hr_zones: Option<&HrZones>,
) -> (Vec<String>, Vec<&'static str>) {
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
        let bpm = u16::try_from(avg_hr).unwrap_or(u16::MAX);
        if hr_zones.is_some_and(|zones| zones.intensity(bpm) >= HrIntensity::Hard) {
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
    user_uuid: Uuid,
    tenant_id: Option<String>,
) -> HashMap<String, Value> {
    let mut metadata = HashMap::new();
    metadata.insert(
        "activity_id".to_owned(),
        Value::String(activity_id.to_owned()),
    );
    metadata.insert("user_id".to_owned(), Value::String(user_uuid.to_string()));
    metadata.insert(
        "tenant_id".to_owned(),
        tenant_id.map_or(Value::Null, Value::String),
    );
    metadata.insert(
        "analysis_type".to_owned(),
        Value::String("intelligence".to_owned()),
    );
    metadata
}

/// Create the intelligence analysis for an activity from its own metrics.
fn create_intelligence_response(
    activity: &Activity,
    activity_id: &str,
    user_uuid: Uuid,
    tenant_id: Option<String>,
    hr_zones: &AthleteHrZones,
) -> (ActivityIntelligenceResult, HashMap<String, Value>) {
    let (insights, recommendations) = generate_activity_insights(activity, hr_zones.as_ref().ok());
    // The reason is carried only when there was a heart rate to grade.
    let hr_zones_unavailable = activity
        .average_heart_rate()
        .and_then(|_| hr_zones.as_ref().err())
        .map(HrZonesUnavailable::reason);

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
        hr_zones_unavailable,
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
    user_uuid: Uuid,
    tenant_id: Option<String>,
    hr_zones: &AthleteHrZones,
    output_format: OutputFormat,
) -> Result<UniversalResponse, ProtocolError> {
    match provider.get_activity(activity_id).await {
        Ok(activity) => {
            let (analysis, metadata) = create_intelligence_response(
                &activity,
                activity_id,
                user_uuid,
                tenant_id,
                hr_zones,
            );
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
                            hr_zones,
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

            // Other errors: a refused credential stays typed.
            Ok(last_known::provider_failure(
                &format!("Failed to fetch activity {activity_id}"),
                &e,
            ))
        }
    }
}

/// Handle `get_activity_intelligence` tool - get AI analysis for activity (async)
///
/// # Errors
/// Returns `ProtocolError` if `activity_id` parameter is missing or validation fails
pub(crate) fn handle_get_activity_intelligence(
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

        // The athlete's heart-rate zones, which grade the session's intensity.
        let hr_zones =
            stored_hr_zones(&executor.resources, request.tenant_id.as_deref(), user_uuid)
                .await
                .map_err(|e| {
                    ProtocolError::InternalError(format!(
                        "get_activity_intelligence: reading the athlete's zones failed: {e}"
                    ))
                })?;

        match executor
            .auth_service
            .create_authenticated_provider(&provider_name, user_uuid, request.tenant_id.as_deref())
            .await
        {
            Ok(provider) => {
                // The live read gets the refresh budget; late or failed, the
                // activity's cached row answers, with a caveat.
                let read = {
                    let (id, tenant) = (activity_id.to_owned(), request.tenant_id.clone());
                    let zones = hr_zones.clone();
                    async move {
                        fetch_and_analyze_activity(
                            provider,
                            &id,
                            user_uuid,
                            tenant,
                            &zones,
                            output_format,
                        )
                        .await
                        .unwrap_or_else(|e| last_known::failed_read(&e))
                    }
                };
                match last_known::read_within_budget(&executor.resources.config().timeouts, read)
                    .await
                {
                    LiveRead::Answered(response) => Ok(response),
                    LiveRead::AuthRequired(provider) => {
                        Err(ProtocolError::ProviderAuthRequired { provider })
                    }
                    LiveRead::Missed(reason, failed) => {
                        let cached = CachedRead {
                            provider: &provider_name,
                            activity_id,
                            user_uuid,
                            tenant_id: request.tenant_id,
                            hr_zones: &hr_zones,
                            output_format,
                        };
                        Ok(cached.serve(executor, reason).await?.unwrap_or(failed))
                    }
                }
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

/// The activity a missed live read asked for, to be served from its cached row.
struct CachedRead<'a> {
    provider: &'a str,
    activity_id: &'a str,
    user_uuid: Uuid,
    tenant_id: Option<String>,
    hr_zones: &'a AthleteHrZones,
    output_format: OutputFormat,
}

impl CachedRead<'_> {
    /// The intelligence for the activity's cached row under a
    /// `provider_unavailable` caveat, or `None` when no row is cached or the
    /// request names no tenant to read it under.
    async fn serve(
        self,
        executor: &UniversalToolExecutor,
        reason: StaleReason,
    ) -> Result<Option<UniversalResponse>, ProtocolError> {
        let Some(tenant) = self
            .tenant_id
            .as_deref()
            .and_then(|t| Uuid::parse_str(t).ok())
            .map(TenantId::from_uuid)
        else {
            return Ok(None);
        };
        let repos = executor.resources.repos();
        let backend = backend_resolver::resolve_backend(
            &repos.auth_repos(),
            self.user_uuid,
            Some(tenant),
            self.provider,
        )
        .await;
        let cache = &repos.activity_cache;
        let Ok(Some(activity)) = cache
            .get_cached_activity(self.user_uuid, &tenant, &backend, self.activity_id)
            .await
        else {
            return Ok(None);
        };
        // The copy reaches the model like a live read, so the provider's AI
        // terms hold on it too: a row they keep from AI is served as no copy.
        let Some(activity) = ai_scope::filter_activities(
            executor.resources.provider_registry().as_ref(),
            vec![activity],
        )
        .into_iter()
        .next() else {
            return Ok(None);
        };
        let synced_at = cache
            .latest_activity_sync(self.user_uuid, &tenant, &backend)
            .await
            .ok()
            .flatten();
        let (analysis, metadata) = create_intelligence_response(
            &activity,
            self.activity_id,
            self.user_uuid,
            self.tenant_id,
            self.hr_zones,
        );
        let mut served = apply_format_typed(
            UniversalResponse {
                success: true,
                result: None,
                error: None,
                metadata: Some(metadata),
            },
            analysis,
            self.output_format,
        )?;
        last_known::attach_caveat(
            &mut served,
            last_known::caveat(&backend, "activity", reason, synced_at),
        );
        Ok(Some(served))
    }
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
pub(crate) async fn process_activity_analysis(
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

#[cfg(test)]
mod tests {
    use std::error::Error;

    use chrono::Utc;
    use pierre_core::models::activity::ActivityBuilder;
    use pierre_core::models::SportType;

    use super::*;

    fn run_at(avg_hr: u32) -> Activity {
        ActivityBuilder::new("run-1", "Run", SportType::Run, Utc::now(), 3600, "test")
            .average_heart_rate(avg_hr)
            .build()
    }

    const HIGH_INTENSITY: &str = "High-intensity effort detected - monitor recovery";

    #[test]
    fn a_heart_rate_is_graded_on_the_athletes_zones() -> Result<(), Box<dyn Error>> {
        // Resting 50, maximum 190: 165 bpm is zone 4, 140 bpm zone 2.
        let zones = HrZones::for_athlete(None, 50, 190)?;
        let (_, hard) = generate_activity_insights(&run_at(165), Some(&zones));
        assert!(hard.contains(&HIGH_INTENSITY), "{hard:?}");
        let (_, easy) = generate_activity_insights(&run_at(140), Some(&zones));
        assert!(!easy.contains(&HIGH_INTENSITY), "{easy:?}");

        // The same 165 bpm on an athlete with a higher ceiling (resting 40,
        // maximum 205) is only zone 3: no fixed bpm decides it.
        let fitter = HrZones::for_athlete(None, 40, 205)?;
        let (_, moderate) = generate_activity_insights(&run_at(165), Some(&fitter));
        assert!(!moderate.contains(&HIGH_INTENSITY), "{moderate:?}");
        Ok(())
    }

    #[test]
    fn without_zones_the_heart_rate_is_reported_and_the_reason_given() {
        let (insights, recommendations) = generate_activity_insights(&run_at(185), None);
        assert!(insights
            .iter()
            .any(|line| line == "Average heart rate: 185 bpm"));
        assert!(!recommendations.contains(&HIGH_INTENSITY));

        let missing = HrZonesUnavailable::MissingHeartRates {
            resting_hr: true,
            max_hr: false,
        };
        let (analysis, _) = create_intelligence_response(
            &run_at(185),
            "run-1",
            Uuid::new_v4(),
            None,
            &Err(missing.clone()),
        );
        assert_eq!(
            analysis.intelligence.hr_zones_unavailable,
            Some(missing.reason())
        );
    }
}
