// ABOUTME: Handler for analyze_performance_trends tool using linear regression
// ABOUTME: Tracks metric trends (pace, speed, HR, distance) over configurable timeframes
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::implementations::analytics::output::{PerformanceTrendsResult, TrendStatistics};
use crate::implementations::stored_copy::{activities_or_stored, with_caveat, Reader, Served};
use crate::protocol::format::{apply_format_typed, extract_output_format};
use crate::protocol::provider_helpers::resolve_provider_for_request;
use crate::protocol::{UniversalRequest, UniversalResponse, UniversalToolExecutor};
use crate::protocols::ProtocolError;
use dravr_cageux::metrics_extractor::{MetricType, SafeMetricExtractor};
use dravr_cageux::physiological_constants::api_limits::MAX_ACTIVITY_LIMIT;
use dravr_cageux::statistical_analysis::StatisticalAnalyzer;
use dravr_cageux::types::{TrendDataPoint, TrendDirection};
use dravr_tronc::dates::Timestamp;
use pierre_core::models::Activity;
use pierre_core::uuid_utils::parse_user_id_for_protocol;
use pierre_formatters::OutputFormat;
use pierre_providers::core::FitnessProvider;
use pierre_providers::deduplication::{merge_duplicates, DedupConfig};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use tracing::warn;

/// Fetch activities and analyze performance trends
///
/// Retrieves recent activities from provider and performs trend analysis
/// for the specified metric and timeframe.
///
/// # Arguments
/// * `provider` - Configured fitness provider
/// * `metric` - Metric to analyze (e.g., "pace", "distance")
/// * `timeframe` - Analysis timeframe (e.g., "week", "month")
/// * `reader` - Whose read it is: their stored copy answers a capped read
///
/// # Returns
/// `UniversalResponse` with trend analysis or error
async fn fetch_and_analyze_trends(
    provider: Box<dyn FitnessProvider>,
    metric: &str,
    timeframe: &str,
    reader: Reader<'_>,
    output_format: OutputFormat,
) -> Result<UniversalResponse, ProtocolError> {
    use MAX_ACTIVITY_LIMIT;

    match activities_or_stored(reader, provider.as_ref(), MAX_ACTIVITY_LIMIT).await {
        Ok(Served {
            value: raw_activities,
            caveat,
        }) => {
            // Trend regression must see one row per session, otherwise duplicate
            // captures bias slope estimates toward the days that were
            // double-recorded.
            let (activities, _fragment_report) =
                merge_duplicates(raw_activities, &DedupConfig::default());
            let analysis = analyze_performance_trend(&activities, metric, timeframe);

            with_caveat(
                apply_format_typed(
                    UniversalResponse {
                        success: true,
                        result: None,
                        error: None,
                        metadata: Some({
                            let mut map = HashMap::new();
                            map.insert(
                                "user_id".to_owned(),
                                serde_json::Value::String(reader.user_id.to_string()),
                            );
                            map
                        }),
                    },
                    analysis,
                    output_format,
                ),
                caveat,
            )
        }
        Err(e) => {
            warn!(error = %e, "activity read for trends failed");
            Ok(UniversalResponse {
                success: false,
                result: None,
                error: Some(format!("Failed to fetch activities: {}", e.tool_message())),
                metadata: None,
            })
        }
    }
}

/// Analyze performance trend for a specific metric over time
fn analyze_performance_trend(
    activities: &[Activity],
    metric: &str,
    timeframe: &str,
) -> PerformanceTrendsResult {
    use SafeMetricExtractor;

    if activities.is_empty() {
        return PerformanceTrendsResult {
            metric: metric.to_owned(),
            timeframe: timeframe.to_owned(),
            trend: "no_data".to_owned(),
            activities_analyzed: 0,
            statistics: None,
            insights: vec!["No activities found for analysis".to_owned()],
        };
    }

    // Parse metric string to MetricType
    let metric_type = match parse_metric_type(metric) {
        Ok(mt) => mt,
        Err(error_msg) => {
            return PerformanceTrendsResult {
                metric: metric.to_owned(),
                timeframe: timeframe.to_owned(),
                trend: "invalid_metric".to_owned(),
                activities_analyzed: 0,
                statistics: None,
                insights: vec![error_msg],
            };
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
        return PerformanceTrendsResult {
            metric: metric.to_owned(),
            timeframe: timeframe.to_owned(),
            trend: "needs_more_data".to_owned(),
            activities_analyzed: filtered_activities.len(),
            statistics: None,
            insights: vec![format!(
                "Need at least 2 activities for trend analysis. Found {}",
                filtered_activities.len()
            )],
        };
    }

    // Extract metric values using SafeMetricExtractor
    let Ok(data_points_with_timestamp) =
        SafeMetricExtractor::extract_metric_values(&filtered_activities, metric_type)
    else {
        return PerformanceTrendsResult {
            metric: metric.to_owned(),
            timeframe: timeframe.to_owned(),
            trend: "insufficient_data".to_owned(),
            activities_analyzed: filtered_activities.len(),
            statistics: None,
            insights: vec![format!(
                "Metric '{metric}' not available in enough activities"
            )],
        };
    };

    if data_points_with_timestamp.len() < 2 {
        return PerformanceTrendsResult {
            metric: metric.to_owned(),
            timeframe: timeframe.to_owned(),
            trend: "insufficient_data".to_owned(),
            activities_analyzed: filtered_activities.len(),
            statistics: None,
            insights: vec![format!(
                "Metric '{metric}' not available in enough activities"
            )],
        };
    }

    compute_trend_statistics(metric, timeframe, metric_type, data_points_with_timestamp)
}

/// Compute trend statistics from data points in any order.
///
/// The points are put in date order, oldest first, before anything reads
/// them: the regression takes their order as time's (cageux fits against the
/// point index), and the first and last points are the start and end the
/// change is measured between. Providers return activities newest-first, so
/// unsorted points read an improving athlete as declining.
fn compute_trend_statistics(
    metric: &str,
    timeframe: &str,
    metric_type: MetricType,
    mut data_points_with_timestamp: Vec<(Timestamp, f64)>,
) -> PerformanceTrendsResult {
    data_points_with_timestamp.sort_by_key(|&(date, _)| date);
    let data_points_with_timestamp = data_points_with_timestamp.as_slice();
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
        return PerformanceTrendsResult {
            metric: metric.to_owned(),
            timeframe: timeframe.to_owned(),
            trend: "calculation_error".to_owned(),
            activities_analyzed: trend_data_points.len(),
            statistics: None,
            insights: vec!["Unable to calculate trend statistics".to_owned()],
        };
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

    let trend_direction = trend_name(trend_direction_enum);

    // Generate insights
    let insights = generate_trend_insights(
        metric,
        trend_direction,
        regression_result.slope,
        regression_result.r_squared,
        data_points_with_timestamp,
    );

    PerformanceTrendsResult {
        metric: metric.to_owned(),
        timeframe: timeframe.to_owned(),
        trend: trend_direction.to_owned(),
        activities_analyzed: data_points_with_timestamp.len(),
        statistics: Some(TrendStatistics {
            slope: regression_result.slope,
            r_squared: regression_result.r_squared,
            correlation: regression_result.correlation,
            standard_error: regression_result.standard_error,
            p_value: regression_result.p_value,
            moving_average_7day: moving_avg,
            start_value: data_points_with_timestamp.first().map(|&(_, v)| v),
            end_value: data_points_with_timestamp.last().map(|&(_, v)| v),
            percent_change: calculate_percent_change(data_points_with_timestamp),
        }),
        insights,
    }
}

/// A trend direction's wire name, shared by the analytics tools that report one.
pub(super) const fn trend_name(trend: TrendDirection) -> &'static str {
    match trend {
        TrendDirection::Improving => "improving",
        TrendDirection::Stable => "stable",
        TrendDirection::Declining => "declining",
    }
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
fn calculate_cutoff_date(timeframe: &str) -> Timestamp {
    use dravr_tronc::dates::Elapsed;

    let now = Timestamp::now();
    match timeframe {
        "week" => now - Elapsed::days(7),
        "quarter" => now - Elapsed::days(90),
        "year" => now - Elapsed::days(365),
        _ => now - Elapsed::days(30), // default to month
    }
}

/// Calculate percent change between first and last data point
fn calculate_percent_change(data: &[(Timestamp, f64)]) -> Option<f64> {
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
    data: &[(Timestamp, f64)],
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
                "Your {metric} is improving; the trend line explains {:.0}% of the variation (R²)",
                r_squared * 100.0
            ));
            if r_squared > 0.7 {
                insights.push("Strong consistent improvement trend detected".to_owned());
            }
        }
        "declining" => {
            insights.push(format!(
                "Your {metric} is declining; the trend line explains {:.0}% of the variation (R²)",
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
pub(crate) fn handle_analyze_performance_trends(
    executor: &UniversalToolExecutor,
    request: UniversalRequest,
) -> Pin<Box<dyn Future<Output = Result<UniversalResponse, ProtocolError>> + Send + '_>> {
    Box::pin(async move {
        use parse_user_id_for_protocol;

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

        match executor
            .auth_service
            .create_authenticated_provider(&provider_name, user_uuid, request.tenant_id.as_deref())
            .await
        {
            Ok(provider) => {
                let result = fetch_and_analyze_trends(
                    provider,
                    metric,
                    timeframe,
                    Reader::new(&executor.resources, user_uuid, request.tenant_id.as_deref()),
                    output_format,
                )
                .await?;

                Ok(result)
            }
            Err(response) => Ok(*response),
        }
    })
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::*;
    use dravr_tronc::dates::Elapsed;

    #[test]
    fn newest_first_points_are_fitted_in_date_order() -> Result<(), Box<dyn Error>> {
        let now = Timestamp::now();
        let days_ago = |n: i64| now - Elapsed::days(n);
        // Speed rising over five weeks, with the noise real sessions carry (a
        // perfectly straight series has no p-value to judge), delivered
        // newest-first as providers do.
        let newest_first = vec![
            (days_ago(0), 3.4),
            (days_ago(7), 3.31),
            (days_ago(14), 3.27),
            (days_ago(21), 3.15),
            (days_ago(28), 3.12),
            (days_ago(35), 3.0),
        ];

        let result = compute_trend_statistics("speed", "month", MetricType::Speed, newest_first);

        assert_eq!(
            result.trend, "improving",
            "bug: rising speed fitted newest-first reads as a decline"
        );
        let statistics = result.statistics.ok_or("a fitted trend")?;
        assert!(
            statistics.slope > 0.0,
            "bug: the slope of rising speed is {}",
            statistics.slope
        );
        assert_eq!(
            statistics.start_value,
            Some(3.0),
            "bug: the start is not the oldest session"
        );
        assert_eq!(
            statistics.end_value,
            Some(3.4),
            "bug: the end is not the newest session"
        );
        assert!(
            statistics.percent_change.is_some_and(|change| change > 0.0),
            "bug: the change from oldest to newest is not a rise: {:?}",
            statistics.percent_change
        );
        assert!(
            result
                .insights
                .iter()
                .all(|line| !line.contains("confidence")),
            "bug: R² is presented as a confidence: {:?}",
            result.insights
        );
        Ok(())
    }

    #[test]
    fn a_faster_runner_reads_as_an_improving_pace() -> Result<(), Box<dyn Error>> {
        use pierre_core::models::{ActivityBuilder, SportType};

        let now = Timestamp::now();
        // Average speed rising over five weeks (with noise, as in real
        // sessions), delivered newest-first as providers do.
        let activities: Vec<Activity> = [
            (0, 3.4),
            (7, 3.31),
            (14, 3.27),
            (21, 3.15),
            (28, 3.12),
            (35, 3.0),
        ]
        .into_iter()
        .map(|(days_ago, speed)| {
            ActivityBuilder::new(
                format!("run-{days_ago}"),
                "Run",
                SportType::Run,
                now - Elapsed::days(days_ago),
                3600,
                "test_provider",
            )
            .average_speed(speed)
            .build()
        })
        .collect();

        let result = analyze_performance_trend(&activities, "pace", "quarter");

        // cageux reads pace as seconds per kilometre, lower being faster
        // (carnet#1804), so a runner whose speed rises reads as improving.
        assert_eq!(
            result.trend, "improving",
            "bug: a runner getting faster reads as a declining pace"
        );
        let statistics = result.statistics.ok_or("a fitted trend")?;
        let start = statistics.start_value.ok_or("a start")?;
        let end = statistics.end_value.ok_or("an end")?;
        assert!(
            (start - 1000.0 / 3.0).abs() < 1e-6,
            "bug: the oldest pace is not 3.0 m/s in s/km: {start}"
        );
        assert!(
            (end - 1000.0 / 3.4).abs() < 1e-6,
            "bug: the newest pace is not 3.4 m/s in s/km: {end}"
        );
        assert!(
            statistics.slope < 0.0,
            "bug: a faster pace has a rising slope"
        );
        Ok(())
    }
}
