// ABOUTME: Handler for detect_patterns tool identifying training patterns in activity data
// ABOUTME: Detects weekly schedules, training blocks, volume progression, and overtraining signals
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::config::environment::default_provider;
use crate::intelligence::physiological_constants::api_limits::DEFAULT_ACTIVITY_LIMIT;
use crate::intelligence::{
    HardEasyPattern, OvertrainingSignals, PatternDetector, RiskLevel, VolumeProgressionPattern,
    VolumeTrend, WeeklySchedulePattern,
};
use crate::models::Activity;
use crate::protocols::universal::handlers::{apply_format_to_response, extract_output_format};
use crate::protocols::universal::{UniversalRequest, UniversalResponse, UniversalToolExecutor};
use crate::protocols::ProtocolError;
use crate::providers::core::FitnessProvider;
use crate::utils::uuid::parse_user_id_for_protocol;
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
) -> UniversalResponse {
    use DEFAULT_ACTIVITY_LIMIT;

    match provider
        .get_activities(Some(DEFAULT_ACTIVITY_LIMIT), None)
        .await
    {
        Ok(activities) => {
            let analysis = detect_activity_patterns(&activities, pattern_type);

            UniversalResponse {
                success: true,
                result: Some(analysis),
                error: None,
                metadata: Some({
                    let mut map = HashMap::new();
                    map.insert(
                        "user_id".to_owned(),
                        serde_json::Value::String(user_uuid.to_string()),
                    );
                    map
                }),
            }
        }
        Err(e) => UniversalResponse {
            success: false,
            result: None,
            error: Some(format!("Failed to fetch activities: {e}")),
            metadata: None,
        },
    }
}

/// Detect patterns in activity data based on pattern type
fn detect_activity_patterns(activities: &[Activity], pattern_type: &str) -> serde_json::Value {
    use PatternDetector;

    if activities.len() < 3 {
        return serde_json::json!({
            "pattern_type": pattern_type,
            "activities_analyzed": activities.len(),
            "patterns_detected": [],
            "insights": ["Need at least 3 activities for pattern detection"],
            "confidence": "insufficient_data",
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
        _ => format_weekly_schedule(&PatternDetector::detect_weekly_schedule(activities)), // default: weekly_schedule
    }
}

/// Format weekly schedule pattern results for JSON response
fn format_weekly_schedule(pattern: &WeeklySchedulePattern) -> serde_json::Value {
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
    let preferred_days: Vec<serde_json::Value> = pattern
        .day_frequencies
        .iter()
        .map(|(day, &count)| {
            serde_json::json!({
                "day": day,
                "frequency": count,
            })
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

    serde_json::json!({
        "pattern_type": "weekly_schedule",
        "preferred_training_days": preferred_days,
        "patterns_detected": patterns,
        "insights": if patterns.is_empty() {
            vec!["No strong weekly schedule pattern detected - training is variable".to_owned()]
        } else {
            patterns.clone()
        },
        "consistency_score": pattern.consistency_score,
        "avg_activities_per_week": pattern.avg_activities_per_week,
        "confidence": confidence,
    })
}

/// Format hard/easy pattern results for JSON response
fn format_hard_easy_pattern(pattern: &HardEasyPattern) -> serde_json::Value {
    let mut insights = vec![pattern.pattern_description.clone()];

    if !pattern.adequate_recovery {
        insights.push("Consider adding more recovery days between hard efforts".to_owned());
    }

    let confidence = if pattern.pattern_detected {
        "medium"
    } else {
        "low"
    };

    serde_json::json!({
        "pattern_type": "training_blocks",
        "pattern_detected": pattern.pattern_detected,
        "intensity_distribution": {
            "hard_percentage": pattern.hard_percentage,
            "easy_percentage": pattern.easy_percentage,
        },
        "adequate_recovery": pattern.adequate_recovery,
        "patterns_detected": if pattern.pattern_detected {
            vec![pattern.pattern_description.clone()]
        } else {
            Vec::<String>::new()
        },
        "insights": insights,
        "confidence": confidence,
    })
}

/// Format volume progression pattern results for JSON response
fn format_volume_progression(pattern: &VolumeProgressionPattern) -> serde_json::Value {
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
            "Volume spikes detected in weeks: {} - monitor for injury risk",
            pattern
                .spike_weeks
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    serde_json::json!({
        "pattern_type": "progression",
        "trend": trend_description,
        "weekly_volumes": pattern.weekly_volumes,
        "week_numbers": pattern.week_numbers,
        "volume_spikes_detected": pattern.volume_spikes_detected,
        "spike_weeks": pattern.spike_weeks,
        "patterns_detected": insights.clone(),
        "insights": insights,
        "confidence": "medium",
    })
}

/// Format overtraining signals results for JSON response
fn format_overtraining_signals(signals: &OvertrainingSignals) -> serde_json::Value {
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

    serde_json::json!({
        "pattern_type": "overtraining",
        "risk_level": risk_level_str,
        "warning_signs": warning_signs,
        "insights": if warning_signs.is_empty() {
            vec!["No significant overtraining signs detected - training load appears manageable".to_owned()]
        } else {
            warning_signs.clone()
        },
        "hr_drift_detected": signals.hr_drift_detected,
        "performance_decline": signals.performance_decline,
        "insufficient_recovery": signals.insufficient_recovery,
        "confidence": "medium",
        "recommendations": recommendations,
    })
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

        let provider_name = request
            .parameters
            .get("provider")
            .and_then(|v| v.as_str())
            .map_or_else(default_provider, String::from);
        let user_uuid = parse_user_id_for_protocol(&request.user_id)?;
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

                let result = fetch_and_detect_patterns(provider, pattern_type, user_uuid).await;

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

                // Apply format transformation
                Ok(apply_format_to_response(result, "patterns", output_format))
            }
            Err(response) => Ok(response),
        }
    })
}
