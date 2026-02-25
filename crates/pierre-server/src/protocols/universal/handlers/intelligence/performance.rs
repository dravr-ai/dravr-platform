// ABOUTME: Handler for predict_performance tool using VDOT and Riegel formulas
// ABOUTME: Predicts race times for 5K, 10K, half marathon, and marathon distances
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::config::environment::default_provider;
use crate::intelligence::physiological_constants::api_limits::DEFAULT_ACTIVITY_LIMIT;
use crate::intelligence::{PerformancePredictor, TrainingLoadCalculator};
use crate::models::Activity;
use crate::protocols::universal::handlers::{apply_format_to_response, extract_output_format};
use crate::protocols::universal::{UniversalRequest, UniversalResponse, UniversalToolExecutor};
use crate::protocols::ProtocolError;
use crate::utils::uuid::parse_user_id_for_protocol;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use tracing::warn;

/// Predict race performance using VDOT methodology from `PerformancePredictor`
fn predict_race_performance(activities: &[Activity], target_sport: &str) -> serde_json::Value {
    use PerformancePredictor;

    // Filter activities by sport type
    let running_activities: Vec<&Activity> = activities
        .iter()
        .filter(|a| format!("{:?}", a.sport_type()).contains("Run"))
        .collect();

    if running_activities.is_empty() {
        return serde_json::json!({
            "target_sport": target_sport,
            "message": "No running activities found for prediction",
            "predictions": {},
        });
    }

    // Find best recent performance using PerformancePredictor
    let owned_activities: Vec<_> = running_activities.iter().copied().cloned().collect();
    let Some(best_activity) = PerformancePredictor::find_best_performance(&owned_activities) else {
        return serde_json::json!({
            "target_sport": target_sport,
            "message": "No suitable activities found for prediction (need distance > 3km with valid time)",
            "predictions": {},
        });
    };

    let best_distance = best_activity.distance_meters().unwrap_or_else(|| {
        warn!(
            activity_id = best_activity.id(),
            "Best activity missing distance_meters despite find_best_performance validation, using 0.0m"
        );
        0.0
    });
    #[allow(clippy::cast_precision_loss)]
    let best_time = best_activity.duration_seconds() as f64;

    // Generate race predictions using PerformancePredictor (includes VDOT calculation)
    match PerformancePredictor::generate_race_predictions(best_distance, best_time) {
        Ok(race_predictions) => {
            // Calculate confidence based on data quality
            let confidence =
                calculate_prediction_confidence(&running_activities, &best_activity.start_date());

            // Convert predictions HashMap to JSON array format for consistency
            let predictions_array: Vec<serde_json::Value> = race_predictions
                .predictions
                .iter()
                .map(|(name, time_seconds)| {
                    let distance_meters = match name.as_str() {
                        "5K" => 5_000.0,
                        "10K" => 10_000.0,
                        "Half Marathon" => 21_097.5,
                        "Marathon" => 42_195.0,
                        _ => 0.0,
                    };
                    let pace_per_km = if distance_meters > 0.0 {
                        PerformancePredictor::format_pace_per_km(distance_meters / time_seconds)
                    } else {
                        "N/A".to_owned()
                    };

                    serde_json::json!({
                        "distance": name,
                        "distance_meters": distance_meters,
                        "predicted_time_seconds": time_seconds.round(),
                        "predicted_time_formatted": PerformancePredictor::format_time(*time_seconds),
                        "predicted_pace_min_km": pace_per_km,
                    })
                })
                .collect();

            serde_json::json!({
                "target_sport": target_sport,
                "vdot": race_predictions.vdot.round(),
                "best_performance": {
                    "distance_meters": best_distance,
                    "time_seconds": best_time,
                    "pace_min_km": PerformancePredictor::format_pace_per_km(best_distance / best_time),
                    "date": best_activity.start_date().to_rfc3339(),
                },
                "predictions": predictions_array,
                "confidence": confidence,
                "activities_analyzed": running_activities.len(),
                "notes": [
                    "Predictions assume proper race preparation and taper",
                    "Based on VDOT methodology by Jack Daniels",
                    "Actual performance may vary with conditions and training",
                ],
            })
        }
        // Error handling for generate_race_predictions failure
        Err(e) => {
            serde_json::json!({
                "target_sport": target_sport,
                "error": format!("Failed to generate predictions: {e}"),
                "predictions": [],
                "message": "Unable to calculate race predictions from available data",
            })
        }
    }
}

/// Calculate prediction confidence based on recency, training volume, and data quality
///
/// Confidence factors per B6 roadmap:
/// - Recency of best performance (< 30 days = high confidence)
/// - Training volume (high CTL = more confidence)
/// - Number of recent races and consistency
#[allow(clippy::cast_precision_loss, clippy::bool_to_int_with_if)] // Multi-level threshold scoring, not simple boolean conversion
fn calculate_prediction_confidence(
    activities: &[&Activity],
    best_activity_date: &chrono::DateTime<chrono::Utc>,
) -> String {
    use chrono::Utc;
    use TrainingLoadCalculator;

    // Factor 1: Recency (< 30 days = high confidence)
    let days_since_best = (Utc::now() - *best_activity_date).num_days();
    let recency_score = if days_since_best < 30 {
        2 // Recent performance
    } else if days_since_best < 90 {
        1 // Moderately recent
    } else {
        0 // Old performance
    };

    // Factor 2: Training volume (CTL)
    let owned_activities: Vec<_> = activities.iter().copied().cloned().collect();
    let calculator = TrainingLoadCalculator::new();
    let ctl_score = if let Ok(training_load) =
        calculator.calculate_training_load(&owned_activities, None, None, None, None, None)
    {
        if training_load.ctl > 80.0 {
            2 // High training load
        } else if training_load.ctl > 40.0 {
            1 // Moderate training load
        } else {
            0 // Low training load
        }
    } else {
        0
    };

    // Factor 3: Number of activities
    let volume_score = if activities.len() >= 20 {
        2
    } else if activities.len() >= 10 {
        1
    } else {
        0
    };

    // Combine factors (max score = 6)
    let total_score = recency_score + ctl_score + volume_score;

    if total_score >= 5 {
        "high".to_owned()
    } else if total_score >= 3 {
        "medium".to_owned()
    } else {
        "low".to_owned()
    }
}

/// Handle `predict_performance` tool - predict future performance
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn handle_predict_performance(
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
                    "predict_performance cancelled by user".to_owned(),
                ));
            }
        }

        let provider_name = request
            .parameters
            .get("provider")
            .and_then(|v| v.as_str())
            .map_or_else(default_provider, String::from);
        let user_uuid = parse_user_id_for_protocol(&request.user_id)?;
        let target_sport = request
            .parameters
            .get("target_sport")
            .and_then(|v| v.as_str())
            .unwrap_or("Run");

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
                    "predict_performance cancelled before authentication".to_owned(),
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
                            "predict_performance cancelled before fetch".to_owned(),
                        ));
                    }
                }

                match provider
                    .get_activities(Some(DEFAULT_ACTIVITY_LIMIT), None)
                    .await
                {
                    Ok(activities) => {
                        // Report progress before prediction
                        if let Some(reporter) = &request.progress_reporter {
                            reporter.report(
                                70.0,
                                Some(100.0),
                                Some("Predicting race performance...".to_owned()),
                            );
                        }

                        let prediction = predict_race_performance(&activities, target_sport);

                        // Report completion
                        if let Some(reporter) = &request.progress_reporter {
                            reporter.report(
                                100.0,
                                Some(100.0),
                                Some("Performance prediction completed".to_owned()),
                            );
                        }

                        let result = UniversalResponse {
                            success: true,
                            result: Some(prediction),
                            error: None,
                            metadata: Some({
                                let mut map = HashMap::new();
                                map.insert(
                                    "user_id".to_owned(),
                                    serde_json::Value::String(user_uuid.to_string()),
                                );
                                map
                            }),
                        };

                        // Apply format transformation
                        Ok(apply_format_to_response(
                            result,
                            "prediction",
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
