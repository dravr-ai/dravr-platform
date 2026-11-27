// ABOUTME: Sleep and recovery analysis tool handlers for MCP protocol
// ABOUTME: Implements 5 tools: analyze_sleep_quality, calculate_recovery_score, suggest_rest_day, track_sleep_trends, optimize_sleep_schedule
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;

use dravr_tronc::dates::Timestamp;
use tracing::{debug, warn};

use super::activity_source::{training_load_activities, training_load_of};
#[cfg(feature = "client-notifications")]
use super::recovery_notice::fire_low_recovery_notification;
#[cfg(feature = "client-notifications")]
use crate::context::scoped_read_subject;
use crate::protocol::sleep_helpers::{
    hrv_trend, latest_sleep_data, no_synced_sleep, sleep_history_data, too_few_synced_nights,
};
use crate::protocol::{UniversalRequest, UniversalResponse, UniversalToolExecutor};
use crate::protocols::ProtocolError;
use dravr_cageux::algorithms::RecoveryAggregationAlgorithm;
use dravr_cageux::config::intelligence::{IntelligenceConfig, SleepScheduleConfig};
use dravr_cageux::recovery_calculator::RecoveryCalculator;
use dravr_cageux::sleep_analysis::{SleepAnalyzer, SleepData};
use dravr_cageux::sleep_guidance::{ScoredNight, SleepNeed};
use pierre_core::uuid_utils::parse_user_id_for_protocol;

use crate::implementations::sleep::output::{
    payload_value, recovery_score_payload, rest_day_payload, sleep_schedule_payload,
    RecoveryTrainingLoad, SleepHighlights, SleepNight, SleepQualityResult, SleepTrendSummary,
    SleepTrendsResult,
};
use crate::protocol::format::{apply_format_typed, extract_output_format};

/// The recovery aggregation: the weighted average on the tenant's
/// recovery-scoring weights, the one aggregation recovery is scored with
/// (carnet#1693).
const fn configured_recovery_algorithm(
    cageux: &IntelligenceConfig<true>,
) -> RecoveryAggregationAlgorithm {
    RecoveryAggregationAlgorithm::from_scoring(&cageux.sleep_recovery.recovery_scoring)
}

/// Nights `track_sleep_trends` reads when the caller names no window.
pub(super) const DEFAULT_TREND_DAYS: u32 = 14;

/// Longest window `track_sleep_trends` reads: a year of nights.
pub(super) const MAX_TREND_DAYS: u32 = 366;

/// Handle `analyze_sleep_quality` tool - analyze the most recent synced night
///
/// Analyzes sleep duration, stages (deep/REM/light), efficiency, and generates quality score.
///
/// Supports two modes:
/// 1. **Synced mode**: the most recent night every connected source synced,
///    merged, or only `sleep_provider`'s when named
/// 2. **Manual mode**: Provide `sleep_data` JSON directly
///
/// # Parameters
/// - `sleep_provider` (optional): Read only this source's synced sleep (e.g., "whoop", "garmin")
/// - `sleep_data` (optional): Manual sleep data JSON (used if `sleep_provider` not specified)
///
/// # Errors
/// Returns `ProtocolError` if sleep data is missing or invalid
// Long function: Protocol handler with async data fetching, HRV analysis, response formatting
pub(crate) fn handle_analyze_sleep_quality(
    executor: &UniversalToolExecutor,
    request: UniversalRequest,
) -> Pin<Box<dyn Future<Output = Result<UniversalResponse, ProtocolError>> + Send + '_>> {
    Box::pin(async move {
        use parse_user_id_for_protocol;

        // Extract output format parameter: "json" (default) or "toon"
        let output_format = extract_output_format(&request);

        // Manual `sleep_data` wins; otherwise the most recent synced night,
        // merged across sources or narrowed to `sleep_provider` when named.
        let source = request
            .parameters
            .get("sleep_provider")
            .and_then(serde_json::Value::as_str);
        let manual = request
            .parameters
            .get("sleep_data")
            .filter(|_| source.is_none());
        let sleep_data: SleepData = if let Some(sleep_data_json) = manual {
            serde_json::from_value(sleep_data_json.clone()).map_err(|e| {
                ProtocolError::InvalidRequest(format!("Invalid sleep_data format: {e}"))
            })?
        } else {
            let user_uuid = parse_user_id_for_protocol(&request.user_id)?;
            match latest_sleep_data(
                executor,
                user_uuid,
                request.tenant_id.as_deref(),
                source,
                1, // Most recent night
            )
            .await
            {
                Ok((data, _sources)) => data,
                Err(response) => return Ok(*response),
            }
        };

        // Get sleep/recovery config
        let cageux_config = executor.cageux_config();
        let config = &cageux_config.sleep_recovery;

        // Calculate sleep quality using foundation module
        let sleep_quality =
            SleepAnalyzer::calculate_sleep_quality(&sleep_data, config).map_err(|e| {
                ProtocolError::InternalError(format!(
                    "sleep_analyzer: Sleep quality calculation failed: {e}"
                ))
            })?;

        // Analyze HRV if available
        let hrv_analysis = hrv_trend(
            executor,
            parse_user_id_for_protocol(&request.user_id).ok(),
            request.tenant_id.as_deref(),
            source,
            &sleep_data,
            config,
        )
        .await
        .map_err(|e| {
            ProtocolError::InternalError(format!("sleep_analyzer: HRV analysis failed: {e}"))
        })?;

        let payload = SleepQualityResult {
            sleep_quality,
            hrv_analysis,
            analysis_date: sleep_data.date,
            provider_score: sleep_data.provider_score,
        };
        let result = UniversalResponse {
            success: true,
            result: None,
            error: None,
            metadata: Some({
                let mut map = HashMap::new();
                map.insert(
                    "analysis_timestamp".into(),
                    serde_json::Value::String(Timestamp::now().to_rfc3339()),
                );
                map.insert(
                    "scientific_references".into(),
                    serde_json::Value::Array(vec![
                        serde_json::Value::String(
                            "Watson et al. (2015) - NSF Sleep Guidelines".into(),
                        ),
                        serde_json::Value::String(
                            "Hirshkowitz et al. (2015) - Sleep Duration Recommendations".into(),
                        ),
                    ]),
                );
                map
            }),
        };

        apply_format_typed(result, payload, output_format)
    })
}

/// Handle `calculate_recovery_score` tool - holistic recovery assessment
///
/// Combines Training Stress Balance (TSB), sleep quality, and HRV into overall recovery score.
///
/// Supports cross-provider integration:
/// - Use `activity_provider` to specify where to fetch training data (e.g., "strava", "garmin")
/// - Sleep is the most recent synced night, merged across sources or only
///   `sleep_provider`'s when named; manual `sleep_data` JSON replaces it
/// - Auto-selects the activity provider if not specified
///
/// # Parameters
/// - `activity_provider` (optional): Provider for activities (default: the athlete's elected provider)
/// - `sleep_provider` (optional): Read only this source's synced sleep (e.g., "whoop", "garmin")
/// - `sleep_data` (optional): Manual sleep data JSON (used if `sleep_provider` not specified)
///
/// # Errors
/// Returns `ProtocolError` if required data is missing or calculation fails
// Long function: Protocol handler inherently long due to async auth, param extraction, calculation, response formatting
pub(crate) fn handle_calculate_recovery_score(
    executor: &UniversalToolExecutor,
    request: UniversalRequest,
) -> Pin<Box<dyn Future<Output = Result<UniversalResponse, ProtocolError>> + Send + '_>> {
    Box::pin(async move {
        use parse_user_id_for_protocol;

        // Extract output format parameter: "json" (default) or "toon"
        let output_format = extract_output_format(&request);

        let user_uuid = parse_user_id_for_protocol(&request.user_id)?;

        // Training load from the athlete's activity provider, elected and
        // fetched the way every activity tool does.
        let (activity_provider, activities) = match training_load_activities(
            executor,
            &request.parameters,
            user_uuid,
            request.tenant_id.as_deref(),
        )
        .await
        {
            Ok(activities) => activities,
            Err(response) => return Ok(*response),
        };

        // CTL, ATL and TSB scored against the athlete's saved thresholds.
        let training_load = training_load_of(
            executor,
            user_uuid,
            request.tenant_id.as_deref(),
            &activities,
        )
        .await?;

        // Sleep is optional here (a TSB-only score stands without it): manual
        // `sleep_data` wins, a named `sleep_provider` must have synced a night,
        // and otherwise the latest night merged across sources is used if any.
        let source = request
            .parameters
            .get("sleep_provider")
            .and_then(serde_json::Value::as_str);
        let sleep_result: Option<(SleepData, Option<String>)> =
            match (source, request.parameters.get("sleep_data")) {
                (None, Some(sleep_data_json)) => {
                    let data = serde_json::from_value(sleep_data_json.clone()).map_err(|e| {
                        ProtocolError::InvalidRequest(format!("Invalid sleep_data format: {e}"))
                    })?;
                    Some((data, None))
                }
                (Some(_), _) => match latest_sleep_data(
                    executor,
                    user_uuid,
                    request.tenant_id.as_deref(),
                    source,
                    1,
                )
                .await
                {
                    Ok((data, sources)) => Some((data, Some(sources.join(" + ")))),
                    Err(response) => return Ok(*response),
                },
                (None, None) => {
                    latest_sleep_data(executor, user_uuid, request.tenant_id.as_deref(), None, 1)
                        .await
                        .ok()
                        .map(|(data, sources)| (data, Some(sources.join(" + "))))
                }
            };

        // Get sleep/recovery config
        let cageux_config = executor.cageux_config();
        let config = &cageux_config.sleep_recovery;

        // Calculate recovery score based on available data
        let (recovery_score, sleep_quality_score, hrv_status, sleep_provider_used) =
            if let Some((sleep_data, sleep_provider)) = sleep_result {
                // Full mode: calculate with sleep data
                let sleep_quality = SleepAnalyzer::calculate_sleep_quality(&sleep_data, config)
                    .map_err(|e| {
                        ProtocolError::InternalError(format!(
                            "sleep_analyzer: Sleep quality calculation failed: {e}"
                        ))
                    })?;

                // Get HRV analysis if available
                let hrv_analysis = hrv_trend(
                    executor,
                    Some(user_uuid),
                    request.tenant_id.as_deref(),
                    source,
                    &sleep_data,
                    config,
                )
                .await
                .map_err(|e| {
                    ProtocolError::InternalError(format!(
                        "sleep_analyzer: HRV analysis failed: {e}"
                    ))
                })?;

                let algorithm = configured_recovery_algorithm(&executor.cageux_config());

                // Calculate holistic recovery score
                let score = RecoveryCalculator::calculate_recovery_score(
                    &training_load,
                    &sleep_quality,
                    hrv_analysis.as_ref(),
                    config,
                    &algorithm,
                )
                .map_err(|e| {
                    ProtocolError::InternalError(format!(
                        "sleep_analyzer: Recovery score calculation failed: {e}"
                    ))
                })?;

                let hrv_status = hrv_analysis.as_ref().map(|h| h.recovery_status);
                (
                    score,
                    Some(sleep_quality.overall_score),
                    hrv_status,
                    sleep_provider,
                )
            } else {
                // TSB-only fallback mode: no sleep data available
                debug!("No sleep data available, using TSB-only recovery calculation");
                let score =
                    RecoveryCalculator::calculate_recovery_score_tsb_only(&training_load, config)
                        .map_err(|e| {
                        ProtocolError::InternalError(format!(
                            "sleep_analyzer: TSB-only recovery score calculation failed: {e}"
                        ))
                    })?;

                (score, None, None, None)
            };

        // Fire low recovery score notification if below threshold. It is the
        // athlete's own: a coach's read of their recovery notifies no one
        // (carnet#1387).
        #[cfg(feature = "client-notifications")]
        if scoped_read_subject().is_none() {
            fire_low_recovery_notification(
                &executor.resources,
                user_uuid,
                request.tenant_id.as_deref(),
                recovery_score.overall_score,
            );
        }

        let payload = recovery_score_payload(
            recovery_score.clone(),
            RecoveryTrainingLoad {
                ctl: training_load.ctl,
                atl: training_load.atl,
                tsb: training_load.tsb,
            },
            sleep_quality_score,
            hrv_status,
            activity_provider.clone(),
            sleep_provider_used.clone(),
        );
        let result = UniversalResponse {
            success: true,
            result: None,
            error: None,
            metadata: Some({
                let mut map = HashMap::new();
                map.insert(
                    "calculation_timestamp".into(),
                    serde_json::Value::String(Timestamp::now().to_rfc3339()),
                );
                map.insert(
                    "components_used".into(),
                    serde_json::Value::Number(serde_json::Number::from(
                        recovery_score.components.components_available,
                    )),
                );
                map.insert(
                    "activity_provider".into(),
                    serde_json::Value::String(activity_provider),
                );
                if let Some(ref provider) = sleep_provider_used {
                    map.insert(
                        "sleep_provider".into(),
                        serde_json::Value::String(provider.clone()),
                    );
                }
                map.insert(
                    "data_completeness".into(),
                    serde_json::to_value(recovery_score.data_completeness)
                        .unwrap_or_else(|_| serde_json::Value::String("unknown".to_owned())),
                );
                map
            }),
        };

        apply_format_typed(result, payload, output_format)
    })
}

/// Handle `suggest_rest_day` tool - AI-powered rest day recommendation
///
/// Analyzes recovery score, training load, and sleep to recommend rest or training.
///
/// Supports cross-provider integration:
/// - Use `activity_provider` to specify where to fetch training data
/// - Sleep is the most recent synced night, merged across sources or only
///   `sleep_provider`'s when named
/// - Auto-selects the activity provider if not specified
///
/// # Parameters
/// - `activity_provider` (optional): Provider for activities (default: the athlete's elected provider)
/// - `sleep_provider` (optional): Read only this source's synced sleep
/// - `sleep_data` (optional): Manual sleep data JSON (used if `sleep_provider` not specified)
///
/// # Errors
/// Returns `ProtocolError` if required data is missing or analysis fails
// Long function: Protocol handler inherently long due to async auth, param extraction, calculation, response formatting
pub(crate) fn handle_suggest_rest_day(
    executor: &UniversalToolExecutor,
    request: UniversalRequest,
) -> Pin<Box<dyn Future<Output = Result<UniversalResponse, ProtocolError>> + Send + '_>> {
    Box::pin(async move {
        use parse_user_id_for_protocol;

        let user_uuid = parse_user_id_for_protocol(&request.user_id)?;

        // Training load from the athlete's activity provider, elected and
        // fetched the way every activity tool does.
        let (_, activities) = match training_load_activities(
            executor,
            &request.parameters,
            user_uuid,
            request.tenant_id.as_deref(),
        )
        .await
        {
            Ok(activities) => activities,
            Err(response) => return Ok(*response),
        };

        // CTL, ATL and TSB scored against the athlete's saved thresholds.
        let training_load = training_load_of(
            executor,
            user_uuid,
            request.tenant_id.as_deref(),
            &activities,
        )
        .await?;

        // Sleep is optional here (TSB-only advice stands without it): manual
        // `sleep_data` wins, a named `sleep_provider` must have synced a night,
        // and otherwise the latest night merged across sources is used if any.
        let source = request
            .parameters
            .get("sleep_provider")
            .and_then(serde_json::Value::as_str);
        let sleep_result: Option<SleepData> = match (source, request.parameters.get("sleep_data")) {
            (None, Some(sleep_data_json)) => Some(
                serde_json::from_value(sleep_data_json.clone()).map_err(|e| {
                    ProtocolError::InvalidRequest(format!("Invalid sleep_data format: {e}"))
                })?,
            ),
            (Some(_), _) => match latest_sleep_data(
                executor,
                user_uuid,
                request.tenant_id.as_deref(),
                source,
                1,
            )
            .await
            {
                Ok((data, _sources)) => Some(data),
                Err(response) => return Ok(*response),
            },
            (None, None) => {
                latest_sleep_data(executor, user_uuid, request.tenant_id.as_deref(), None, 1)
                    .await
                    .ok()
                    .map(|(data, _sources)| data)
            }
        };

        // Get sleep/recovery config
        let cageux_config = executor.cageux_config();
        let config = &cageux_config.sleep_recovery;

        // Calculate recovery score and generate recommendation based on available data
        let (recovery_score, recommendation, sleep_quality_score, sleep_hours, hrv_status) =
            if let Some(sleep_data) = sleep_result {
                // Full mode: calculate with sleep data
                let sleep_quality = SleepAnalyzer::calculate_sleep_quality(&sleep_data, config)
                    .map_err(|e| {
                        ProtocolError::InternalError(format!(
                            "sleep_analyzer: Sleep quality calculation failed: {e}"
                        ))
                    })?;

                // HRV analysis
                let hrv_analysis = hrv_trend(
                    executor,
                    Some(user_uuid),
                    request.tenant_id.as_deref(),
                    source,
                    &sleep_data,
                    config,
                )
                .await
                .map_err(|e| {
                    ProtocolError::InternalError(format!(
                        "sleep_analyzer: HRV analysis failed: {e}"
                    ))
                })?;

                let algorithm = configured_recovery_algorithm(&executor.cageux_config());

                // Calculate recovery score
                let score = RecoveryCalculator::calculate_recovery_score(
                    &training_load,
                    &sleep_quality,
                    hrv_analysis.as_ref(),
                    config,
                    &algorithm,
                )
                .map_err(|e| {
                    ProtocolError::InternalError(format!(
                        "sleep_analyzer: Recovery score calculation failed: {e}"
                    ))
                })?;

                // Generate rest day recommendation
                let rec = RecoveryCalculator::recommend_rest_day(
                    &score,
                    &sleep_data,
                    &training_load,
                    config,
                )
                .map_err(|e| {
                    ProtocolError::InternalError(format!(
                        "sleep_analyzer: Rest day recommendation failed: {e}"
                    ))
                })?;

                let hrv_status = hrv_analysis.as_ref().map(|h| h.recovery_status);
                (
                    score,
                    rec,
                    Some(sleep_quality.overall_score),
                    Some(sleep_data.duration_hours),
                    hrv_status,
                )
            } else {
                // TSB-only fallback mode: no sleep data available
                debug!("No sleep data available, using TSB-only rest day recommendation");
                let score =
                    RecoveryCalculator::calculate_recovery_score_tsb_only(&training_load, config)
                        .map_err(|e| {
                        ProtocolError::InternalError(format!(
                            "sleep_analyzer: TSB-only recovery score calculation failed: {e}"
                        ))
                    })?;

                let rec =
                    RecoveryCalculator::recommend_rest_day_tsb_only(&score, &training_load, config)
                        .map_err(|e| {
                            ProtocolError::InternalError(format!(
                                "sleep_analyzer: TSB-only rest day recommendation failed: {e}"
                            ))
                        })?;

                (score, rec, None, None, None)
            };

        let payload = rest_day_payload(
            recommendation.clone(),
            &recovery_score,
            training_load.tsb,
            sleep_quality_score,
            sleep_hours,
            hrv_status,
        );
        Ok(UniversalResponse {
            success: true,
            result: Some(payload_value("suggest_rest_day", &payload)?),
            error: None,
            metadata: Some({
                let mut map = HashMap::new();
                map.insert(
                    "recommendation_timestamp".into(),
                    serde_json::Value::String(Timestamp::now().to_rfc3339()),
                );
                // Only include confidence if it's a valid f64 value
                if let Some(confidence_number) =
                    serde_json::Number::from_f64(recommendation.confidence)
                {
                    map.insert(
                        "confidence_percent".into(),
                        serde_json::Value::Number(confidence_number),
                    );
                } else {
                    warn!(
                        confidence = recommendation.confidence,
                        "Invalid confidence value (NaN/Infinity), omitting from metadata"
                    );
                }
                map.insert(
                    "data_completeness".into(),
                    serde_json::to_value(recovery_score.data_completeness)
                        .unwrap_or_else(|_| serde_json::Value::String("unknown".to_owned())),
                );
                map
            }),
        })
    })
}

/// Handle `track_sleep_trends` tool - analyze sleep patterns over time
///
/// Correlates sleep quality with performance and training load.
///
/// Supports two modes:
/// 1. **Synced mode**: the last `days` nights every connected source synced,
///    merged per night, or only `sleep_provider`'s when named
/// 2. **Manual mode**: Provide `sleep_history` JSON array directly
///
/// # Parameters
/// - `sleep_provider` (optional): Read only this source's synced sleep history
/// - `days` (optional): Number of days of history to fetch (default: 14)
/// - `sleep_history` (optional): Manual sleep history JSON array
///
/// # Errors
/// Returns `ProtocolError` if data is insufficient or analysis fails
// Long function: Protocol handler inherently long due to trend calculation, statistics aggregation, response formatting
pub(crate) fn handle_track_sleep_trends(
    executor: &UniversalToolExecutor,
    request: UniversalRequest,
) -> Pin<Box<dyn Future<Output = Result<UniversalResponse, ProtocolError>> + Send + '_>> {
    Box::pin(async move {
        use parse_user_id_for_protocol;

        // Extract output format parameter: "json" (default) or "toon"
        let output_format = extract_output_format(&request);

        // Manual `sleep_history` wins; otherwise the synced nights of the last
        // `days` days, merged across sources or narrowed to `sleep_provider`.
        let source = request
            .parameters
            .get("sleep_provider")
            .and_then(serde_json::Value::as_str);
        let manual = request
            .parameters
            .get("sleep_history")
            .filter(|_| source.is_none());
        // The synced window, `None` when the nights came by hand.
        let mut synced_days = None;
        let sleep_history: Vec<SleepData> = if let Some(sleep_history_json) = manual {
            serde_json::from_value(sleep_history_json.clone()).map_err(|e| {
                ProtocolError::InvalidRequest(format!("Invalid sleep_history format: {e}"))
            })?
        } else {
            let user_uuid = parse_user_id_for_protocol(&request.user_id)?;
            let days = request
                .parameters
                .get("days")
                .and_then(serde_json::Value::as_u64)
                .map_or(DEFAULT_TREND_DAYS, |d| {
                    u32::try_from(d.clamp(1, u64::from(MAX_TREND_DAYS))).unwrap_or(MAX_TREND_DAYS)
                });
            synced_days = Some(days);
            match sleep_history_data(
                executor,
                user_uuid,
                request.tenant_id.as_deref(),
                source,
                days,
            )
            .await
            {
                // Oldest first, so the trend reads forward in time.
                Ok(nights) => nights.into_iter().rev().map(|(data, _)| data).collect(),
                Err(response) => return Ok(*response),
            }
        };

        // The trend, its window and its thresholds are cageux's (carnet#1803):
        // a window is also the fewest nights a trend is read from.
        let cageux_config = executor.cageux_config();
        let config = &cageux_config.sleep_recovery;
        let trend_min_days = config.sleep_trend.window_nights;
        if sleep_history.len() < trend_min_days {
            // Synced nights that fall short are what the athlete has, not a
            // mistake in the call: no night at all answers as every sleep
            // tool does, too few say how many. Nights passed by hand are the
            // caller's input, and too few of them is a parameter error.
            return match synced_days {
                Some(days) if sleep_history.is_empty() => {
                    Ok(no_synced_sleep(source, days, "sleep_history"))
                }
                Some(days) => Ok(too_few_synced_nights(
                    sleep_history.len(),
                    days,
                    trend_min_days,
                )),
                None => Err(ProtocolError::InvalidRequest(format!(
                    "At least {trend_min_days} days of sleep data required for trend analysis"
                ))),
            };
        }

        // Calculate average sleep metrics
        #[allow(clippy::cast_precision_loss)]
        // Safe: sleep_history.len() validated >= 7, well within f64 precision
        let avg_duration = sleep_history.iter().map(|s| s.duration_hours).sum::<f64>()
            / sleep_history.len() as f64;

        #[allow(clippy::cast_precision_loss)]
        // Safe: count of sleep records with efficiency, small number, well within f64 precision
        let efficiency_count = sleep_history
            .iter()
            .filter(|s| s.efficiency_percent.is_some())
            .count();
        let avg_efficiency = if efficiency_count > 0 {
            sleep_history
                .iter()
                .filter_map(|s| s.efficiency_percent)
                .sum::<f64>()
                / efficiency_count as f64
        } else {
            0.0
        };

        // Each night's quality, then cageux's trend across them: the latest
        // window's mean against the window before it.
        let nights: Vec<ScoredNight> = sleep_history
            .iter()
            .filter_map(|sleep| {
                SleepAnalyzer::calculate_sleep_quality(sleep, config)
                    .ok()
                    .map(|quality| ScoredNight {
                        date: sleep.date,
                        score: quality.overall_score,
                    })
            })
            .collect();
        let quality_trend =
            SleepAnalyzer::quality_trend(&nights, &config.sleep_trend).map_err(|e| {
                ProtocolError::InvalidRequest(format!(
                    "No night's sleep quality could be scored for trend analysis: {e}"
                ))
            })?;
        let trend = quality_trend.direction.as_str();

        // Generate insights
        let mut insights = Vec::new();
        insights.push(format!(
            "Average sleep duration: {avg_duration:.1}h over {} days",
            sleep_history.len()
        ));
        insights.push(format!("Average sleep efficiency: {avg_efficiency:.1}%"));
        insights.push(format!("Sleep quality trend: {trend}"));

        let athlete_min_hours = config.sleep_duration.athlete_min_hours;
        if avg_duration < athlete_min_hours {
            insights.push(format!(
                "Sleep duration below athlete recommendation ({avg_duration:.1}h < {athlete_min_hours:.1}h)"
            ));
        }

        let payload = SleepTrendsResult {
            trends: SleepTrendSummary {
                average_duration_hours: avg_duration,
                average_efficiency_percent: avg_efficiency,
                quality_trend: trend.to_owned(),
                recent_7day_avg: quality_trend.recent_average,
                previous_7day_avg: quality_trend.previous_average,
            },
            highlights: SleepHighlights {
                best_night: quality_trend.best_night.map(|night| SleepNight {
                    date: night.date,
                    score: night.score,
                }),
                worst_night: quality_trend.worst_night.map(|night| SleepNight {
                    date: night.date,
                    score: night.score,
                }),
            },
            insights,
            data_points: nights.len(),
        };
        let result = UniversalResponse {
            success: true,
            result: None,
            error: None,
            metadata: Some({
                let mut map = HashMap::new();
                map.insert(
                    "analysis_timestamp".into(),
                    serde_json::Value::String(Timestamp::now().to_rfc3339()),
                );
                map.insert(
                    "days_analyzed".into(),
                    serde_json::Value::Number(serde_json::Number::from(sleep_history.len())),
                );
                map
            }),
        };

        apply_format_typed(result, payload, output_format)
    })
}

/// Handle `optimize_sleep_schedule` tool - recommend optimal sleep based on training
///
/// Suggests sleep duration and timing based on upcoming workouts and recovery needs.
///
/// Supports cross-provider integration:
/// - Use `activity_provider` to specify where to fetch training load data
/// - Auto-selects best available provider if not specified
///
/// # Parameters
/// - `activity_provider` (optional): Provider for activities (default: the athlete's elected provider)
/// - `upcoming_workout_intensity` (optional): "low", "moderate", or "high"
/// - `typical_wake_time` (optional): Wake time in "HH:MM" format (default: "06:00")
///
/// # Errors
/// Returns `ProtocolError` if required parameters are missing
// Long function: Protocol handler inherently long due to async auth, param extraction, calculation, response formatting
pub(crate) fn handle_optimize_sleep_schedule(
    executor: &UniversalToolExecutor,
    request: UniversalRequest,
) -> Pin<Box<dyn Future<Output = Result<UniversalResponse, ProtocolError>> + Send + '_>> {
    Box::pin(async move {
        use parse_user_id_for_protocol;

        let user_uuid = parse_user_id_for_protocol(&request.user_id)?;

        // Training load from the athlete's activity provider, elected and
        // fetched the way every activity tool does.
        let (_, activities) = match training_load_activities(
            executor,
            &request.parameters,
            user_uuid,
            request.tenant_id.as_deref(),
        )
        .await
        {
            Ok(activities) => activities,
            Err(response) => return Ok(*response),
        };

        // CTL, ATL and TSB scored against the athlete's saved thresholds.
        let training_load = training_load_of(
            executor,
            user_uuid,
            request.tenant_id.as_deref(),
            &activities,
        )
        .await?;

        // Get sleep/recovery config
        let cageux_config = executor.cageux_config();
        let config = &cageux_config.sleep_recovery;

        // Get upcoming workout info (optional)
        let upcoming_workout_intensity = request
            .parameters
            .get("upcoming_workout_intensity")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("moderate");

        // cageux's sleep target for this load (carnet#1803): the athlete
        // optimum, plus its fatigue bonus when form is in the deep-fatigue or
        // heavy-block band (below -20% of CTL), else its high-load bonus when
        // ATL is high.
        let target = SleepAnalyzer::sleep_target(&training_load, config);
        let recommended_hours = target.hours;
        let carrying_fatigue = target.need == SleepNeed::CarryingFatigue;

        // Calculate recommended sleep window
        let wake_time = request
            .parameters
            .get("typical_wake_time")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("06:00");

        let bedtime = bedtime(wake_time, recommended_hours, &config.sleep_schedule);

        // Generate recommendations
        let mut recommendations = Vec::new();
        recommendations.push(format!(
            "Target {recommended_hours:.1} hours of sleep tonight"
        ));
        recommendations.push(format!("Recommended bedtime: {bedtime}"));

        if carrying_fatigue {
            recommendations.push("Extra sleep needed - form is below -20% of fitness".to_owned());
        }

        if upcoming_workout_intensity == "high" {
            recommendations.push(
                "High-intensity workout planned - prioritize sleep quality tonight".to_owned(),
            );
        }

        let payload = sleep_schedule_payload(
            recommended_hours,
            bedtime,
            wake_time,
            RecoveryTrainingLoad {
                ctl: training_load.ctl,
                atl: training_load.atl,
                tsb: training_load.tsb,
            },
            upcoming_workout_intensity,
            recommendations,
        );
        Ok(UniversalResponse {
            success: true,
            result: Some(payload_value("optimize_sleep_schedule", &payload)?),
            error: None,
            metadata: Some({
                let mut map = HashMap::new();
                map.insert(
                    "calculation_timestamp".into(),
                    serde_json::Value::String(Timestamp::now().to_rfc3339()),
                );
                map
            }),
        })
    })
}

/// The wake time used when one is not given as `"HH:MM"`: 06:00, in minutes
/// after midnight
const DEFAULT_WAKE_MINUTES: u32 = 6 * 60;

/// Parse hour component from wake time string
fn parse_hour(hour_str: &str) -> u32 {
    match hour_str.parse() {
        Ok(h) if (0..24).contains(&h) => h,
        Ok(h) => {
            warn!(hour = h, "Invalid hour value, using default 6");
            6
        }
        Err(e) => {
            warn!(
                hour_str = hour_str,
                error = %e,
                "Failed to parse hour, using default 6"
            );
            6
        }
    }
}

/// Parse minute component from wake time string
fn parse_minute(minute_str: &str) -> u32 {
    match minute_str.parse() {
        Ok(m) if (0..60).contains(&m) => m,
        Ok(m) => {
            warn!(minute = m, "Invalid minute value, using default 0");
            0
        }
        Err(e) => {
            warn!(
                minute_str = minute_str,
                error = %e,
                "Failed to parse minute, using default 0"
            );
            0
        }
    }
}

/// The wake time `"HH:MM"` in minutes after midnight: 06:00 when it is not
/// that shape, and an hour or minute out of range reads as 6 or 0
fn wake_minutes(wake_time: &str) -> u32 {
    let parts: Vec<&str> = wake_time.split(':').collect();
    let [hour, minute] = parts.as_slice() else {
        warn!(
            wake_time = wake_time,
            "Invalid wake_time format (expected HH:MM), using default 06:00"
        );
        return DEFAULT_WAKE_MINUTES;
    };
    parse_hour(hour) * 60 + parse_minute(minute)
}

/// The bedtime, `"HH:MM"`, that gives `sleep_hours` and cageux's wind-down
/// before `wake_time` ([`SleepAnalyzer::bedtime_minutes`])
fn bedtime(wake_time: &str, sleep_hours: f64, schedule: &SleepScheduleConfig) -> String {
    let minutes = SleepAnalyzer::bedtime_minutes(wake_minutes(wake_time), sleep_hours, schedule);
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dravr_cageux::config::intelligence::SleepRecoveryConfig;

    fn schedule() -> SleepScheduleConfig {
        SleepRecoveryConfig::default().sleep_schedule
    }

    #[test]
    fn the_bedtime_leaves_the_sleep_and_the_wind_down_before_waking() {
        // 06:00 less 8 hours and cageux's 15-minute wind-down: the evening before.
        assert_eq!(
            bedtime("06:00", 8.0, &schedule()),
            "21:45",
            "bug: the bedtime does not leave 8 h and the wind-down before 06:00"
        );
        // Just before midnight: 08:30 less 8.5 h and 15 min.
        assert_eq!(
            bedtime("08:30", 8.5, &schedule()),
            "23:45",
            "bug: the bedtime does not leave 8.5 h and the wind-down before 08:30"
        );
        // After midnight, the hour is written with two digits: 10:00 less 8 h
        // and 15 min.
        assert_eq!(
            bedtime("10:00", 8.0, &schedule()),
            "01:45",
            "bug: a bedtime after midnight is not written as HH:MM"
        );
    }

    #[test]
    fn a_wake_time_not_given_as_hours_and_minutes_reads_as_six() {
        assert_eq!(
            wake_minutes("6am"),
            DEFAULT_WAKE_MINUTES,
            "bug: a wake time without a colon does not read as 06:00"
        );
        assert_eq!(
            wake_minutes("25:00"),
            DEFAULT_WAKE_MINUTES,
            "bug: an hour past 23 does not read as 6"
        );
        assert_eq!(
            bedtime("6am", 8.0, &schedule()),
            "21:45",
            "bug: the bedtime for an unreadable wake time is not counted back from 06:00"
        );
    }
}
