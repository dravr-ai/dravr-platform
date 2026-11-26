// ABOUTME: Handler for the calculate_fitness_score tool: dravr-cageux's fitness score over a window
// ABOUTME: Aerobic fitness, strength endurance and consistency on the athlete's zones, trended against the window before
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::trends::trend_name;
#[cfg(feature = "client-notifications")]
use crate::context::scoped_conversation_id;
#[cfg(feature = "client-notifications")]
use crate::context::scoped_read_subject;
use crate::implementations::analytics::output::{
    FitnessComponents, FitnessInterpretation, FitnessLoadMetrics, FitnessScoreDetail,
    FitnessScoreResult, NoFitnessScore, ProvidersUsed,
};
use crate::implementations::stored_copy::{activities_or_stored, with_caveat, Reader, Served};
use crate::implementations::stored_physiology::stored_profile;
use crate::protocol::format::{apply_format_typed, extract_output_format};
use crate::protocol::provider_helpers::resolve_provider_for_request;
use crate::protocol::{UniversalRequest, UniversalResponse, UniversalToolExecutor};
use crate::protocols::ProtocolError;
#[cfg(feature = "client-notifications")]
use crate::runtime::ToolRuntime;
use dravr_cageux::config::intelligence::IntelligenceConfig;
use dravr_cageux::performance_analyzer::AdvancedPerformanceAnalyzer;
use dravr_cageux::physiological_constants::api_limits::DEFAULT_ACTIVITY_LIMIT;
use dravr_cageux::training_load::TrainingLoadCalculator;
use dravr_cageux::zones::HrZones;
use dravr_tronc::dates::{Elapsed, Timestamp, Zone};
use pierre_core::models::zones::{athlete_hr_zones, HrZonesUnavailable};
use pierre_core::models::{Activity, FormReading};
use pierre_core::uuid_utils::parse_user_id_for_protocol;
use pierre_fitness_compute::AthleteInputs;
#[cfg(feature = "client-notifications")]
use pierre_notifications::triggers as notification_triggers;
#[cfg(feature = "client-notifications")]
use pierre_notifications::TenantId;
use pierre_providers::deduplication::{merge_duplicates, DedupConfig};
use pierre_services::athlete_clock::athlete_zone;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
#[cfg(feature = "client-notifications")]
use std::sync::Arc;
use tracing::warn;
#[cfg(feature = "client-notifications")]
use uuid::Uuid;

/// Score fitness with dravr-cageux's [`AdvancedPerformanceAnalyzer`] over the
/// `timeframe` window, with its trend against the window of equal length
/// before it (carnet#1807).
///
/// The aerobic and strength-endurance components read the athlete's
/// heart-rate zones, `hr_zones`; without them there is no score, and the
/// answer says why rather than guessing one. The training load reported
/// beside the score is read over the full history, each session scored
/// against `athlete`'s saved thresholds, as of the athlete's today in `zone`.
pub async fn calculate_fitness_metrics(
    activities: &[Activity],
    timeframe: &str,
    athlete: &AthleteInputs,
    hr_zones: Result<HrZones, HrZonesUnavailable>,
    cageux_config: &IntelligenceConfig<true>,
    zone: &Zone,
    providers_used: ProvidersUsed,
) -> FitnessScoreResult {
    use dravr_cageux::performance_analyzer::PerformanceAnalyzerTrait;

    let now = Timestamp::now();
    let window_days = window_days_for(timeframe);
    // Providers return newest-first; the training load wants time order.
    let mut chronological = activities.to_vec();
    chronological.sort_by_key(Activity::start_date);
    let activities_analyzed = window_days.map_or(chronological.len(), |days| {
        let cutoff = now - Elapsed::days(i64::from(days));
        chronological
            .iter()
            .filter(|a| a.start_date() >= cutoff)
            .count()
    });
    let unscored = Unscored {
        timeframe,
        window_days,
        history_span_days: history_span_days(&chronological, now),
        activities_analyzed,
        providers_used,
    };

    if chronological.is_empty() {
        return unscored.because("No activities found for fitness calculation".to_owned());
    }
    if activities_analyzed == 0 {
        return unscored.because(window_days.map_or_else(
            || "No activities found in the fetched history".to_owned(),
            |days| format!("No activities found in the last {days} days"),
        ));
    }
    let zones = match hr_zones {
        Ok(zones) => zones,
        Err(missing) => {
            return unscored.because(format!("No fitness score: {}", missing.reason()));
        }
    };

    let analyzer = AdvancedPerformanceAnalyzer::new(cageux_config).with_hr_zones(zones);
    let analyzer = match window_days {
        Some(days) => analyzer.with_window_days(days),
        None => analyzer.with_whole_history(),
    };
    let fitness = match analyzer.calculate_fitness_score(&chronological).await {
        Ok(fitness) => fitness,
        Err(e) => return unscored.because(format!("The fitness score could not be computed: {e}")),
    };
    // With the zones set, cageux gives every component.
    let (Some(overall), Some(aerobic), Some(strength)) = (
        fitness.overall_score,
        fitness.aerobic_fitness,
        fitness.strength_endurance,
    ) else {
        return unscored
            .because("The fitness score could not be computed from the zones".to_owned());
    };

    // The training load beside the score, over the FULL fetched history: the
    // EMA is seeded at the mean daily TSS of the span it is given, so a
    // truncated input reads its own opening days as the athlete's prior load
    // (registre#415).
    let calculator =
        TrainingLoadCalculator::as_of_today(cageux_config.algorithms.clone(), zone.clone(), now);
    let training_load = athlete.training_load(calculator, &chronological).ok();

    FitnessScoreResult::Scored(Box::new(FitnessScoreDetail {
        timeframe: timeframe.to_owned(),
        window_days,
        history_span_days: unscored.history_span_days,
        fitness_score: whole_points(overall),
        previous_fitness_score: fitness.previous_overall_score.map(whole_points),
        trend: fitness.trend.map(|trend| trend_name(trend).to_owned()),
        components: FitnessComponents {
            aerobic_fitness: aerobic.round(),
            strength_endurance: strength.round(),
            consistency: fitness.consistency.round(),
        },
        metrics: FitnessLoadMetrics {
            ctl: training_load.as_ref().map_or(0.0, |l| l.ctl).round(),
            atl: training_load.as_ref().map_or(0.0, |l| l.atl).round(),
            tsb: training_load.as_ref().map_or(0.0, |l| l.tsb).round(),
        },
        activities_analyzed,
        interpretation: FitnessInterpretation {
            aerobic_fitness: "Aerobic work: sessions above zone 1, by how far their heart rate \
                              sat above it and for how long"
                .to_owned(),
            strength_endurance: "Hard work: sessions in zone 3 and above, by heart rate and \
                                 duration"
                .to_owned(),
            consistency: "Sessions per week against a weekly target".to_owned(),
            ctl: FormReading::ctl_definition(
                cageux_config.algorithms.params.training_load_ctl_days,
            ),
        },
        providers_used: unscored.providers_used,
    }))
}

/// What an answer without a score carries besides its reason.
struct Unscored<'call> {
    timeframe: &'call str,
    window_days: Option<u32>,
    history_span_days: u32,
    activities_analyzed: usize,
    providers_used: ProvidersUsed,
}

impl Unscored<'_> {
    /// No score, and why.
    fn because(self, message: String) -> FitnessScoreResult {
        FitnessScoreResult::NoData(NoFitnessScore {
            timeframe: self.timeframe.to_owned(),
            window_days: self.window_days,
            history_span_days: self.history_span_days,
            message,
            activities_analyzed: self.activities_analyzed,
            providers_used: self.providers_used,
        })
    }
}

/// A 0-100 score rounded to whole points.
fn whole_points(score: f64) -> i32 {
    // Safe: cageux bounds the score to 0-100.
    #[allow(clippy::cast_possible_truncation)]
    let points = score.round() as i32;
    points
}

/// Days the scoring window covers, or `None` for "every activity fetched".
///
/// The vocabulary matches `analyze_performance_trends`, which already ships it
/// and is the one windowed analytics tool whose schema and handler agree.
const fn window_days_for(timeframe: &str) -> Option<u32> {
    match timeframe.as_bytes() {
        b"quarter" => Some(90),
        b"year" => Some(365),
        b"all_time" => None,
        // Every other value, including an absent one, is the documented default.
        _ => Some(30),
    }
}

/// How many days of history were fetched.
///
/// Reported alongside the window so a reply states the period it analysed
/// instead of inferring one from a smoothing constant that happened to sit
/// nearby in the payload (registre#415).
fn history_span_days(chronological: &[Activity], now: Timestamp) -> u32 {
    chronological.first().map_or(0, |oldest| {
        u32::try_from((now - oldest.start_date()).num_days().max(0)).unwrap_or(u32::MAX)
    })
}

/// Handle `calculate_fitness_score` tool - dravr-cageux's fitness score
///
/// # Parameters
/// - `provider` (optional): Activity provider (default: configured default)
/// - `timeframe` (optional): `month` (default), `quarter`, `year`, or `all_time`
pub(crate) fn handle_calculate_fitness_score(
    executor: &UniversalToolExecutor,
    request: UniversalRequest,
) -> Pin<Box<dyn Future<Output = Result<UniversalResponse, ProtocolError>> + Send + '_>> {
    Box::pin(async move {
        use parse_user_id_for_protocol;
        use DEFAULT_ACTIVITY_LIMIT;

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
        let timeframe = request
            .parameters
            .get("timeframe")
            .and_then(|v| v.as_str())
            .unwrap_or("month");

        // Extract output format parameter: "json" (default) or "toon"
        let output_format = extract_output_format(&request);

        // The saved physiology: its thresholds score the sessions for the
        // training load, and its resting and maximum heart rates cut the
        // zones the score reads.
        let profile = stored_profile(&executor.resources, request.tenant_id.as_deref(), user_uuid)
            .await
            .map_err(|e| {
                ProtocolError::InternalError(format!(
                    "calculate_fitness_score: reading the saved physiology failed: {e}"
                ))
            })?;
        let athlete = AthleteInputs::from_profile(profile.as_ref());
        let hr_zones = athlete_hr_zones(profile.as_ref());

        match executor
            .auth_service
            .create_authenticated_provider(&provider_name, user_uuid, request.tenant_id.as_deref())
            .await
        {
            Ok(provider) => {
                match activities_or_stored(
                    Reader::new(&executor.resources, user_uuid, request.tenant_id.as_deref()),
                    provider.as_ref(),
                    DEFAULT_ACTIVITY_LIMIT,
                )
                .await
                {
                    Ok(Served {
                        value: raw_activities,
                        caveat,
                    }) => {
                        // Collapse fragments before scoring so CTL/ATL and the
                        // consistency component aren't inflated by re-uploads
                        // or auto-split GPS recordings of the same workout.
                        let (activities, _fragment_report) =
                            merge_duplicates(raw_activities, &DedupConfig::default());

                        let analysis = calculate_fitness_metrics(
                            &activities,
                            timeframe,
                            &athlete,
                            hr_zones,
                            &executor.cageux_config(),
                            &athlete_zone(executor.resources.repos(), user_uuid).await,
                            ProvidersUsed {
                                activity_provider: provider_name.clone(),
                                sleep_provider: None,
                            },
                        )
                        .await;

                        // Fire fitness improvement notification if trend is
                        // improving. It is the athlete's own: a coach's read of
                        // them notifies no one (carnet#1387).
                        #[cfg(feature = "client-notifications")]
                        if scoped_read_subject().is_none() {
                            fire_fitness_improvement_notification(
                                &executor.resources,
                                user_uuid,
                                request.tenant_id.as_deref(),
                                &analysis,
                            );
                        }

                        let result = UniversalResponse {
                            success: true,
                            result: None,
                            error: None,
                            metadata: Some({
                                let mut map = HashMap::new();
                                map.insert(
                                    "user_id".to_owned(),
                                    serde_json::Value::String(user_uuid.to_string()),
                                );
                                map.insert(
                                    "activity_provider".to_owned(),
                                    serde_json::Value::String(provider_name),
                                );
                                map
                            }),
                        };

                        // Apply format transformation
                        with_caveat(apply_format_typed(result, analysis, output_format), caveat)
                    }
                    Err(e) => {
                        warn!(error = %e, "activity read for fitness score failed");
                        Ok(UniversalResponse {
                            success: false,
                            result: None,
                            error: Some(format!(
                                "Failed to fetch activities: {}",
                                e.tool_message()
                            )),
                            metadata: None,
                        })
                    }
                }
            }
            Err(response) => Ok(*response),
        }
    })
}

/// Fire a fitness improvement notification when the trend is "improving"
/// and the fitness score exceeds a meaningful threshold.
#[cfg(feature = "client-notifications")]
fn fire_fitness_improvement_notification(
    resources: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant_id_str: Option<&str>,
    analysis: &FitnessScoreResult,
) {
    let Some(service) = &resources.notification_service() else {
        return;
    };
    let Some(tenant_str) = tenant_id_str else {
        return;
    };
    let Ok(tenant_uuid) = tenant_str.parse::<Uuid>() else {
        return;
    };
    let tenant_id = TenantId(tenant_uuid);

    // Only the scored arm can be improving; the empty answers carry no trend.
    let FitnessScoreResult::Scored(detail) = analysis else {
        return;
    };
    let (trend, score) = (detail.trend.as_deref(), detail.fitness_score);

    // Only trigger when fitness trend is improving and score is meaningful
    if trend == Some("improving") && score > 0 {
        notification_triggers::trigger_fitness_improvement(
            service,
            user_id,
            tenant_id,
            "Fitness Score",
            &format!("{score}"),
            scoped_conversation_id().as_deref(),
        );
    }
}
