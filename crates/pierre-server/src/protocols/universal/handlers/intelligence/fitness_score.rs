// ABOUTME: Handler for calculate_fitness_score tool using CTL/ATL/TSS methodology
// ABOUTME: Computes composite fitness score from training load, consistency, and performance trend
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::config::environment::default_provider;
use crate::intelligence::physiological_constants::api_limits::DEFAULT_ACTIVITY_LIMIT;
use crate::intelligence::{SleepAnalyzer, TrainingLoadCalculator};
use crate::models::Activity;
use crate::protocols::universal::handlers::{apply_format_to_response, extract_output_format};
use crate::protocols::universal::{UniversalRequest, UniversalResponse, UniversalToolExecutor};
use crate::protocols::ProtocolError;
use crate::utils::uuid::parse_user_id_for_protocol;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use tracing::warn;
#[cfg(feature = "client-notifications")]
use {
    crate::mcp::resources::ServerContext, pierre_notifications::triggers as notification_triggers,
    pierre_notifications::TenantId, std::sync::Arc, uuid::Uuid,
};

/// Information about recovery adjustment applied to fitness score
struct RecoveryAdjustmentInfo {
    /// Recovery quality score (0-100)
    recovery_score: f64,
    /// Adjustment factor applied to fitness score (0.9-1.1)
    adjustment_factor: f64,
    /// Provider name used for sleep data
    provider_name: String,
}

/// Fetch sleep data and calculate recovery adjustment for fitness score
///
/// Fetches recent sleep data from the specified provider and calculates a recovery
/// score that adjusts the fitness score based on current recovery status.
///
/// Recovery adjustment factors:
/// - 90-100 (Excellent): +5% bonus (1.05)
/// - 70-89 (Good): No adjustment (1.0)
/// - 50-69 (Moderate): -5% penalty (0.95)
/// - <50 (Poor): -10% penalty (0.90)
async fn fetch_and_calculate_recovery_adjustment(
    executor: &UniversalToolExecutor,
    user_uuid: uuid::Uuid,
    tenant_id: Option<&str>,
    sleep_provider_name: &str,
    analysis: &mut serde_json::Value,
) -> Result<RecoveryAdjustmentInfo, String> {
    use crate::protocols::universal::handlers::sleep_recovery::fetch_provider_sleep_data;

    // Fetch sleep data from provider
    let sleep_data =
        fetch_provider_sleep_data(executor, user_uuid, tenant_id, sleep_provider_name, 1)
            .await
            .map_err(|e| e.error.unwrap_or_else(|| "Unknown error".to_owned()))?;

    // Calculate sleep quality score using SleepAnalyzer
    let cageux_config = executor.cageux_config();
    let config = &cageux_config.sleep_recovery;
    let sleep_quality = SleepAnalyzer::calculate_sleep_quality(&sleep_data, config)
        .map_err(|e| format!("Sleep quality calculation failed: {e}"))?;

    let recovery_score = sleep_quality.overall_score;

    // Calculate adjustment factor based on recovery score
    let adjustment_factor = if recovery_score >= 90.0 {
        1.05 // Excellent recovery: +5%
    } else if recovery_score >= 70.0 {
        1.0 // Good recovery: no adjustment
    } else if recovery_score >= 50.0 {
        0.95 // Moderate recovery: -5%
    } else {
        0.90 // Poor recovery: -10%
    };

    // Apply adjustment to fitness score in the analysis
    if let Some(obj) = analysis.as_object_mut() {
        if let Some(serde_json::Value::Number(score)) = obj.get("fitness_score") {
            if let Some(current_score) = score.as_i64() {
                // Safe: fitness score is 0-100, adjustment factor is 0.9-1.1, result fits in i64
                #[allow(clippy::cast_precision_loss)]
                #[allow(clippy::cast_possible_truncation)]
                let adjusted_score = ((current_score as f64) * adjustment_factor).round() as i64;
                obj.insert(
                    "fitness_score".to_owned(),
                    serde_json::Value::Number(adjusted_score.into()),
                );
                obj.insert(
                    "fitness_score_unadjusted".to_owned(),
                    serde_json::Value::Number(current_score.into()),
                );
            }
        }
    }

    Ok(RecoveryAdjustmentInfo {
        recovery_score,
        adjustment_factor,
        provider_name: sleep_provider_name.to_owned(),
    })
}

/// Calculate fitness metrics using CTL/ATL/TSS methodology
/// Calculate fitness metrics using proper 3-component formula with `TrainingLoadCalculator`
fn calculate_fitness_metrics(activities: &[Activity], timeframe: &str) -> serde_json::Value {
    use chrono::{Duration, Utc};
    use TrainingLoadCalculator;

    if activities.is_empty() {
        return serde_json::json!({
            "timeframe": timeframe,
            "fitness_score": 0,
            "level": "Beginner",
            "message": "No activities found for fitness calculation",
        });
    }

    // Filter activities by timeframe
    let now = Utc::now();
    let timeframe_days = match timeframe {
        "last_90_days" => 90,
        "all_time" => 365 * 10, // 10 years
        _ => 30,                // default to 30 days (includes "last_30_days")
    };

    let cutoff_date = now - Duration::days(timeframe_days);
    let filtered_activities: Vec<_> = activities
        .iter()
        .filter(|a| a.start_date() >= cutoff_date)
        .cloned()
        .collect();

    if filtered_activities.is_empty() {
        return serde_json::json!({
            "timeframe": timeframe,
            "fitness_score": 0,
            "level": "Beginner",
            "message": format!("No activities found in the last {timeframe_days} days"),
        });
    }

    // Component 1: CTL (Chronic Training Load) - 40% weight
    // Sort oldest-first — EMA calculation requires chronological order
    let mut sorted_activities = filtered_activities.clone();
    sorted_activities.sort_by_key(Activity::start_date);
    let calculator = TrainingLoadCalculator::new();
    let training_load = calculator
        .calculate_training_load(&sorted_activities, None, None, None, None, None)
        .ok();

    let ctl = training_load.as_ref().map_or(0.0, |l| l.ctl);
    let atl = training_load.as_ref().map_or(0.0, |l| l.atl);
    let tsb = training_load.as_ref().map_or(0.0, |l| l.tsb);

    // CTL component: normalize to 0-100 scale (150 CTL = 100 score)
    let ctl_score = (ctl / 150.0 * 100.0).min(100.0);

    // Component 2: Consistency (% weeks with 3+ activities) - 30% weight
    let consistency_score = calculate_consistency_score(&filtered_activities);

    // Component 3: Performance trend (pace improvement) - 30% weight
    let performance_score = calculate_performance_trend(&filtered_activities);

    // Combine components with weights: 40% CTL, 30% consistency, 30% performance
    let fitness_score =
        ctl_score.mul_add(0.4, consistency_score.mul_add(0.3, performance_score * 0.3));

    // Classify fitness level based on score
    let fitness_level = classify_fitness_level(fitness_score);

    // Determine trend (comparing first half vs second half)
    let trend = calculate_trend(&filtered_activities);

    #[allow(clippy::cast_possible_truncation)]
    let fitness_score_int = fitness_score.round() as i32;

    serde_json::json!({
        "timeframe": timeframe,
        "fitness_score": fitness_score_int,
        "level": fitness_level,
        "trend": trend,
        "components": {
            "ctl_score": ctl_score.round(),
            "consistency_score": consistency_score.round(),
            "performance_score": performance_score.round(),
        },
        "metrics": {
            "ctl": ctl.round(),
            "atl": atl.round(),
            "tsb": tsb.round(),
        },
        "activities_analyzed": filtered_activities.len(),
        "interpretation": {
            "ctl": "Chronic Training Load - long-term fitness (42-day average)",
            "consistency": "Training frequency and regularity",
            "performance": "Pace/speed improvement over time",
        },
    })
}

/// Calculate consistency score: percentage of weeks with 3+ activities
fn calculate_consistency_score(activities: &[Activity]) -> f64 {
    use HashMap;

    if activities.is_empty() {
        return 0.0;
    }

    // Get the date range
    let first_date = activities.iter().map(Activity::start_date).min();
    let last_date = activities.iter().map(Activity::start_date).max();

    let (Some(first), Some(last)) = (first_date, last_date) else {
        return 0.0;
    };

    // Calculate total weeks spanned
    let weeks_spanned = ((last - first).num_days() / 7).max(1);

    // Group activities by week number (days since first / 7)
    let mut activities_per_week: HashMap<i64, u32> = HashMap::new();

    for activity in activities {
        let days_since_first = (activity.start_date() - first).num_days();
        let week_number = days_since_first / 7;
        *activities_per_week.entry(week_number).or_insert(0) += 1;
    }

    // Count weeks with 3+ activities
    let active_weeks = activities_per_week
        .values()
        .filter(|&&count| count >= 3)
        .count();

    // Calculate consistency score as percentage
    #[allow(clippy::cast_precision_loss)]
    let score = (active_weeks as f64 / weeks_spanned as f64) * 100.0;
    score.min(100.0)
}

/// Calculate performance trend: improvement in average pace over time
fn calculate_performance_trend(activities: &[Activity]) -> f64 {
    let activities_with_pace: Vec<_> = activities
        .iter()
        .filter(|a| a.distance_meters().is_some() && a.duration_seconds() > 0)
        .collect();

    if activities_with_pace.len() < 4 {
        return 50.0; // neutral score for insufficient data
    }

    // Split into first and second half
    let mid_point = activities_with_pace.len() / 2;
    let first_half = &activities_with_pace[..mid_point];
    let second_half = &activities_with_pace[mid_point..];

    // Calculate average pace for each half (seconds per meter)
    #[allow(clippy::cast_precision_loss)]
    let first_avg_pace: f64 = first_half
        .iter()
        .map(|a| {
            let distance = a.distance_meters().unwrap_or_else(|| {
                warn!(
                    activity_id = a.id(),
                    "Activity missing distance_meters in pace calculation, using 1.0m fallback"
                );
                1.0
            });
            a.duration_seconds() as f64 / distance
        })
        .sum::<f64>()
        / first_half.len() as f64;

    #[allow(clippy::cast_precision_loss)]
    let second_avg_pace: f64 = second_half
        .iter()
        .map(|a| {
            let distance = a.distance_meters().unwrap_or_else(|| {
                warn!(
                    activity_id = a.id(),
                    "Activity missing distance_meters in pace calculation, using 1.0m fallback"
                );
                1.0
            });
            a.duration_seconds() as f64 / distance
        })
        .sum::<f64>()
        / second_half.len() as f64;

    // Lower pace is better (faster), so improvement is first_pace - second_pace
    let pace_improvement_pct = ((first_avg_pace - second_avg_pace) / first_avg_pace) * 100.0;

    // Map improvement to 0-100 score
    // -10% to +10% improvement maps to 0-100
    let score = (pace_improvement_pct + 10.0) * 5.0;
    score.clamp(0.0, 100.0)
}

/// Classify fitness level based on composite score (0-100)
fn classify_fitness_level(score: f64) -> &'static str {
    if score >= 80.0 {
        "Excellent"
    } else if score >= 60.0 {
        "Good"
    } else if score >= 40.0 {
        "Moderate"
    } else if score >= 20.0 {
        "Developing"
    } else {
        "Beginner"
    }
}

/// Calculate fitness trend by comparing recent vs older activities
fn calculate_trend(activities: &[Activity]) -> &'static str {
    if activities.len() < 4 {
        return "stable";
    }

    let mid_point = activities.len() / 2;
    let older_half = &activities[..mid_point];
    let recent_half = &activities[mid_point..];

    #[allow(clippy::cast_precision_loss)]
    let older_avg_duration = older_half
        .iter()
        .map(Activity::duration_seconds)
        .sum::<u64>() as f64
        / older_half.len() as f64;

    #[allow(clippy::cast_precision_loss)]
    let recent_avg_duration = recent_half
        .iter()
        .map(Activity::duration_seconds)
        .sum::<u64>() as f64
        / recent_half.len() as f64;

    let change_pct = ((recent_avg_duration - older_avg_duration) / older_avg_duration) * 100.0;

    if change_pct > 15.0 {
        "improving"
    } else if change_pct < -15.0 {
        "declining"
    } else {
        "stable"
    }
}

/// Handle `calculate_fitness_score` tool - calculate overall fitness score
///
/// Supports cross-provider integration:
/// - Use `provider` to specify where to fetch activity data (default: configured default provider)
/// - Use `sleep_provider` to optionally fetch recovery data from a different provider
///
/// When `sleep_provider` is specified, recovery quality factors into the fitness score:
/// - Excellent recovery (90-100): +5% fitness score bonus
/// - Good recovery (70-89): No adjustment
/// - Poor recovery (<70): -5% to -10% penalty
///
/// # Parameters
/// - `provider` (optional): Activity provider (default: configured default)
/// - `sleep_provider` (optional): Sleep/recovery provider for cross-provider analysis
/// - `timeframe` (optional): `month`, `last_90_days`, or `all_time`
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn handle_calculate_fitness_score(
    executor: &UniversalToolExecutor,
    request: UniversalRequest,
) -> Pin<Box<dyn Future<Output = Result<UniversalResponse, ProtocolError>> + Send + '_>> {
    Box::pin(async move {
        use parse_user_id_for_protocol;
        use DEFAULT_ACTIVITY_LIMIT;

        // Check cancellation at start
        if let Some(token) = &request.cancellation_token {
            if token.is_cancelled().await {
                return Err(ProtocolError::OperationCancelled(
                    "calculate_fitness_score cancelled by user".to_owned(),
                ));
            }
        }

        let provider_name = request
            .parameters
            .get("provider")
            .and_then(|v| v.as_str())
            .map_or_else(default_provider, String::from);
        let user_uuid = parse_user_id_for_protocol(&request.user_id)?;
        let timeframe = request
            .parameters
            .get("timeframe")
            .and_then(|v| v.as_str())
            .unwrap_or("month");

        // Extract optional sleep_provider for cross-provider recovery analysis
        let sleep_provider = request
            .parameters
            .get("sleep_provider")
            .and_then(|v| v.as_str());

        // Extract output format parameter: "json" (default) or "toon"
        let output_format = extract_output_format(&request);

        // Report progress - starting authentication
        if let Some(reporter) = &request.progress_reporter {
            reporter.report(
                20.0,
                Some(100.0),
                Some("Checking authentication...".to_owned()),
            );
        }

        // Check cancellation before auth
        if let Some(token) = &request.cancellation_token {
            if token.is_cancelled().await {
                return Err(ProtocolError::OperationCancelled(
                    "calculate_fitness_score cancelled before authentication".to_owned(),
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
                        40.0,
                        Some(100.0),
                        Some("Authenticated - fetching activities...".to_owned()),
                    );
                }

                // Check cancellation before provider creation
                if let Some(token) = &request.cancellation_token {
                    if token.is_cancelled().await {
                        return Err(ProtocolError::OperationCancelled(
                            "calculate_fitness_score cancelled before fetch".to_owned(),
                        ));
                    }
                }

                match provider
                    .get_activities(Some(DEFAULT_ACTIVITY_LIMIT), None)
                    .await
                {
                    Ok(activities) => {
                        // Report progress before calculation
                        if let Some(reporter) = &request.progress_reporter {
                            reporter.report(
                                70.0,
                                Some(100.0),
                                Some("Calculating fitness metrics...".to_owned()),
                            );
                        }

                        let mut analysis = calculate_fitness_metrics(&activities, timeframe);

                        // If sleep_provider is specified, fetch recovery data and adjust score
                        let recovery_info = if let Some(sleep_provider_name) = sleep_provider {
                            match fetch_and_calculate_recovery_adjustment(
                                executor,
                                user_uuid,
                                request.tenant_id.as_deref(),
                                sleep_provider_name,
                                &mut analysis,
                            )
                            .await
                            {
                                Ok(info) => Some(info),
                                Err(err_msg) => {
                                    warn!(
                                        sleep_provider = sleep_provider_name,
                                        error = %err_msg,
                                        "Failed to fetch recovery data, proceeding without adjustment"
                                    );
                                    None
                                }
                            }
                        } else {
                            None
                        };

                        // Report completion
                        if let Some(reporter) = &request.progress_reporter {
                            reporter.report(
                                100.0,
                                Some(100.0),
                                Some("Fitness score calculated".to_owned()),
                            );
                        }

                        // Add recovery and provider info to response
                        if let Some(obj) = analysis.as_object_mut() {
                            if let Some(ref info) = recovery_info {
                                obj.insert(
                                    "recovery_adjustment".to_owned(),
                                    serde_json::json!({
                                        "recovery_score": info.recovery_score,
                                        "adjustment_factor": info.adjustment_factor,
                                        "sleep_provider": info.provider_name,
                                    }),
                                );
                            }
                            obj.insert(
                                "providers_used".to_owned(),
                                serde_json::json!({
                                    "activity_provider": provider_name,
                                    "sleep_provider": sleep_provider,
                                }),
                            );
                        }

                        // Fire fitness improvement notification if trend is improving
                        #[cfg(feature = "client-notifications")]
                        fire_fitness_improvement_notification(
                            &executor.resources,
                            user_uuid,
                            request.tenant_id.as_deref(),
                            &analysis,
                        );

                        let result = UniversalResponse {
                            success: true,
                            result: Some(analysis),
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
                                if let Some(sp) = sleep_provider {
                                    map.insert(
                                        "sleep_provider".to_owned(),
                                        serde_json::Value::String(sp.to_owned()),
                                    );
                                }
                                if recovery_info.is_some() {
                                    map.insert(
                                        "recovery_factored".to_owned(),
                                        serde_json::Value::Bool(true),
                                    );
                                }
                                map
                            }),
                        };

                        // Apply format transformation
                        Ok(apply_format_to_response(
                            result,
                            "fitness_score",
                            output_format,
                        ))
                    }
                    Err(e) => Ok(UniversalResponse {
                        success: false,
                        result: None,
                        error: Some(format!("Failed to fetch activities: {e}")),
                        metadata: None,
                    }),
                }
            }
            Err(response) => Ok(response),
        }
    })
}

/// Fire a fitness improvement notification when the trend is "improving"
/// and the fitness score exceeds a meaningful threshold.
#[cfg(feature = "client-notifications")]
fn fire_fitness_improvement_notification(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant_id_str: Option<&str>,
    analysis: &serde_json::Value,
) {
    let Some(service) = &resources.notification_service else {
        return;
    };
    let Some(tenant_str) = tenant_id_str else {
        return;
    };
    let Ok(tenant_uuid) = tenant_str.parse::<Uuid>() else {
        return;
    };
    let tenant_id = TenantId(tenant_uuid);

    let trend = analysis["trend"].as_str().unwrap_or("");
    let score = analysis["fitness_score"].as_i64().unwrap_or(0);

    // Only trigger when fitness trend is improving and score is meaningful
    if trend == "improving" && score > 0 {
        notification_triggers::trigger_fitness_improvement(
            service,
            user_id,
            tenant_id,
            "Fitness Score",
            &format!("{score}"),
        );
    }
}
