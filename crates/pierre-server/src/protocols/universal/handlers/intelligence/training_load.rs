// ABOUTME: Handler for analyze_training_load tool with CTL/ATL/TSB metrics
// ABOUTME: Calculates chronic/acute training load and stress balance with optional sleep recovery context
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::config::environment::default_provider;
use crate::config::intelligence::IntelligenceConfig;
use crate::intelligence::physiological_constants::api_limits::DEFAULT_ACTIVITY_LIMIT;
use crate::intelligence::{SleepAnalyzer, TrainingLoadCalculator, TssDataPoint};
use crate::models::Activity;
use crate::protocols::universal::handlers::{apply_format_to_response, extract_output_format};
use crate::protocols::universal::{UniversalRequest, UniversalResponse, UniversalToolExecutor};
use crate::protocols::ProtocolError;
use crate::utils::uuid::parse_user_id_for_protocol;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use tracing::warn;

/// Recovery context from sleep/HRV data for training load interpretation
struct RecoveryContextInfo {
    /// Sleep quality score (0-100)
    sleep_quality_score: f64,
    /// Recovery status interpretation
    recovery_status: String,
    /// HRV RMSSD if available
    hrv_rmssd: Option<f64>,
    /// Sleep duration in hours
    sleep_hours: f64,
}

/// Fetch recovery context for training load analysis
///
/// Fetches recent sleep data and provides recovery context to interpret
/// training load data (CTL/ATL/TSB) more accurately.
async fn fetch_recovery_context_for_training_load(
    executor: &UniversalToolExecutor,
    user_uuid: uuid::Uuid,
    tenant_id: Option<&str>,
    sleep_provider_name: &str,
) -> Result<RecoveryContextInfo, String> {
    use crate::protocols::universal::handlers::sleep_recovery::fetch_provider_sleep_data;
    use SleepAnalyzer;

    // Fetch sleep data from provider
    let sleep_data =
        fetch_provider_sleep_data(executor, user_uuid, tenant_id, sleep_provider_name, 1)
            .await
            .map_err(|e| e.error.unwrap_or_else(|| "Unknown error".to_owned()))?;

    // Calculate sleep quality score
    let config = &IntelligenceConfig::global().sleep_recovery;
    let sleep_quality = SleepAnalyzer::calculate_sleep_quality(&sleep_data, config)
        .map_err(|e| format!("Sleep quality calculation failed: {e}"))?;

    // Determine recovery status based on sleep quality
    let recovery_status = if sleep_quality.overall_score >= 90.0 {
        "excellent".to_owned()
    } else if sleep_quality.overall_score >= 75.0 {
        "good".to_owned()
    } else if sleep_quality.overall_score >= 60.0 {
        "moderate".to_owned()
    } else if sleep_quality.overall_score >= 40.0 {
        "fair".to_owned()
    } else {
        "poor".to_owned()
    };

    Ok(RecoveryContextInfo {
        sleep_quality_score: sleep_quality.overall_score,
        recovery_status,
        hrv_rmssd: sleep_data.hrv_rmssd_ms,
        sleep_hours: sleep_data.duration_hours,
    })
}

/// Analyze detailed training load from activities
fn analyze_detailed_training_load(activities: &[Activity], timeframe: &str) -> serde_json::Value {
    use TrainingLoadCalculator;

    if activities.is_empty() {
        return serde_json::json!({
            "timeframe": timeframe,
            "message": "No activities found for training load analysis",
        });
    }

    // Use TrainingLoadCalculator from Phase 1 foundation
    let calculator = TrainingLoadCalculator::new();

    // Calculate training load (CTL, ATL, TSB) using real TSS calculation
    // Note: For accurate TSS, we'd need user's FTP, LTHR, max_hr, etc.
    // For now, use None values which will trigger estimation
    let Ok(training_load) = calculator.calculate_training_load(
        activities, None, // FTP
        None, // LTHR
        None, // max_hr
        None, // resting_hr
        None, // weight_kg
    ) else {
        return serde_json::json!({
            "timeframe": timeframe,
            "message": "Unable to calculate training load - insufficient activity data",
        });
    };

    let ctl = training_load.ctl;
    let atl = training_load.atl;
    let tsb = training_load.tsb;

    // Calculate weekly TSS totals from TSS history
    let weekly_tss = calculate_weekly_tss_from_history(&training_load.tss_history);

    // Determine load status
    let load_status = determine_load_status(ctl, atl, tsb);

    // Check for overtraining risk
    let overtraining_risk = if tsb < -30.0 {
        "high"
    } else if tsb < -20.0 {
        "moderate"
    } else {
        "low"
    };

    // Taper recommendations
    let taper_recommendation = if tsb > 10.0 {
        "Well tapered - ready for peak performance"
    } else if tsb > 0.0 {
        "Good taper status"
    } else if tsb > -10.0 {
        "Consider light taper for upcoming events"
    } else {
        "Significant taper needed before racing"
    };

    // Periodization suggestions
    let mut periodization_suggestions = Vec::new();
    if atl > ctl * 1.5 {
        periodization_suggestions
            .push("Recent spike in training - allow adaptation time".to_owned());
    }
    if ctl < 30.0 {
        periodization_suggestions
            .push("Building base - focus on consistency and volume".to_owned());
    } else if ctl > 80.0 {
        periodization_suggestions
            .push("High fitness level - maintain or add recovery weeks".to_owned());
    }

    serde_json::json!({
        "timeframe": timeframe,
        "load_metrics": {
            "ctl": ctl.round(),
            "atl": atl.round(),
            "tsb": tsb.round(),
            "weekly_tss": weekly_tss,
        },
        "load_status": load_status,
        "overtraining_risk": overtraining_risk,
        "taper_status": taper_recommendation,
        "periodization_suggestions": periodization_suggestions,
        "training_zones": classify_training_load(ctl),
        "recommendations": generate_load_recommendations(ctl, atl, tsb),
        "activities_analyzed": training_load.tss_history.len(),
        "interpretation": {
            "ctl": "Chronic Training Load - fitness level (42-day average TSS)",
            "atl": "Acute Training Load - fatigue level (7-day average TSS)",
            "tsb": "Training Stress Balance - form indicator (CTL - ATL)",
            "positive_tsb": "Fresh and recovered, ready for hard training",
            "negative_tsb": "Fatigued, prioritize recovery",
        },
    })
}

/// Calculate weekly TSS totals from `TssDataPoint` history (Phase 1 format)
fn calculate_weekly_tss_from_history(tss_history: &[TssDataPoint]) -> Vec<serde_json::Value> {
    use HashMap;

    if tss_history.is_empty() {
        return Vec::new();
    }

    // Group by week
    let mut weekly_totals: HashMap<i32, f64> = HashMap::new();
    let first_date = tss_history[0].date;

    for point in tss_history {
        let days_diff = (point.date - first_date).num_days();
        #[allow(clippy::cast_possible_truncation)]
        let week_number_i32 = (days_diff / 7) as i32;
        *weekly_totals.entry(week_number_i32).or_insert(0.0) += point.tss;
    }

    // Convert to sorted vec
    let mut weeks: Vec<(i32, f64)> = weekly_totals.into_iter().collect();
    weeks.sort_by_key(|(week, _)| *week);

    weeks
        .iter()
        .map(|(week, tss)| {
            serde_json::json!({
                "week": week,
                "total_tss": tss.round(),
            })
        })
        .collect()
}

/// Determine overall load status
fn determine_load_status(_ctl: f64, _atl: f64, tsb: f64) -> String {
    if tsb < -25.0 {
        "Overreached - high fatigue".to_owned()
    } else if tsb < -10.0 {
        "Productive - building fitness under fatigue".to_owned()
    } else if tsb < 5.0 {
        "Balanced - good training stress balance".to_owned()
    } else if tsb < 15.0 {
        "Fresh - ready for quality work".to_owned()
    } else {
        "Very fresh - possibly detraining".to_owned()
    }
}

/// Classify training load level
fn classify_training_load(ctl: f64) -> serde_json::Value {
    let level = if ctl < 25.0 {
        "Beginner"
    } else if ctl < 45.0 {
        "Intermediate"
    } else if ctl < 70.0 {
        "Advanced"
    } else if ctl < 100.0 {
        "Elite"
    } else {
        "Very High"
    };

    serde_json::json!({
        "level": level,
        "ctl_range": match level {
            "Beginner" => "< 25",
            "Intermediate" => "25-45",
            "Advanced" => "45-70",
            "Elite" => "70-100",
            _ => "> 100",
        },
    })
}

/// Generate load-specific recommendations
fn generate_load_recommendations(ctl: f64, atl: f64, tsb: f64) -> Vec<String> {
    let mut recommendations = Vec::new();

    // TSB-based recommendations
    if tsb < -25.0 {
        recommendations.push("⚠️ Critical fatigue - take 2-3 rest days immediately".to_owned());
        recommendations.push("Reduce training volume by 50% this week".to_owned());
    } else if tsb < -15.0 {
        recommendations.push("High fatigue - schedule recovery week".to_owned());
        recommendations.push("Reduce intensity and add extra rest day".to_owned());
    } else if tsb < -5.0 {
        recommendations
            .push("Moderate fatigue - maintain current load or slight reduction".to_owned());
    } else if tsb > 15.0 {
        recommendations.push("Very fresh - good time for breakthrough workout or race".to_owned());
    }

    // CTL/ATL ratio analysis
    let ratio = if ctl > 0.0 { atl / ctl } else { 0.0 };
    if ratio > 1.5 {
        recommendations
            .push("Recent training spike detected - allow 1-2 weeks adaptation".to_owned());
    } else if ratio < 0.8 && ctl > 30.0 {
        recommendations.push("Well adapted to training - can increase load gradually".to_owned());
    }

    // Progressive load recommendations
    if ctl < 30.0 {
        recommendations.push("Build weekly TSS by 3-5 points per week".to_owned());
    } else if ctl > 80.0 {
        recommendations
            .push("High load - incorporate recovery weeks (reduce by 20-30%)".to_owned());
    }

    if recommendations.is_empty() {
        recommendations
            .push("Training load is well balanced - maintain current approach".to_owned());
    }

    recommendations
}

/// Handle `analyze_training_load` tool - analyze training load metrics (CTL/ATL/TSB)
#[must_use]
pub fn handle_analyze_training_load(
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
                    "analyze_training_load cancelled by user".to_owned(),
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
            .unwrap_or("week");

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
                    "analyze_training_load cancelled before authentication".to_owned(),
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
                            "analyze_training_load cancelled before fetch".to_owned(),
                        ));
                    }
                }

                match provider
                    .get_activities(Some(DEFAULT_ACTIVITY_LIMIT), None)
                    .await
                {
                    Ok(activities) => {
                        // Report progress before analysis
                        if let Some(reporter) = &request.progress_reporter {
                            reporter.report(
                                70.0,
                                Some(100.0),
                                Some(format!(
                                    "Analyzing training load for {} activities...",
                                    activities.len()
                                )),
                            );
                        }

                        let mut analysis = analyze_detailed_training_load(&activities, timeframe);

                        // If sleep_provider is specified, fetch recovery context
                        let recovery_context = if let Some(sleep_provider_name) = sleep_provider {
                            match fetch_recovery_context_for_training_load(
                                executor,
                                user_uuid,
                                request.tenant_id.as_deref(),
                                sleep_provider_name,
                            )
                            .await
                            {
                                Ok(context) => {
                                    // Add recovery context to analysis
                                    if let Some(obj) = analysis.as_object_mut() {
                                        obj.insert(
                                            "recovery_context".to_owned(),
                                            serde_json::json!({
                                                "sleep_quality_score": context.sleep_quality_score,
                                                "recovery_status": context.recovery_status,
                                                "hrv_available": context.hrv_rmssd.is_some(),
                                                "hrv_rmssd": context.hrv_rmssd,
                                                "sleep_hours": context.sleep_hours,
                                                "sleep_provider": sleep_provider_name,
                                            }),
                                        );
                                    }
                                    Some(context)
                                }
                                Err(err_msg) => {
                                    warn!(
                                        sleep_provider = sleep_provider_name,
                                        error = %err_msg,
                                        "Failed to fetch recovery context, proceeding without"
                                    );
                                    None
                                }
                            }
                        } else {
                            None
                        };

                        // Add provider info
                        if let Some(obj) = analysis.as_object_mut() {
                            obj.insert(
                                "providers_used".to_owned(),
                                serde_json::json!({
                                    "activity_provider": provider_name,
                                    "sleep_provider": sleep_provider,
                                }),
                            );
                        }

                        // Report completion
                        if let Some(reporter) = &request.progress_reporter {
                            reporter.report(
                                100.0,
                                Some(100.0),
                                Some("Training load analysis completed".to_owned()),
                            );
                        }

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
                                if recovery_context.is_some() {
                                    map.insert(
                                        "recovery_context_included".to_owned(),
                                        serde_json::Value::Bool(true),
                                    );
                                }
                                map
                            }),
                        };

                        // Apply format transformation
                        Ok(apply_format_to_response(
                            result,
                            "training_load",
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
