// ABOUTME: Handler for detect_patterns tool identifying training patterns in activity data
// ABOUTME: Detects weekly schedules, training blocks, volume progression, and overtraining signals
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::implementations::analytics::output::{
    DayFrequency, HardEasyPatternResult, InsufficientPatternData, IntensityDistribution,
    OvertrainingResult, PatternsResult, VolumeProgressionResult, WeeklySchedulePatternResult,
};
use crate::protocol::format::{apply_format_typed, extract_output_format};
use crate::protocol::provider_helpers::resolve_provider_for_request;
use crate::protocol::{UniversalRequest, UniversalResponse, UniversalToolExecutor};
use crate::protocols::ProtocolError;
use pierre_core::civil_time::resolve_zone;
use pierre_core::models::Activity;
use pierre_core::uuid_utils::parse_user_id_for_protocol;
use pierre_formatters::OutputFormat;
use pierre_intelligence::physiological_constants::api_limits::DEFAULT_ACTIVITY_LIMIT;
use pierre_intelligence::{
    HardEasyPattern, OvertrainingSignals, PatternDetector, RiskLevel, VolumeProgressionPattern,
    VolumeTrend, WeeklySchedulePattern,
};
use pierre_providers::core::FitnessProvider;
use pierre_providers::deduplication::{dedupe_and_report, DedupConfig};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;

/// Fetch activities and detect patterns
///
/// Retrieves recent activities from provider and performs pattern detection
/// based on the specified pattern type.
///
/// # Arguments
/// * `provider` - Configured fitness provider
/// * `pattern_type` - Type of pattern to detect (e.g., "`weekly_schedule`", "overtraining")
/// * `user_uuid` - User UUID for response metadata
///
/// # Returns
/// `UniversalResponse` with pattern analysis or error
async fn fetch_and_detect_patterns(
    provider: Box<dyn FitnessProvider>,
    pattern_type: &str,
    user_uuid: uuid::Uuid,
    user_timezone: Option<&str>,
    output_format: OutputFormat,
) -> Result<UniversalResponse, ProtocolError> {
    use DEFAULT_ACTIVITY_LIMIT;

    match provider
        .get_activities(Some(DEFAULT_ACTIVITY_LIMIT), None)
        .await
    {
        Ok(raw_activities) => {
            // Collapse fragments so the overtraining detector doesn't trip on
            // synthetic activity-density signals from re-uploaded GPS files.
            let (activities, _fragment_report) =
                dedupe_and_report(&raw_activities, &DedupConfig::default());
            let analysis = detect_activity_patterns(&activities, pattern_type, user_timezone);

            apply_format_typed(
                UniversalResponse {
                    success: true,
                    result: None,
                    error: None,
                    metadata: Some({
                        let mut map = HashMap::new();
                        map.insert(
                            "user_id".to_owned(),
                            serde_json::Value::String(user_uuid.to_string()),
                        );
                        map
                    }),
                },
                analysis,
                output_format,
            )
        }
        Err(e) => Ok(UniversalResponse {
            success: false,
            result: None,
            error: Some(format!("Failed to fetch activities: {e}")),
            metadata: None,
        }),
    }
}

/// Detect patterns in activity data based on pattern type
fn detect_activity_patterns(
    activities: &[Activity],
    pattern_type: &str,
    user_timezone: Option<&str>,
) -> PatternsResult {
    use PatternDetector;

    if activities.len() < 3 {
        return PatternsResult::Insufficient(InsufficientPatternData {
            pattern_type: pattern_type.to_owned(),
            activities_analyzed: activities.len(),
            patterns_detected: vec![],
            insights: vec!["Need at least 3 activities for pattern detection".to_owned()],
            confidence: "insufficient_data".to_owned(),
        });
    }

    match pattern_type {
        "training_blocks" => {
            format_hard_easy_pattern(&PatternDetector::detect_hard_easy_pattern(activities))
        }
        "progression" => {
            format_volume_progression(&PatternDetector::detect_volume_progression(activities))
        }
        "overtraining" => {
            format_overtraining_signals(&PatternDetector::detect_overtraining_signals(activities))
        }
        // The weekday and hour histograms are counted on the athlete's civil
        // clock. Read in UTC, a 21:00 America/Toronto session was counted on
        // the following weekday and reported as a 01:00 habit (registre#252).
        _ => format_weekly_schedule(&PatternDetector::detect_weekly_schedule(
            activities,
            resolve_zone(user_timezone),
        )), // default: weekly_schedule
    }
}

/// Format weekly schedule pattern results for JSON response
fn format_weekly_schedule(pattern: &WeeklySchedulePattern) -> PatternsResult {
    use chrono::Weekday;

    // Convert Weekday enum to string
    let day_to_string = |weekday: &Weekday| -> &str {
        match weekday {
            Weekday::Mon => "Monday",
            Weekday::Tue => "Tuesday",
            Weekday::Wed => "Wednesday",
            Weekday::Thu => "Thursday",
            Weekday::Fri => "Friday",
            Weekday::Sat => "Saturday",
            Weekday::Sun => "Sunday",
        }
    };

    // Build preferred days list with frequencies
    let preferred_days: Vec<DayFrequency> = pattern
        .day_frequencies
        .iter()
        .map(|(day, &count)| DayFrequency {
            day: day.clone(),
            frequency: count,
        })
        .collect();

    // Generate pattern descriptions
    let mut patterns = Vec::new();
    if pattern.consistency_score > 30.0 {
        patterns.push(format!(
            "Consistent weekly schedule detected: primarily trains on {}",
            pattern
                .most_common_days
                .iter()
                .map(day_to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    // Determine confidence based on consistency score
    let confidence = if pattern.consistency_score > 40.0 {
        "high"
    } else if pattern.consistency_score > 20.0 {
        "medium"
    } else {
        "low"
    };

    PatternsResult::WeeklySchedule(Box::new(WeeklySchedulePatternResult {
        pattern_type: "weekly_schedule".to_owned(),
        preferred_training_days: preferred_days,
        insights: if patterns.is_empty() {
            vec!["No strong weekly schedule pattern detected - training is variable".to_owned()]
        } else {
            patterns.clone()
        },
        patterns_detected: patterns,
        consistency_score: pattern.consistency_score,
        avg_activities_per_week: pattern.avg_activities_per_week,
        confidence: confidence.to_owned(),
    }))
}

/// Format hard/easy pattern results for JSON response
fn format_hard_easy_pattern(pattern: &HardEasyPattern) -> PatternsResult {
    let mut insights = vec![pattern.pattern_description.clone()];

    if !pattern.adequate_recovery {
        insights.push("Consider adding more recovery days between hard efforts".to_owned());
    }

    let confidence = if pattern.pattern_detected {
        "medium"
    } else {
        "low"
    };

    PatternsResult::HardEasy(Box::new(HardEasyPatternResult {
        pattern_type: "training_blocks".to_owned(),
        pattern_detected: pattern.pattern_detected,
        intensity_distribution: IntensityDistribution {
            hard_percentage: pattern.hard_percentage,
            easy_percentage: pattern.easy_percentage,
        },
        adequate_recovery: pattern.adequate_recovery,
        patterns_detected: if pattern.pattern_detected {
            vec![pattern.pattern_description.clone()]
        } else {
            Vec::new()
        },
        insights,
        confidence: confidence.to_owned(),
    }))
}

/// Format volume progression pattern results for JSON response
fn format_volume_progression(pattern: &VolumeProgressionPattern) -> PatternsResult {
    use VolumeTrend;

    let mut insights = Vec::new();
    let trend_description = match pattern.trend {
        VolumeTrend::Increasing => {
            insights.push("Volume is increasing - progressive overload detected".to_owned());
            "increasing"
        }
        VolumeTrend::Decreasing => {
            insights.push("Volume is decreasing - taper or recovery phase".to_owned());
            "decreasing"
        }
        VolumeTrend::Stable => {
            insights.push("Volume is stable - maintaining consistent training load".to_owned());
            "stable"
        }
    };

    if pattern.volume_spikes_detected {
        insights.push(format!(
            "Volume spikes detected in weeks: {} - sharp jumps above your recent baseline; build in recovery before adding more",
            pattern
                .spike_weeks
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    PatternsResult::VolumeProgression(Box::new(VolumeProgressionResult {
        pattern_type: "progression".to_owned(),
        trend: trend_description.to_owned(),
        weekly_volumes: pattern.weekly_volumes.clone(),
        week_numbers: pattern.week_numbers.clone(),
        volume_spikes_detected: pattern.volume_spikes_detected,
        spike_weeks: pattern.spike_weeks.clone(),
        patterns_detected: insights.clone(),
        insights,
        confidence: "medium".to_owned(),
    }))
}

/// Format overtraining signals results for JSON response
fn format_overtraining_signals(signals: &OvertrainingSignals) -> PatternsResult {
    use RiskLevel;

    let mut warning_signs = Vec::new();

    if signals.hr_drift_detected {
        if let Some(drift_pct) = signals.hr_drift_percent {
            warning_signs.push(format!(
                "Heart rate drift detected: {drift_pct:.1}% increase - possible fatigue"
            ));
        } else {
            warning_signs.push("Heart rate drift detected - possible fatigue".to_owned());
        }
    }

    if signals.performance_decline {
        warning_signs.push("Performance declining despite training - check recovery".to_owned());
    }

    if signals.insufficient_recovery {
        warning_signs.push("Insufficient recovery time between hard efforts".to_owned());
    }

    let risk_level_str = match signals.risk_level {
        RiskLevel::Low => "low",
        RiskLevel::Moderate => "moderate",
        RiskLevel::High => "high",
    };

    let recommendations = match signals.risk_level {
        RiskLevel::High => vec![
            "Take additional rest days",
            "Reduce training intensity and volume",
            "Focus on recovery and sleep quality",
            "Consider consulting with a coach or sports medicine professional",
        ],
        RiskLevel::Moderate => vec![
            "Monitor recovery closely",
            "Ensure adequate rest days",
            "Review training intensity distribution",
        ],
        RiskLevel::Low => vec![
            "Continue current training approach",
            "Maintain good recovery habits",
        ],
    };

    PatternsResult::Overtraining(Box::new(OvertrainingResult {
        pattern_type: "overtraining".to_owned(),
        risk_level: risk_level_str.to_owned(),
        insights: if warning_signs.is_empty() {
            vec![
                "No significant overtraining signs detected - training load appears manageable"
                    .to_owned(),
            ]
        } else {
            warning_signs.clone()
        },
        warning_signs,
        hr_drift_detected: signals.hr_drift_detected,
        performance_decline: signals.performance_decline,
        insufficient_recovery: signals.insufficient_recovery,
        confidence: "medium".to_owned(),
        recommendations: recommendations.into_iter().map(ToOwned::to_owned).collect(),
    }))
}

/// Handle `detect_patterns` tool - detect patterns in activity data
#[must_use]
pub fn handle_detect_patterns(
    executor: &UniversalToolExecutor,
    request: UniversalRequest,
) -> Pin<Box<dyn Future<Output = Result<UniversalResponse, ProtocolError>> + Send + '_>> {
    Box::pin(async move {
        use parse_user_id_for_protocol;

        // Check cancellation at start
        if let Some(token) = &request.cancellation_token {
            if token.is_cancelled().await {
                return Err(ProtocolError::OperationCancelled(
                    "detect_patterns cancelled by user".to_owned(),
                ));
            }
        }

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
            Err(response) => return Ok(response),
        };
        let pattern_type = request
            .parameters
            .get("pattern_type")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ProtocolError::InvalidRequest("Missing required parameter: pattern_type".to_owned())
            })?;

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
                    "detect_patterns cancelled before authentication".to_owned(),
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
                        Some("Authenticated - analyzing activities for patterns...".to_owned()),
                    );
                }

                // Check cancellation before pattern detection
                if let Some(token) = &request.cancellation_token {
                    if token.is_cancelled().await {
                        return Err(ProtocolError::OperationCancelled(
                            "detect_patterns cancelled before analysis".to_owned(),
                        ));
                    }
                }

                // The athlete's own zone: the weekly-schedule histograms are
                // counted on their civil clock, not the server's (registre#252).
                let user_timezone = executor
                    .resources
                    .repos()
                    .users
                    .get_global(user_uuid)
                    .await
                    .ok()
                    .flatten()
                    .and_then(|user| user.timezone);

                let result = fetch_and_detect_patterns(
                    provider,
                    pattern_type,
                    user_uuid,
                    user_timezone.as_deref(),
                    output_format,
                )
                .await?;

                // Report completion on success
                if result.success {
                    if let Some(reporter) = &request.progress_reporter {
                        reporter.report(
                            100.0,
                            Some(100.0),
                            Some("Pattern detection completed".to_owned()),
                        );
                    }
                }

                Ok(result)
            }
            Err(response) => Ok(response),
        }
    })
}
