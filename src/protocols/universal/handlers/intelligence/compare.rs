// ABOUTME: Handler for compare_activities tool supporting PR, similar, and specific comparisons
// ABOUTME: Compares metrics (pace, HR, elevation, power) between activities with insight generation
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::config::environment::default_provider;
use crate::constants::units::METERS_PER_KM;
use crate::errors::ErrorCode;
use crate::intelligence::physiological_constants::api_limits::DEFAULT_ACTIVITY_LIMIT;
use crate::models::Activity;
use crate::protocols::universal::handlers::{apply_format_to_response, extract_output_format};
use crate::protocols::universal::{UniversalRequest, UniversalResponse, UniversalToolExecutor};
use crate::protocols::ProtocolError;
use crate::providers::core::FitnessProvider;
use crate::utils::uuid::parse_user_id_for_protocol;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;

/// Execute activity comparison with authenticated provider
async fn execute_activity_comparison(
    provider: Box<dyn FitnessProvider>,
    activity_id: &str,
    comparison_type: &str,
    compare_activity_id: Option<&str>,
    user_uuid: uuid::Uuid,
    request: &UniversalRequest,
) -> UniversalResponse {
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

            let all_activities = provider
                .get_activities(Some(DEFAULT_ACTIVITY_LIMIT), None)
                .await
                .unwrap_or_default();

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

            UniversalResponse {
                success: true,
                result: Some(comparison),
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
        Err(e) => {
            let error_message = if e.code == ErrorCode::ResourceNotFound {
                format!(
                    "Activity '{activity_id}' not found. Please use get_activities to retrieve your activity IDs first, then use compare_activities with a valid ID from the list."
                )
            } else {
                format!("Failed to fetch activity {activity_id}: {e}")
            };

            UniversalResponse {
                success: false,
                result: None,
                error: Some(error_message),
                metadata: None,
            }
        }
    }
}

/// Compare an activity using different comparison strategies
fn compare_activity_logic(
    target: &Activity,
    all_activities: &[Activity],
    comparison_type: &str,
    compare_activity_id: Option<&str>,
) -> serde_json::Value {
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
) -> serde_json::Value {
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
        return serde_json::json!({
            "activity_id": target.id(),
            "comparison_type": "similar_activities",
            "comparison_count": 0,
            "insights": ["No similar activities found for comparison"],
        });
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
            comparisons.push(serde_json::json!({
                "metric": "pace",
                "current": target_p,
                "average": avg_p,
                "difference_percent": pace_diff_pct,
                "improved": pace_diff_pct < 0.0, // faster pace = lower value
            }));

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
            comparisons.push(serde_json::json!({
                "metric": "heart_rate",
                "current": target_h,
                "average": avg_h,
                "difference_percent": hr_diff_pct,
                "improved": hr_diff_pct < 0.0, // lower HR = better efficiency
            }));

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
            comparisons.push(serde_json::json!({
                "metric": "elevation_gain",
                "current": target_elev,
                "average": avg_elev,
                "difference_percent": elev_diff_pct,
            }));
        }
    }

    if insights.is_empty() {
        insights.push(format!(
            "Compared with {} similar activities",
            similar.len()
        ));
    }

    serde_json::json!({
        "activity_id": target.id(),
        "comparison_type": "similar_activities",
        "comparison_count": similar.len(),
        "sport_type": format!("{:?}", target.sport_type()),
        "comparisons": comparisons,
        "insights": insights,
    })
}

/// Compare activity with personal records
fn compare_with_personal_records(
    target: &Activity,
    all_activities: &[Activity],
) -> serde_json::Value {
    // Find same sport activities
    let same_sport: Vec<&Activity> = all_activities
        .iter()
        .filter(|a| a.sport_type() == target.sport_type())
        .collect();

    if same_sport.is_empty() {
        return serde_json::json!({
            "activity_id": target.id(),
            "comparison_type": "pr_comparison",
            "insights": ["No other activities of this sport type found"],
        });
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
            pr_comparisons.push(serde_json::json!({
                "metric": "distance",
                "current": distance,
                "personal_record": max_d,
                "is_record": is_pr,
                "percent_of_pr": (distance / max_d) * 100.0,
            }));

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
        pr_comparisons.push(serde_json::json!({
            "metric": "pace",
            "current": tp,
            "personal_record": bp,
            "is_record": is_pr,
        }));

        if is_pr && (bp - tp).abs() > 0.1 {
            insights.push("New pace PR! 🚀".to_owned());
        }
    }

    // Compare with highest power (if available)
    if let Some(power) = target.average_power() {
        let max_power = same_sport.iter().filter_map(|a| a.average_power()).max();

        if let Some(max_p) = max_power {
            let is_pr = power >= max_p;
            pr_comparisons.push(serde_json::json!({
                "metric": "average_power",
                "current": power,
                "personal_record": max_p,
                "is_record": is_pr,
            }));

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

    serde_json::json!({
        "activity_id": target.id(),
        "comparison_type": "pr_comparison",
        "sport_type": format!("{:?}", target.sport_type()),
        "pr_comparisons": pr_comparisons,
        "insights": insights,
    })
}

/// Compare activity with a specific activity by ID
fn compare_with_specific_activity(
    target: &Activity,
    all_activities: &[Activity],
    compare_id: &str,
) -> serde_json::Value {
    // Find the specific activity to compare with
    let compare_activity = all_activities.iter().find(|a| a.id() == compare_id);

    let Some(compare) = compare_activity else {
        return serde_json::json!({
            "activity_id": target.id(),
            "comparison_type": "specific_activity",
            "error": format!("Activity with ID '{compare_id}' not found"),
            "insights": [format!("Could not find activity '{compare_id}' for comparison")],
        });
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
            comparisons.push(serde_json::json!({
                "metric": "distance",
                "current": target_dist,
                "comparison": compare_dist,
                "difference_percent": dist_diff_pct,
            }));
        }
    }

    // Pace comparison
    if let (Some(target_p), Some(compare_p)) = (target_pace, compare_pace) {
        if compare_p > 0.0 {
            let pace_diff_pct = ((target_p - compare_p) / compare_p) * 100.0;
            comparisons.push(serde_json::json!({
                "metric": "pace",
                "current": target_p,
                "comparison": compare_p,
                "difference_percent": pace_diff_pct,
                "improved": pace_diff_pct < 0.0, // faster pace = lower value
            }));
            add_pace_insights(pace_diff_pct, &mut insights);
        }
    }

    // Heart rate comparison
    if let (Some(target_h), Some(compare_h)) = (target_hr, compare_hr) {
        if compare_h > 0.0 {
            let hr_diff_pct = ((target_h - compare_h) / compare_h) * 100.0;
            comparisons.push(serde_json::json!({
                "metric": "heart_rate",
                "current": target_h,
                "comparison": compare_h,
                "difference_percent": hr_diff_pct,
                "improved": hr_diff_pct < 0.0, // lower HR = better efficiency
            }));
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
        comparisons.push(serde_json::json!({
            "metric": "duration",
            "current": target.duration_seconds(),
            "comparison": compare.duration_seconds(),
            "difference_percent": duration_diff_pct,
        }));
    }

    // Elevation comparison
    if let (Some(target_elev), Some(compare_elev)) =
        (target.elevation_gain(), compare.elevation_gain())
    {
        if compare_elev > 0.0 {
            let elev_diff_pct = ((target_elev - compare_elev) / compare_elev) * 100.0;
            comparisons.push(serde_json::json!({
                "metric": "elevation_gain",
                "current": target_elev,
                "comparison": compare_elev,
                "difference_percent": elev_diff_pct,
            }));
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
            comparisons.push(serde_json::json!({
                "metric": "average_power",
                "current": target_power,
                "comparison": compare_power,
                "difference_percent": power_diff_pct,
                "improved": power_diff_pct > 0.0, // higher power = better
            }));
            add_power_insights(power_diff_pct, &mut insights);
        }
    }

    if insights.is_empty() {
        insights.push("Metrics are similar to the comparison activity".to_owned());
    }

    serde_json::json!({
        "activity_id": target.id(),
        "comparison_type": "specific_activity",
        "comparison_activity_id": compare_id,
        "comparison_activity_name": compare.name(),
        "sport_type": format!("{:?}", target.sport_type()),
        "comparisons": comparisons,
        "insights": insights,
    })
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

        let provider_name = request
            .parameters
            .get("provider")
            .and_then(|v| v.as_str())
            .map_or_else(default_provider, String::from);
        let user_uuid = parse_user_id_for_protocol(&request.user_id)?;
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
                )
                .await;

                // Apply format transformation
                Ok(apply_format_to_response(
                    result,
                    "comparison",
                    output_format,
                ))
            }
            Err(response) => Ok(response),
        }
    })
}
