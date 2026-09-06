// ABOUTME: Handler for compare_activities tool supporting PR, similar, and specific comparisons
// ABOUTME: Compares metrics (pace, HR, elevation, power) between activities with insight generation
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::implementations::analytics::output::{
    CompareActivitiesResult, MetricComparison, PersonalRecordComparison,
};
use crate::protocol::format::{apply_format_typed, extract_output_format};
use crate::protocol::provider_helpers::resolve_provider_for_request;
use crate::protocol::{UniversalRequest, UniversalResponse, UniversalToolExecutor};
use crate::protocols::ProtocolError;
use pierre_config::constants::units::METERS_PER_KM;
use pierre_core::errors::ErrorCode;
use pierre_core::models::Activity;
use pierre_core::uuid_utils::parse_user_id_for_protocol;
use pierre_formatters::OutputFormat;
use pierre_intelligence::physiological_constants::api_limits::DEFAULT_ACTIVITY_LIMIT;
use pierre_providers::core::FitnessProvider;
use pierre_providers::deduplication::{dedupe_and_report, DedupConfig};
use std::cmp::Ordering;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;

/// The fields a comparison mode does not fill.
///
/// `..empty_comparison()` at each construction site keeps every builder to
/// the fields its own mode actually sets, which is what makes the modes
/// legible side by side. `insights` is deliberately not defaulted: every
/// answer carries at least one, so a builder that forgot it should not
/// compile.
fn empty_comparison() -> CompareActivitiesResult {
    CompareActivitiesResult {
        activity_id: String::new(),
        comparison_type: String::new(),
        comparison_count: None,
        sport_type: None,
        comparison_activity_id: None,
        comparison_activity_name: None,
        comparisons: None,
        pr_comparisons: None,
        error: None,
        insights: Vec::new(),
    }
}

/// Execute activity comparison with authenticated provider
async fn execute_activity_comparison(
    provider: Box<dyn FitnessProvider>,
    activity_id: &str,
    comparison_type: &str,
    compare_activity_id: Option<&str>,
    user_uuid: uuid::Uuid,
    request: &UniversalRequest,
    output_format: OutputFormat,
) -> Result<UniversalResponse, ProtocolError> {
    use DEFAULT_ACTIVITY_LIMIT;

    match provider.get_activity(activity_id).await {
        Ok(target_activity) => {
            // Report progress after getting target activity
            if let Some(reporter) = &request.progress_reporter {
                reporter.report(
                    66.0,
                    Some(100.0),
                    Some("Target activity retrieved - comparing...".to_owned()),
                );
            }

            let raw_activities = provider
                .get_activities(Some(DEFAULT_ACTIVITY_LIMIT), None)
                .await
                .unwrap_or_default();
            // Compare against canonical sessions only — comparing a workout
            // against its own fragments is degenerate and surfaces "0% delta"
            // noise.
            let (all_activities, _fragment_report) =
                dedupe_and_report(&raw_activities, &DedupConfig::default());

            let comparison = compare_activity_logic(
                &target_activity,
                &all_activities,
                comparison_type,
                compare_activity_id,
            );

            // Report completion
            if let Some(reporter) = &request.progress_reporter {
                reporter.report(
                    100.0,
                    Some(100.0),
                    Some("Comparison completed successfully".to_owned()),
                );
            }

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
                comparison,
                output_format,
            )
        }
        Err(e) => {
            let error_message = if e.code == ErrorCode::ResourceNotFound {
                format!(
                    "Activity '{activity_id}' not found. Please use get_activities to retrieve your activity IDs first, then use compare_activities with a valid ID from the list."
                )
            } else {
                format!("Failed to fetch activity {activity_id}: {e}")
            };

            Ok(UniversalResponse {
                success: false,
                result: None,
                error: Some(error_message),
                metadata: None,
            })
        }
    }
}

/// Compare an activity using different comparison strategies
fn compare_activity_logic(
    target: &Activity,
    all_activities: &[Activity],
    comparison_type: &str,
    compare_activity_id: Option<&str>,
) -> CompareActivitiesResult {
    match comparison_type {
        "pr_comparison" => compare_with_personal_records(target, all_activities),
        "specific_activity" => compare_activity_id.map_or_else(
            || compare_with_similar_activities(target, all_activities),
            |compare_id| compare_with_specific_activity(target, all_activities, compare_id),
        ),
        _ => compare_with_similar_activities(target, all_activities),
    }
}

/// Compare activity with similar past activities
fn compare_with_similar_activities(
    target: &Activity,
    all_activities: &[Activity],
) -> CompareActivitiesResult {
    // Find similar activities (same sport, similar distance/duration)
    let similar: Vec<&Activity> = all_activities
        .iter()
        .filter(|a| {
            a.id() != target.id()
                && a.sport_type() == target.sport_type()
                && is_similar_distance(a.distance_meters(), target.distance_meters())
        })
        .take(5)
        .collect();

    if similar.is_empty() {
        return CompareActivitiesResult {
            activity_id: target.id().to_owned(),
            comparison_type: "similar_activities".to_owned(),
            comparison_count: Some(0),
            insights: vec!["No similar activities found for comparison".to_owned()],
            ..empty_comparison()
        };
    }

    // Calculate average metrics from similar activities
    let avg_pace = calculate_average_pace(&similar);
    let avg_hr = calculate_average_hr(&similar);
    let avg_elevation = calculate_average_elevation(&similar);

    // Calculate target metrics
    let target_pace = calculate_pace(target);
    let target_hr = target.average_heart_rate().map(f64::from);

    // Generate comparisons
    let mut comparisons = Vec::new();
    let mut insights = Vec::new();

    if let (Some(target_p), Some(avg_p)) = (target_pace, avg_pace) {
        if avg_p > 0.0 {
            let pace_diff_pct = ((target_p - avg_p) / avg_p) * 100.0;
            // faster pace = lower value
            comparisons.push(MetricComparison {
                metric: "pace".to_owned(),
                current: target_p,
                average: Some(avg_p),
                comparison: None,
                difference_percent: pace_diff_pct,
                improved: Some(pace_diff_pct < 0.0),
            });

            if pace_diff_pct < -5.0 {
                insights.push(format!(
                    "Pace improved by {:.1}% compared to similar activities",
                    pace_diff_pct.abs()
                ));
            } else if pace_diff_pct > 5.0 {
                insights.push(format!(
                    "Pace was {pace_diff_pct:.1}% slower than similar activities"
                ));
            }
        }
    }

    if let (Some(target_h), Some(avg_h)) = (target_hr, avg_hr) {
        if avg_h > 0.0 {
            let hr_diff_pct = ((target_h - avg_h) / avg_h) * 100.0;
            // lower HR = better efficiency
            comparisons.push(MetricComparison {
                metric: "heart_rate".to_owned(),
                current: target_h,
                average: Some(avg_h),
                comparison: None,
                difference_percent: hr_diff_pct,
                improved: Some(hr_diff_pct < 0.0),
            });

            if hr_diff_pct < -5.0 {
                insights
                    .push("Heart rate efficiency improved - same effort at lower HR".to_owned());
            } else if hr_diff_pct > 5.0 {
                insights.push("Heart rate was higher - consider recovery or pacing".to_owned());
            }
        }
    }

    if let (Some(target_elev), Some(avg_elev)) = (target.elevation_gain(), avg_elevation) {
        if avg_elev > 0.0 {
            let elev_diff_pct = ((target_elev - avg_elev) / avg_elev) * 100.0;
            comparisons.push(MetricComparison {
                metric: "elevation_gain".to_owned(),
                current: target_elev,
                average: Some(avg_elev),
                comparison: None,
                difference_percent: elev_diff_pct,
                improved: None,
            });
        }
    }

    if insights.is_empty() {
        insights.push(format!(
            "Compared with {} similar activities",
            similar.len()
        ));
    }

    CompareActivitiesResult {
        activity_id: target.id().to_owned(),
        comparison_type: "similar_activities".to_owned(),
        comparison_count: Some(similar.len()),
        sport_type: Some(format!("{:?}", target.sport_type())),
        comparisons: Some(comparisons),
        insights,
        ..empty_comparison()
    }
}

/// Compare activity with personal records
fn compare_with_personal_records(
    target: &Activity,
    all_activities: &[Activity],
) -> CompareActivitiesResult {
    // Find same sport activities
    let same_sport: Vec<&Activity> = all_activities
        .iter()
        .filter(|a| a.sport_type() == target.sport_type())
        .collect();

    if same_sport.is_empty() {
        return CompareActivitiesResult {
            activity_id: target.id().to_owned(),
            comparison_type: "pr_comparison".to_owned(),
            insights: vec!["No other activities of this sport type found".to_owned()],
            ..empty_comparison()
        };
    }

    let mut pr_comparisons = Vec::new();
    let mut insights = Vec::new();

    // Compare with longest distance
    if let Some(distance) = target.distance_meters() {
        let max_distance = same_sport
            .iter()
            .filter_map(|a| a.distance_meters())
            .max_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));

        if let Some(max_d) = max_distance {
            let is_pr = distance >= max_d;
            pr_comparisons.push(PersonalRecordComparison {
                metric: "distance".to_owned(),
                current: distance,
                personal_record: max_d,
                is_record: is_pr,
                percent_of_pr: Some((distance / max_d) * 100.0),
            });

            if is_pr && (distance - max_d).abs() > 100.0 {
                insights.push("New distance PR! 🎉".to_owned());
            }
        }
    }

    // Compare with fastest pace
    let target_pace = calculate_pace(target);
    let best_pace = same_sport
        .iter()
        .filter_map(|a| calculate_pace(a))
        .min_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));

    if let (Some(tp), Some(bp)) = (target_pace, best_pace) {
        let is_pr = tp <= bp;
        pr_comparisons.push(PersonalRecordComparison {
            metric: "pace".to_owned(),
            current: tp,
            personal_record: bp,
            is_record: is_pr,
            percent_of_pr: None,
        });

        if is_pr && (bp - tp).abs() > 0.1 {
            insights.push("New pace PR! 🚀".to_owned());
        }
    }

    // Compare with highest power (if available)
    if let Some(power) = target.average_power() {
        let max_power = same_sport.iter().filter_map(|a| a.average_power()).max();

        if let Some(max_p) = max_power {
            let is_pr = power >= max_p;
            pr_comparisons.push(PersonalRecordComparison {
                metric: "average_power".to_owned(),
                current: f64::from(power),
                personal_record: f64::from(max_p),
                is_record: is_pr,
                percent_of_pr: None,
            });

            if is_pr && power > max_p {
                insights.push("New power PR! 💪".to_owned());
            }
        }
    }

    if insights.is_empty() {
        insights.push(format!(
            "Compared with {} activities in this sport",
            same_sport.len()
        ));
    }

    CompareActivitiesResult {
        activity_id: target.id().to_owned(),
        comparison_type: "pr_comparison".to_owned(),
        sport_type: Some(format!("{:?}", target.sport_type())),
        pr_comparisons: Some(pr_comparisons),
        insights,
        ..empty_comparison()
    }
}

/// Compare activity with a specific activity by ID
fn compare_with_specific_activity(
    target: &Activity,
    all_activities: &[Activity],
    compare_id: &str,
) -> CompareActivitiesResult {
    // Find the specific activity to compare with
    let compare_activity = all_activities.iter().find(|a| a.id() == compare_id);

    let Some(compare) = compare_activity else {
        return CompareActivitiesResult {
            activity_id: target.id().to_owned(),
            comparison_type: "specific_activity".to_owned(),
            error: Some(format!("Activity with ID '{compare_id}' not found")),
            insights: vec![format!(
                "Could not find activity '{compare_id}' for comparison"
            )],
            ..empty_comparison()
        };
    };

    // Calculate metrics for both activities
    let target_pace = calculate_pace(target);
    let compare_pace = calculate_pace(compare);
    let target_hr = target.average_heart_rate().map(f64::from);
    let compare_hr = compare.average_heart_rate().map(f64::from);

    let mut comparisons = Vec::new();
    let mut insights = Vec::new();

    // Distance comparison
    if let (Some(target_dist), Some(compare_dist)) =
        (target.distance_meters(), compare.distance_meters())
    {
        if compare_dist > 0.0 {
            let dist_diff_pct = ((target_dist - compare_dist) / compare_dist) * 100.0;
            comparisons.push(MetricComparison {
                metric: "distance".to_owned(),
                current: target_dist,
                average: None,
                comparison: Some(compare_dist),
                difference_percent: dist_diff_pct,
                improved: None,
            });
        }
    }

    // Pace comparison
    if let (Some(target_p), Some(compare_p)) = (target_pace, compare_pace) {
        if compare_p > 0.0 {
            let pace_diff_pct = ((target_p - compare_p) / compare_p) * 100.0;
            // faster pace = lower value
            comparisons.push(MetricComparison {
                metric: "pace".to_owned(),
                current: target_p,
                average: None,
                comparison: Some(compare_p),
                difference_percent: pace_diff_pct,
                improved: Some(pace_diff_pct < 0.0),
            });
            add_pace_insights(pace_diff_pct, &mut insights);
        }
    }

    // Heart rate comparison
    if let (Some(target_h), Some(compare_h)) = (target_hr, compare_hr) {
        if compare_h > 0.0 {
            let hr_diff_pct = ((target_h - compare_h) / compare_h) * 100.0;
            // lower HR = better efficiency
            comparisons.push(MetricComparison {
                metric: "heart_rate".to_owned(),
                current: target_h,
                average: None,
                comparison: Some(compare_h),
                difference_percent: hr_diff_pct,
                improved: Some(hr_diff_pct < 0.0),
            });
            add_heart_rate_insights(hr_diff_pct, &mut insights);
        }
    }

    // Duration comparison
    if compare.duration_seconds() > 0 {
        #[allow(clippy::cast_precision_loss)]
        let duration_diff_pct = ((target.duration_seconds() as f64
            - compare.duration_seconds() as f64)
            / compare.duration_seconds() as f64)
            * 100.0;
        #[allow(clippy::cast_precision_loss)]
        comparisons.push(MetricComparison {
            metric: "duration".to_owned(),
            current: target.duration_seconds() as f64,
            average: None,
            comparison: Some(compare.duration_seconds() as f64),
            difference_percent: duration_diff_pct,
            improved: None,
        });
    }

    // Elevation comparison
    if let (Some(target_elev), Some(compare_elev)) =
        (target.elevation_gain(), compare.elevation_gain())
    {
        if compare_elev > 0.0 {
            let elev_diff_pct = ((target_elev - compare_elev) / compare_elev) * 100.0;
            comparisons.push(MetricComparison {
                metric: "elevation_gain".to_owned(),
                current: target_elev,
                average: None,
                comparison: Some(compare_elev),
                difference_percent: elev_diff_pct,
                improved: None,
            });
        }
    }

    // Power comparison (if available)
    if let (Some(target_power), Some(compare_power)) =
        (target.average_power(), compare.average_power())
    {
        if compare_power > 0 {
            let power_diff_pct = ((f64::from(target_power) - f64::from(compare_power))
                / f64::from(compare_power))
                * 100.0;
            // higher power = better
            comparisons.push(MetricComparison {
                metric: "average_power".to_owned(),
                current: f64::from(target_power),
                average: None,
                comparison: Some(f64::from(compare_power)),
                difference_percent: power_diff_pct,
                improved: Some(power_diff_pct > 0.0),
            });
            add_power_insights(power_diff_pct, &mut insights);
        }
    }

    if insights.is_empty() {
        insights.push("Metrics are similar to the comparison activity".to_owned());
    }

    CompareActivitiesResult {
        activity_id: target.id().to_owned(),
        comparison_type: "specific_activity".to_owned(),
        comparison_activity_id: Some(compare_id.to_owned()),
        comparison_activity_name: Some(compare.name().to_owned()),
        sport_type: Some(format!("{:?}", target.sport_type())),
        comparisons: Some(comparisons),
        insights,
        ..empty_comparison()
    }
}

/// Helper to generate pace comparison insights
fn add_pace_insights(pace_diff_pct: f64, insights: &mut Vec<String>) {
    if pace_diff_pct < -5.0 {
        insights.push(format!(
            "Pace improved by {:.1}% compared to the selected activity",
            pace_diff_pct.abs()
        ));
    } else if pace_diff_pct > 5.0 {
        insights.push(format!(
            "Pace was {pace_diff_pct:.1}% slower than the selected activity"
        ));
    } else {
        insights.push("Pace was similar to the selected activity".to_owned());
    }
}

/// Helper to generate heart rate comparison insights
fn add_heart_rate_insights(hr_diff_pct: f64, insights: &mut Vec<String>) {
    if hr_diff_pct < -5.0 {
        insights.push("Heart rate efficiency improved - same effort at lower HR".to_owned());
    } else if hr_diff_pct > 5.0 {
        insights.push("Heart rate was higher - review pacing or recovery status".to_owned());
    }
}

/// Helper to generate power comparison insights
fn add_power_insights(power_diff_pct: f64, insights: &mut Vec<String>) {
    if power_diff_pct > 5.0 {
        insights.push(format!("Power output increased by {power_diff_pct:.1}%"));
    }
}

/// Check if two distances are similar (within 10%)
fn is_similar_distance(dist1: Option<f64>, dist2: Option<f64>) -> bool {
    match (dist1, dist2) {
        (Some(d1), Some(d2)) => {
            if d2 == 0.0 {
                return false;
            }
            let ratio = (d1 / d2 - 1.0).abs();
            ratio < 0.1 // within 10%
        }
        _ => false,
    }
}

/// Calculate pace in min/km
fn calculate_pace(activity: &Activity) -> Option<f64> {
    if let Some(distance) = activity.distance_meters() {
        if distance > 0.0 && activity.duration_seconds() > 0 {
            #[allow(clippy::cast_precision_loss)]
            let seconds_per_km = (activity.duration_seconds() as f64 / distance) * METERS_PER_KM;
            return Some(seconds_per_km / 60.0); // convert to min/km
        }
    }
    None
}

/// Calculate average pace from activities
fn calculate_average_pace(activities: &[&Activity]) -> Option<f64> {
    let paces: Vec<f64> = activities
        .iter()
        .filter_map(|a| calculate_pace(a))
        .collect();
    if paces.is_empty() {
        return None;
    }
    #[allow(clippy::cast_precision_loss)]
    let avg = paces.iter().sum::<f64>() / paces.len() as f64;
    Some(avg)
}

/// Calculate average heart rate from activities
fn calculate_average_hr(activities: &[&Activity]) -> Option<f64> {
    let hrs: Vec<f64> = activities
        .iter()
        .filter_map(|a| a.average_heart_rate().map(f64::from))
        .collect();
    if hrs.is_empty() {
        return None;
    }
    #[allow(clippy::cast_precision_loss)]
    let avg = hrs.iter().sum::<f64>() / hrs.len() as f64;
    Some(avg)
}

/// Calculate average elevation from activities
fn calculate_average_elevation(activities: &[&Activity]) -> Option<f64> {
    let elevs: Vec<f64> = activities
        .iter()
        .filter_map(|a| a.elevation_gain())
        .collect();
    if elevs.is_empty() {
        return None;
    }
    #[allow(clippy::cast_precision_loss)]
    let avg = elevs.iter().sum::<f64>() / elevs.len() as f64;
    Some(avg)
}

/// Handle `compare_activities` tool - compare two activities
#[must_use]
pub fn handle_compare_activities(
    executor: &UniversalToolExecutor,
    request: UniversalRequest,
) -> Pin<Box<dyn Future<Output = Result<UniversalResponse, ProtocolError>> + Send + '_>> {
    Box::pin(async move {
        use parse_user_id_for_protocol;

        // Check cancellation at start
        if let Some(token) = &request.cancellation_token {
            if token.is_cancelled().await {
                return Err(ProtocolError::OperationCancelled(
                    "compare_activities cancelled by user".to_owned(),
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
        let activity_id = request
            .parameters
            .get("activity_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ProtocolError::InvalidRequest("Missing required parameter: activity_id".to_owned())
            })?;
        let comparison_type = request
            .parameters
            .get("comparison_type")
            .and_then(|v| v.as_str())
            .unwrap_or("similar_activities");
        let compare_activity_id = request
            .parameters
            .get("compare_activity_id")
            .and_then(|v| v.as_str());

        // Extract output format parameter: "json" (default) or "toon"
        let output_format = extract_output_format(&request);

        match executor
            .auth_service
            .create_authenticated_provider(&provider_name, user_uuid, request.tenant_id.as_deref())
            .await
        {
            Ok(provider) => {
                // Report progress after auth
                if let Some(reporter) = &request.progress_reporter {
                    reporter.report(
                        33.0,
                        Some(100.0),
                        Some("Authenticated - fetching activities for comparison...".to_owned()),
                    );
                }

                let result = execute_activity_comparison(
                    provider,
                    activity_id,
                    comparison_type,
                    compare_activity_id,
                    user_uuid,
                    &request,
                    output_format,
                )
                .await?;

                Ok(result)
            }
            Err(response) => Ok(response),
        }
    })
}
