// ABOUTME: Handler for analyze_performance_trends tool using linear regression
// ABOUTME: Tracks metric trends (pace, speed, HR, distance) over configurable timeframes
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::config::environment::default_provider;
use crate::intelligence::physiological_constants::api_limits::MAX_ACTIVITY_LIMIT;
use crate::intelligence::{
    MetricType, SafeMetricExtractor, StatisticalAnalyzer, TrendDataPoint, TrendDirection,
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

/// Fetch activities and analyze performance trends
///
/// Retrieves recent activities from provider and performs trend analysis
/// for the specified metric and timeframe.
///
/// # Arguments
/// * `provider` - Configured fitness provider
/// * `metric` - Metric to analyze (e.g., "pace", "distance")
/// * `timeframe` - Analysis timeframe (e.g., "week", "month")
/// * `user_uuid` - User UUID for response metadata
///
/// # Returns
/// `UniversalResponse` with trend analysis or error
async fn fetch_and_analyze_trends(
    provider: Box<dyn FitnessProvider>,
    metric: &str,
    timeframe: &str,
    user_uuid: uuid::Uuid,
) -> UniversalResponse {
    use MAX_ACTIVITY_LIMIT;

    match provider
        .get_activities(Some(MAX_ACTIVITY_LIMIT), None)
        .await
    {
        Ok(activities) => {
            let analysis = analyze_performance_trend(&activities, metric, timeframe);

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

/// Analyze performance trend for a specific metric over time
fn analyze_performance_trend(
    activities: &[Activity],
    metric: &str,
    timeframe: &str,
) -> serde_json::Value {
    use SafeMetricExtractor;

    if activities.is_empty() {
        return serde_json::json!({
            "metric": metric,
            "timeframe": timeframe,
            "trend": "no_data",
            "activities_analyzed": 0,
            "insights": ["No activities found for analysis"]
        });
    }

    // Parse metric string to MetricType
    let metric_type = match parse_metric_type(metric) {
        Ok(mt) => mt,
        Err(error_msg) => {
            return serde_json::json!({
                "metric": metric,
                "timeframe": timeframe,
                "trend": "invalid_metric",
                "activities_analyzed": 0,
                "insights": [error_msg]
            });
        }
    };

    // Filter activities by timeframe
    let cutoff_date = calculate_cutoff_date(timeframe);
    let filtered_activities: Vec<Activity> = activities
        .iter()
        .filter(|a| a.start_date() >= cutoff_date)
        .cloned()
        .collect();

    if filtered_activities.len() < 2 {
        return serde_json::json!({
            "metric": metric,
            "timeframe": timeframe,
            "trend": "needs_more_data",
            "activities_analyzed": filtered_activities.len(),
            "insights": [format!("Need at least 2 activities for trend analysis. Found {}", filtered_activities.len())]
        });
    }

    // Extract metric values using SafeMetricExtractor
    let Ok(data_points_with_timestamp) =
        SafeMetricExtractor::extract_metric_values(&filtered_activities, metric_type)
    else {
        return serde_json::json!({
            "metric": metric,
            "timeframe": timeframe,
            "trend": "insufficient_data",
            "activities_analyzed": filtered_activities.len(),
            "insights": [format!("Metric '{metric}' not available in enough activities")]
        });
    };

    if data_points_with_timestamp.len() < 2 {
        return serde_json::json!({
            "metric": metric,
            "timeframe": timeframe,
            "trend": "insufficient_data",
            "activities_analyzed": filtered_activities.len(),
            "insights": [format!("Metric '{metric}' not available in enough activities")]
        });
    }

    // Convert to TrendDataPoint format and perform regression
    compute_trend_statistics(metric, timeframe, metric_type, &data_points_with_timestamp)
}

/// Compute trend statistics from data points
fn compute_trend_statistics(
    metric: &str,
    timeframe: &str,
    metric_type: MetricType,
    data_points_with_timestamp: &[(chrono::DateTime<chrono::Utc>, f64)],
) -> serde_json::Value {
    // Convert to TrendDataPoint format
    let trend_data_points: Vec<TrendDataPoint> = data_points_with_timestamp
        .iter()
        .map(|(date, value)| TrendDataPoint {
            date: *date,
            value: *value,
            smoothed_value: None,
        })
        .collect();

    // Perform linear regression using StatisticalAnalyzer
    let Ok(regression_result) = StatisticalAnalyzer::linear_regression(&trend_data_points) else {
        return serde_json::json!({
            "metric": metric,
            "timeframe": timeframe,
            "trend": "calculation_error",
            "activities_analyzed": trend_data_points.len(),
            "insights": ["Unable to calculate trend statistics"]
        });
    };

    // Calculate simple average for comparison
    let sum: f64 = data_points_with_timestamp.iter().map(|(_, v)| v).sum();
    // Cast is safe: data point count far below f64 precision limit (2^53)
    #[allow(clippy::cast_precision_loss)] // Safe: realistic data point counts
    let moving_avg = sum / data_points_with_timestamp.len() as f64;

    // Determine trend direction using proper logic
    let slope_threshold = 0.01;
    let trend_direction_enum = StatisticalAnalyzer::determine_trend_direction(
        &regression_result,
        metric_type.is_lower_better(),
        slope_threshold,
    );

    let trend_direction = match trend_direction_enum {
        TrendDirection::Improving => "improving",
        TrendDirection::Stable => "stable",
        TrendDirection::Declining => "declining",
    };

    // Generate insights
    let insights = generate_trend_insights(
        metric,
        trend_direction,
        regression_result.slope,
        regression_result.r_squared,
        data_points_with_timestamp,
    );

    serde_json::json!({
        "metric": metric,
        "timeframe": timeframe,
        "trend": trend_direction,
        "activities_analyzed": data_points_with_timestamp.len(),
        "statistics": {
            "slope": regression_result.slope,
            "r_squared": regression_result.r_squared,
            "confidence": regression_result.r_squared,
            "correlation": regression_result.correlation,
            "standard_error": regression_result.standard_error,
            "p_value": regression_result.p_value,
            "moving_average_7day": moving_avg,
            "start_value": data_points_with_timestamp.first().map(|(_, v)| v),
            "end_value": data_points_with_timestamp.last().map(|(_, v)| v),
            "percent_change": calculate_percent_change(data_points_with_timestamp),
        },
        "insights": insights,
    })
}

/// Parse metric string to `MetricType`
fn parse_metric_type(metric: &str) -> Result<MetricType, String> {
    use MetricType;
    match metric.to_lowercase().as_str() {
        "pace" => Ok(MetricType::Pace),
        "speed" => Ok(MetricType::Speed),
        "heart_rate" | "hr" => Ok(MetricType::HeartRate),
        "distance" => Ok(MetricType::Distance),
        "duration" => Ok(MetricType::Duration),
        "elevation" => Ok(MetricType::Elevation),
        "power" => Ok(MetricType::Power),
        _ => Err(format!("Unknown metric type: {metric}")),
    }
}

/// Calculate cutoff date based on timeframe
fn calculate_cutoff_date(timeframe: &str) -> chrono::DateTime<chrono::Utc> {
    use chrono::{Duration, Utc};

    let now = Utc::now();
    match timeframe {
        "week" => now - Duration::days(7),
        "quarter" => now - Duration::days(90),
        "year" => now - Duration::days(365),
        _ => now - Duration::days(30), // default to month
    }
}

/// Calculate percent change between first and last data point
fn calculate_percent_change(data: &[(chrono::DateTime<chrono::Utc>, f64)]) -> Option<f64> {
    if data.len() < 2 {
        return None;
    }

    let first = data.first()?.1;
    let last = data.last()?.1;

    if first.abs() < f64::EPSILON {
        return None;
    }

    Some(((last - first) / first) * 100.0)
}

/// Generate insights from trend analysis
fn generate_trend_insights(
    metric: &str,
    trend: &str,
    slope: f64,
    r_squared: f64,
    data: &[(chrono::DateTime<chrono::Utc>, f64)],
) -> Vec<String> {
    let mut insights = Vec::new();

    if let (Some(last), Some(first)) = (data.last(), data.first()) {
        insights.push(format!(
            "Analyzed {} data points over {} days",
            data.len(),
            (last.0 - first.0).num_days()
        ));
    } else {
        insights.push(format!("Analyzed {} data points", data.len()));
    }

    match trend {
        "improving" => {
            insights.push(format!(
                "Your {} is improving with {:.1}% confidence",
                metric,
                r_squared * 100.0
            ));
            if r_squared > 0.7 {
                insights.push("Strong consistent improvement trend detected".to_owned());
            }
        }
        "declining" => {
            insights.push(format!(
                "Your {} is declining with {:.1}% confidence",
                metric,
                r_squared * 100.0
            ));
            if slope < -0.05 {
                insights.push("Consider reviewing your training plan or recovery".to_owned());
            }
        }
        "stable" => {
            if r_squared < 0.3 {
                insights.push(
                    "Performance is variable - maintain consistency for clearer trends".to_owned(),
                );
            } else {
                insights.push(format!("Your {metric} is maintaining steady performance"));
            }
        }
        _ => {}
    }

    if let Some(percent_change) = calculate_percent_change(data) {
        insights.push(format!(
            "Overall change: {percent_change:.1}% from start to end"
        ));
    }

    insights
}

/// Handle `analyze_performance_trends` tool - analyze performance over time
#[must_use]
pub fn handle_analyze_performance_trends(
    executor: &UniversalToolExecutor,
    request: UniversalRequest,
) -> Pin<Box<dyn Future<Output = Result<UniversalResponse, ProtocolError>> + Send + '_>> {
    Box::pin(async move {
        use parse_user_id_for_protocol;

        // Check cancellation at start
        if let Some(token) = &request.cancellation_token {
            if token.is_cancelled().await {
                return Err(ProtocolError::OperationCancelled(
                    "analyze_performance_trends cancelled by user".to_owned(),
                ));
            }
        }

        let provider_name = request
            .parameters
            .get("provider")
            .and_then(|v| v.as_str())
            .map_or_else(default_provider, String::from);
        let user_uuid = parse_user_id_for_protocol(&request.user_id)?;
        let metric = request
            .parameters
            .get("metric")
            .and_then(|v| v.as_str())
            .unwrap_or("pace");
        let timeframe = request
            .parameters
            .get("timeframe")
            .and_then(|v| v.as_str())
            .unwrap_or("month");

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
                    "analyze_performance_trends cancelled before authentication".to_owned(),
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
                        Some(
                            "Authenticated - fetching activities for trend analysis...".to_owned(),
                        ),
                    );
                }

                // Check cancellation before analysis
                if let Some(token) = &request.cancellation_token {
                    if token.is_cancelled().await {
                        return Err(ProtocolError::OperationCancelled(
                            "analyze_performance_trends cancelled before analysis".to_owned(),
                        ));
                    }
                }

                let result = fetch_and_analyze_trends(provider, metric, timeframe, user_uuid).await;

                // Report completion on success
                if result.success {
                    if let Some(reporter) = &request.progress_reporter {
                        reporter.report(
                            100.0,
                            Some(100.0),
                            Some("Performance trend analysis completed".to_owned()),
                        );
                    }
                }

                // Apply format transformation
                Ok(apply_format_to_response(result, "trends", output_format))
            }
            Err(response) => Ok(response),
        }
    })
}
