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
use crate::implementations::provider_context_reads::{
    read_activity_context, source_label, ContextLimits, CONTEXT_READ_BUDGET,
};
use crate::implementations::stored_copy::{
    activities_or_stored, attach_caveat, caveat, with_caveat, Reader, Served,
};
use crate::implementations::stored_physiology::stored_hr_zones;
use crate::protocol::format::{apply_format_typed, extract_output_format};
use crate::protocol::provider_helpers::{by_id_reader, resolve_provider_for_request};
use crate::protocol::{
    auth_required_provider, UniversalRequest, UniversalResponse, UniversalToolExecutor,
};
use crate::protocols::ProtocolError;
use dravr_cageux::physiological_constants::business_thresholds::{
    ACHIEVEMENT_DISTANCE_THRESHOLD_KM, ACHIEVEMENT_ELEVATION_THRESHOLD_M,
};
use dravr_cageux::zones::{HrIntensity, HrZones};
use pierre_config::constants::units::METERS_PER_KM;
use pierre_core::errors::{AppResult, ErrorCode};
use pierre_core::models::zones::HrZonesUnavailable;
use pierre_core::models::{Activity, TenantId};
use pierre_core::untrusted::{display_line, ACTIVITY_NAME_MAX_CHARS};
use pierre_core::uuid_utils::parse_user_id_for_protocol;
use pierre_formatters::OutputFormat;
use pierre_providers::core::FitnessProvider;
use pierre_providers::deduplication::{merge_duplicates, DedupConfig};
use pierre_providers::{ai_scope, backend_resolver};
use serde_json::{json, Value};
use tracing::warn;
use uuid::Uuid;

use dravr_tronc::dates::Timestamp;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// The athlete's heart-rate zones as this tool reads them: the zones, or why
/// there are none.
type AthleteHrZones = Result<HrZones, HrZonesUnavailable>;

/// How many of the connection's recent workouts a missed activity id lists.
const MISSED_ID_RECENT_LIMIT: usize = 5;

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
        let km = distance / METERS_PER_KM;
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
        timestamp: Timestamp::now().to_rfc3339(),
        intelligence,
        performance_metrics: ActivityPerformanceMetrics {
            distance_km: activity.distance_meters().map(|d| d / METERS_PER_KM),
            duration_minutes: Some(duration_minutes),
            elevation_meters: activity.elevation_gain(),
            average_heart_rate: activity.average_heart_rate(),
            max_heart_rate: activity.max_heart_rate(),
            calories: activity.calories(),
        },
        auto_selected: None,
        provider_context: None,
    };

    (
        analysis,
        build_intelligence_metadata(activity_id, user_uuid, tenant_id),
    )
}

/// The activity an analysis is asked for, whose it is, and the provider it is
/// read from as the athlete knows it.
#[derive(Debug, Clone, Copy)]
struct Asked<'req> {
    /// The provider's id for the activity.
    activity_id: &'req str,
    /// The athlete, for the response metadata.
    user_uuid: Uuid,
    /// The request's tenant, for the response metadata.
    tenant_id: Option<&'req str>,
    /// The provider's name for the athlete (`Strava`), labelling its figures.
    source: &'req str,
    /// How the provider's own figures are read beside the activity; `None`
    /// when they are not read at all (the provider's terms keep them from
    /// this caller).
    context: Option<ContextLimits<'req>>,
}

/// Fetch activity and create intelligence response
///
/// Retrieves activity data from provider and generates intelligence analysis.
/// Returns error response if activity fetch fails.
///
/// # Arguments
/// * `provider` - Configured activity provider
/// * `reader` - The reader of the copy standing for the activity's workout,
///   when it is not `provider` (an export or upload the athlete brought in)
/// * `asked` - The activity to fetch, whose it is, and how its context is read
/// * `recent` - The connection's recent workouts, awaited only when the id
///   misses; a stored copy standing in for them carries its caveat
///
/// # Returns
/// `UniversalResponse` with intelligence or error
async fn fetch_and_analyze_activity(
    provider: &dyn FitnessProvider,
    reader: Option<Box<dyn FitnessProvider>>,
    asked: Asked<'_>,
    hr_zones: &AthleteHrZones,
    output_format: OutputFormat,
    recent: impl Future<Output = AppResult<Served<Vec<Activity>>>> + Send,
) -> Result<UniversalResponse, ProtocolError> {
    let Asked {
        activity_id,
        user_uuid,
        tenant_id,
        source,
        context,
    } = asked;
    let tenant_id = tenant_id.map(str::to_owned);
    let target = reader.as_deref().unwrap_or(provider);
    // The provider's own zones and best efforts are read beside the activity,
    // from the provider that owns the id: never from the copy an upload
    // stands in with, whose id is not the provider's.
    let context_read = async {
        let limits = context.filter(|_| reader.is_none())?;
        read_activity_context(provider, source, activity_id, limits).await
    };
    let (fetched, provider_context) = tokio::join!(target.get_activity(activity_id), context_read);
    match fetched {
        Ok(activity) => {
            let (mut analysis, metadata) = create_intelligence_response(
                &activity,
                activity_id,
                user_uuid,
                tenant_id,
                hr_zones,
            );
            analysis.provider_context = provider_context;
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
                // The connection's recent workouts, each recording of one
                // merged into its session as the activity list shows it.
                let recent = recent.await.map(|served| {
                    served.map(|rows| merge_duplicates(rows, &DedupConfig::from_env()).0)
                });
                match recent {
                    Ok(Served {
                        value: activities,
                        caveat,
                    }) if !activities.is_empty() => {
                        let activity_list: Vec<String> = activities
                            .iter()
                            .map(|a| {
                                format!(
                                    "- {} (ID: {}): {} - {:?}",
                                    a.start_date().utc_date(),
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
                            selected_activity_date: most_recent.start_date().utc_date().to_string(),
                            available_activities: activity_list,
                        });

                        let formatted = apply_format_typed(
                            UniversalResponse {
                                success: true,
                                result: None,
                                error: None,
                                metadata: Some(metadata),
                            },
                            analysis,
                            output_format,
                        );
                        return with_caveat(formatted, caveat);
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
                        warn!(error = %fetch_err, "listing activities after a missed id failed");
                        return Ok(UniversalResponse {
                            success: false,
                            result: None,
                            error: Some(format!(
                                "Activity '{activity_id}' not found. Failed to fetch available activities: {}",
                                fetch_err.tool_message()
                            )),
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
                // The copy standing for the workout answers, live and cached.
                let (reader_name, reader) = match by_id_reader(
                    &executor.resources,
                    user_uuid,
                    &provider_name,
                    request.tenant_id.as_deref(),
                    activity_id,
                )
                .await
                {
                    Ok(Some((name, reader))) => (name, Some(reader)),
                    Ok(None) => (provider_name.clone(), None),
                    Err(e) => {
                        return Ok(last_known::provider_failure(
                            &format!("Failed to fetch activity {activity_id}"),
                            &e,
                        ))
                    }
                };
                // The live read gets the refresh budget; late or failed, the
                // activity's cached row answers, with a caveat.
                let read = {
                    let (id, tenant) = (activity_id.to_owned(), request.tenant_id.clone());
                    let runtime = Arc::clone(&executor.resources);
                    let zones = hr_zones.clone();
                    let registry = executor.resources.provider_registry();
                    let source = source_label(registry, &provider_name);
                    let context_budget =
                        registry.session_context_budget(provider.name(), user_uuid);
                    // A caller the provider's terms refuse is never asked
                    // for: its refused reads would spend the athlete's
                    // context-read windows for nothing.
                    let served =
                        ai_scope::first_party_only_read(registry.as_ref(), provider.name(), None)
                            .is_ok();
                    async move {
                        // A missed id lists the recent workouts; a read the
                        // session-read cap refused lists the stored ones
                        // (carnet#1661).
                        let store = Reader::new(&runtime, user_uuid, tenant.as_deref());
                        let recent =
                            activities_or_stored(store, provider.as_ref(), MISSED_ID_RECENT_LIMIT);
                        fetch_and_analyze_activity(
                            provider.as_ref(),
                            reader,
                            Asked {
                                activity_id: &id,
                                user_uuid,
                                tenant_id: tenant.as_deref(),
                                source: &source,
                                context: served.then_some(ContextLimits {
                                    time: CONTEXT_READ_BUDGET,
                                    budget: context_budget.as_ref(),
                                }),
                            },
                            &zones,
                            output_format,
                            recent,
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
                            provider: &reader_name,
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
        let backend =
            backend_resolver::resolve_backend(repos, self.user_uuid, Some(tenant), self.provider)
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
        attach_caveat(&mut served, caveat(&backend, "activity", reason, synced_at));
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

    use async_trait::async_trait;
    use dravr_tronc::dates::{Elapsed, Timestamp};
    use pierre_core::errors::{AppError, AppResult};
    use pierre_core::models::activity::ActivityBuilder;
    use pierre_core::models::{Athlete, SportType};
    use pierre_providers::core::{ActivityQueryParams, OAuth2Credentials, ProviderConfig};
    use pierre_providers::pagination::{CursorPage, PaginationParams};
    use pierre_providers::CoreFitnessProvider;

    use super::*;

    fn run_at(avg_hr: u32) -> Activity {
        ActivityBuilder::new(
            "run-1",
            "Run",
            SportType::Run,
            Timestamp::now(),
            3600,
            "test",
        )
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

    /// A connection whose by-id read always misses, listing a Garmin
    /// auto-split: a 2-minute warm-up piece and the 58-minute session it
    /// belongs to, back to back, warm-up first (newest first).
    struct AutoSplitOnly {
        config: ProviderConfig,
    }

    #[async_trait]
    impl CoreFitnessProvider for AutoSplitOnly {
        fn name(&self) -> &'static str {
            "garmin"
        }

        fn config(&self) -> &ProviderConfig {
            &self.config
        }

        async fn set_credentials(&self, _: OAuth2Credentials) -> AppResult<()> {
            Ok(())
        }

        async fn is_authenticated(&self) -> bool {
            true
        }

        async fn refresh_token_if_needed(&self) -> AppResult<()> {
            Ok(())
        }

        async fn get_athlete(&self) -> AppResult<Athlete> {
            Err(AppError::internal("not read by this test"))
        }

        async fn get_activities_with_params(
            &self,
            _: &ActivityQueryParams,
        ) -> AppResult<Vec<Activity>> {
            let start = Timestamp::now() - Elapsed::hours(3);
            let main = ActivityBuilder::new(
                "main",
                "Morning Run",
                SportType::Run,
                start,
                3_480,
                "garmin",
            )
            .distance_meters(11_000.0)
            .build();
            let warm_up = ActivityBuilder::new(
                "warm-up",
                "Morning Run",
                SportType::Run,
                start + Elapsed::seconds(3_482),
                120,
                "garmin",
            )
            .distance_meters(300.0)
            .build();
            Ok(vec![warm_up, main])
        }

        async fn get_activities_cursor(
            &self,
            _: &PaginationParams,
        ) -> AppResult<CursorPage<Activity>> {
            Err(AppError::internal("not read by this test"))
        }

        async fn get_activity(&self, id: &str) -> AppResult<Activity> {
            Err(AppError::not_found(format!("activity {id}")))
        }
    }

    /// When the asked id misses, the recent workouts offered instead are the
    /// merged sessions the activity list shows, and the one analysed is a
    /// whole session, never a piece of one.
    #[tokio::test]
    async fn a_missed_id_offers_whole_sessions_not_their_pieces() {
        let provider = AutoSplitOnly {
            config: ProviderConfig {
                name: "garmin".to_owned(),
                auth_url: String::new(),
                token_url: String::new(),
                api_base_url: String::new(),
                revoke_url: None,
                default_scopes: Vec::new(),
            },
        };
        let recent = async {
            provider
                .get_activities(Some(MISSED_ID_RECENT_LIMIT), None)
                .await
                .map(Served::live)
        };
        let response = fetch_and_analyze_activity(
            &provider,
            None,
            Asked {
                activity_id: "unknown",
                user_uuid: Uuid::new_v4(),
                tenant_id: None,
                source: "Garmin",
                context: None,
            },
            &Err(HrZonesUnavailable::MissingHeartRates {
                resting_hr: true,
                max_hr: true,
            }),
            OutputFormat::Json,
            recent,
        )
        .await
        .unwrap();
        let result = response.result.expect("the substitute is analysed");
        let auto = &result["auto_selected"];
        assert_eq!(auto["selected_activity"], "main", "{result}");
        assert_eq!(
            auto["available_activities"].as_array().map(Vec::len),
            Some(1),
            "{result}"
        );
    }
}
