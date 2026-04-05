// ABOUTME: Handler for generate_recommendations tool with AI and static analysis
// ABOUTME: Generates training plan, recovery, intensity, goal-specific, and nutrition recommendations
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::config::environment::default_provider;
use crate::constants::limits::METERS_PER_KILOMETER;
use crate::constants::time_constants;
use crate::errors::AppResult;
use crate::intelligence::physiological_constants::api_limits::DEFAULT_ACTIVITY_LIMIT;
use crate::intelligence::training_load::TrainingLoad;
use crate::intelligence::{
    PatternDetector, PerformancePredictor, RiskLevel, TrainingLoadCalculator, TrainingStatus,
};
use crate::mcp::resources::ServerResources;
use crate::mcp::sampling_peer::SamplingPeer;
use crate::mcp::schema::{Content, CreateMessageRequest, ModelPreferences, PromptMessage};

const ACTIVITY_SUMMARY_PLACEHOLDER: &str = "{activity_summary}";
const RECOMMENDATION_TYPE_PLACEHOLDER: &str = "{recommendation_type}";
use crate::models::Activity;
use crate::protocols::universal::handlers::{apply_format_to_response, extract_output_format};
use crate::protocols::universal::{UniversalRequest, UniversalResponse, UniversalToolExecutor};
use crate::protocols::ProtocolError;
use crate::utils::uuid::parse_user_id_for_protocol;
use chrono::{Duration, Utc};
use std::cmp::Ordering;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tracing::warn;

/// Generate training recommendations via MCP sampling
///
/// Sends activity data to the client's LLM via MCP sampling for AI-powered coaching advice.
/// Returns natural language recommendations based on training patterns.
///
/// # Arguments
/// * `sampling_peer` - MCP sampling peer for LLM requests
/// * `activities` - Recent activity data
/// * `recommendation_type` - Type of recommendations requested
///
/// # Returns
/// JSON response with LLM-generated training recommendations
///
/// # Errors
/// Returns error if sampling request fails or response is invalid
async fn generate_recommendations_via_sampling(
    sampling_peer: &Arc<SamplingPeer>,
    resources: &ServerResources,
    activities: &[Activity],
    recommendation_type: &str,
) -> AppResult<serde_json::Value> {
    use {Content, CreateMessageRequest, ModelPreferences, PromptMessage};

    // Prepare activity summary for LLM analysis
    let activity_summary = if activities.is_empty() {
        "No recent training data available.".to_owned()
    } else {
        let recent_count = activities.len().min(10);
        let recent_activities = &activities[..recent_count];

        let total_distance: f64 = recent_activities
            .iter()
            .filter_map(Activity::distance_meters)
            .sum();
        let total_duration: u64 = recent_activities
            .iter()
            .map(Activity::duration_seconds)
            .sum();
        let activity_types: Vec<String> = recent_activities
            .iter()
            .map(|a| format!("{:?}", a.sport_type()))
            .collect();

        {
            #[allow(clippy::cast_precision_loss)]
            let duration_hours = total_duration as f64 / 3600.0;
            #[allow(clippy::cast_precision_loss)]
            let activities_per_week = recent_count as f64 / 4.0;

            format!(
                "Recent training data ({recent_count} activities):\n\
                 - Total distance: {:.2} km\n\
                 - Total duration: {duration_hours:.1} hours\n\
                 - Activity types: {}\n\
                 - Activities per week: {activities_per_week:.1}",
                total_distance / 1000.0,
                activity_types.join(", ")
            )
        }
    };

    // Create prompt for LLM from template
    let prompt = resources
        .recommendation_analysis_prompt()
        .replace(ACTIVITY_SUMMARY_PLACEHOLDER, &activity_summary)
        .replace(RECOMMENDATION_TYPE_PLACEHOLDER, recommendation_type);

    // Send sampling request to client's LLM
    let request = CreateMessageRequest {
        messages: vec![PromptMessage::user(Content::Text { text: prompt })],
        model_preferences: Some(ModelPreferences {
            // High intelligence priority - client decides actual model
            hints: None,
            intelligence_priority: Some(0.8),
            cost_priority: None,
            speed_priority: None,
        }),
        max_tokens: 1024,
        temperature: Some(0.7),
        system_prompt: Some(resources.recommendation_system_prompt().trim().to_owned()),
        include_context: None,
        stop_sequences: None,
        metadata: None,
    };

    let result = sampling_peer.create_message(request).await?;

    // Parse LLM response as JSON
    let response_text = &result.content.text;
    serde_json::from_str::<serde_json::Value>(response_text).or_else(|_| {
        // If LLM didn't return pure JSON, wrap the text in a response structure
        Ok(serde_json::json!({
            "recommendation_type": recommendation_type,
            "recommendations": [response_text],
            "priority": "medium",
            "reasoning": "Generated via MCP sampling",
            "source": "mcp_sampling"
        }))
    })
}

/// Generate personalized training recommendations
fn generate_training_recommendations(
    activities: &[Activity],
    recommendation_type: &str,
) -> serde_json::Value {
    if activities.is_empty() {
        return serde_json::json!({
            "recommendation_type": recommendation_type,
            "recommendations": ["Start with 2-3 easy activities per week to build base fitness"],
            "priority": "medium",
            "reasoning": "No recent training data available",
        });
    }

    // Filter to last 4 weeks for recommendation generation
    let four_weeks_ago = Utc::now() - Duration::days(28);
    let recent_activities: Vec<_> = activities
        .iter()
        .filter(|a| a.start_date() >= four_weeks_ago)
        .cloned()
        .collect();

    if recent_activities.is_empty() {
        return serde_json::json!({
            "recommendation_type": recommendation_type,
            "recommendations": ["Resume training gradually - start with 2-3 easy sessions per week"],
            "priority": "high",
            "reasoning": "No training activity in the last 4 weeks",
        });
    }

    match recommendation_type {
        "training_plan" => generate_training_plan_recommendations(&recent_activities),
        "recovery" => generate_recovery_recommendations(&recent_activities),
        "intensity" => generate_intensity_recommendations(&recent_activities),
        "goal_specific" => generate_goal_specific_recommendations(&recent_activities),
        "nutrition" => generate_nutrition_recommendations(&recent_activities),
        _ => generate_comprehensive_recommendations(&recent_activities),
    }
}

/// Generate weekly training plan recommendations using training load analysis
fn generate_training_plan_recommendations(activities: &[Activity]) -> serde_json::Value {
    // Analyze volume progression to detect spikes
    let volume_pattern = PatternDetector::detect_volume_progression(activities);
    let weekly_schedule = PatternDetector::detect_weekly_schedule(activities);

    // Sort oldest-first — EMA calculation requires chronological order
    let mut sorted = activities.to_vec();
    sorted.sort_by_key(Activity::start_date);

    // Calculate training load metrics
    let calculator = TrainingLoadCalculator::new();
    let training_load = calculator
        .calculate_training_load(&sorted, None, None, None, None, None)
        .ok();

    let mut recommendations = Vec::new();
    let mut priority = "medium";
    let reasoning = if volume_pattern.volume_spikes_detected {
        recommendations.push(
            "Volume spike detected - reduce next week's volume by 10-15% to prevent injury"
                .to_owned(),
        );
        priority = "high";
        format!(
            "Training volume increased rapidly (spike detected in weeks: {})",
            volume_pattern
                .spike_weeks
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        )
    } else {
        String::from("Based on volume and consistency analysis")
    };

    // Training load recommendations
    if let Some(load) = &training_load {
        if load.atl > 150.0 {
            recommendations
                .push("Acute training load is very high - schedule a recovery week".to_owned());
            priority = "high";
        } else if load.ctl < 40.0 {
            recommendations
                .push("Build fitness gradually - increase weekly volume by 5-10%".to_owned());
        } else if load.ctl > 100.0 {
            recommendations.push(
                "Strong fitness base - maintain current volume and add quality work".to_owned(),
            );
        }
    }

    // Consistency recommendations
    if weekly_schedule.consistency_score < 20.0 {
        recommendations
            .push("Training schedule is inconsistent - aim for same days each week".to_owned());
        if weekly_schedule.avg_activities_per_week < 3.0 {
            recommendations.push("Increase frequency to 3-4 activities per week".to_owned());
            if priority == "medium" {
                priority = "high";
            }
        }
    } else if weekly_schedule.avg_activities_per_week > 6.0 {
        recommendations
            .push("Very high training frequency - ensure at least 1 complete rest day".to_owned());
    }

    // Provide structured weekly plan based on consistency
    let suggested_structure = if weekly_schedule.avg_activities_per_week < 3.0 {
        vec![serde_json::json!({
            "focus": "Build frequency",
            "sessions_per_week": 3,
            "key_workouts": ["Easy run", "Tempo run", "Long run"],
        })]
    } else if weekly_schedule.avg_activities_per_week <= 5.0 {
        vec![serde_json::json!({
            "focus": "Balanced training",
            "sessions_per_week": 4,
            "key_workouts": ["2 easy runs", "1 quality session (intervals/tempo)", "1 long run"],
        })]
    } else {
        vec![serde_json::json!({
            "focus": "High volume management",
            "sessions_per_week": "5-6",
            "key_workouts": ["Mostly easy runs (80%)", "1-2 quality sessions", "1 long run"],
        })]
    };

    serde_json::json!({
        "recommendation_type": "training_plan",
        "priority": priority,
        "reasoning": reasoning,
        "recommendations": recommendations,
        "suggested_structure": suggested_structure,
        "metrics": {
            "avg_activities_per_week": weekly_schedule.avg_activities_per_week,
            "consistency_score": weekly_schedule.consistency_score,
            "volume_spike_detected": volume_pattern.volume_spikes_detected,
            "ctl": training_load.as_ref().map(|l| l.ctl),
            "atl": training_load.as_ref().map(|l| l.atl),
        },
    })
}

/// Helper to process TSB status and add recommendations
fn process_tsb_recommendations(
    load: &TrainingLoad,
    recommendations: &mut Vec<String>,
    priority: &mut &str,
    recovery_status: &mut &str,
    reasoning: &mut String,
) {
    let status = TrainingLoadCalculator::interpret_tsb(load.tsb);
    let recovery_days = TrainingLoadCalculator::recommend_recovery_days(load.tsb);

    match status {
        TrainingStatus::Overreaching => {
            recommendations.push(format!(
                "You're overreaching (TSB: {:.1}) - take {recovery_days} recovery days",
                load.tsb
            ));
            *priority = "high";
            *recovery_status = "overreaching";
            *reasoning = format!(
                "TSB is {:.1}, indicating deep fatigue requiring immediate recovery",
                load.tsb
            );
        }
        TrainingStatus::Productive => {
            recommendations
                .push("Good training zone - maintain current load with recovery days".to_owned());
            *recovery_status = "productive";
        }
        TrainingStatus::Fresh => {
            recommendations.push("Well-recovered - ready for quality training".to_owned());
            *recovery_status = "fresh";
        }
        TrainingStatus::Detraining => {
            recommendations
                .push("TSB is high - consider increasing training load gradually".to_owned());
            *recovery_status = "detraining_risk";
        }
    }

    // Check for overtraining risk
    let risk = TrainingLoadCalculator::check_overtraining_risk(load);
    if risk.risk_level == RiskLevel::High {
        *priority = "high";
        recommendations.push("High overtraining risk detected - prioritize recovery".to_owned());
        for factor in &risk.risk_factors {
            recommendations.push(format!("⚠️ {factor}"));
        }
    }
}

/// Generate recovery recommendations using TSB and overtraining signals
fn generate_recovery_recommendations(activities: &[Activity]) -> serde_json::Value {
    // Sort oldest-first — EMA calculation requires chronological order
    let mut sorted = activities.to_vec();
    sorted.sort_by_key(Activity::start_date);

    // Calculate TSB (Training Stress Balance)
    let calculator = TrainingLoadCalculator::new();
    let training_load = calculator
        .calculate_training_load(&sorted, None, None, None, None, None)
        .ok();

    // Detect overtraining signals
    let overtraining_signals = PatternDetector::detect_overtraining_signals(activities);

    let mut recommendations = Vec::new();
    let mut priority = "medium";
    let mut recovery_status = "unknown";
    let mut reasoning = String::from("Based on training stress balance analysis");

    // TSB-based recovery recommendations (highest priority)
    if let Some(load) = &training_load {
        process_tsb_recommendations(
            load,
            &mut recommendations,
            &mut priority,
            &mut recovery_status,
            &mut reasoning,
        );
    }

    // Overtraining signal detection
    if overtraining_signals.hr_drift_detected {
        if let Some(drift_pct) = overtraining_signals.hr_drift_percent {
            recommendations.push(format!(
                "Heart rate drift detected ({drift_pct:.1}% increase) - sign of fatigue"
            ));
            if priority == "medium" {
                priority = "high";
            }
        }
    }

    if overtraining_signals.performance_decline {
        recommendations
            .push("Performance declining despite training - increase recovery".to_owned());
    }

    if overtraining_signals.insufficient_recovery {
        recommendations
            .push("Insufficient recovery between hard sessions - add easy days".to_owned());
    }

    // Provide recovery-specific tips based on status
    let recovery_actions = match recovery_status {
        "overreaching" => vec![
            "Take complete rest days",
            "Focus on sleep quality (8-9 hours)",
            "Light stretching or yoga only",
            "Monitor resting heart rate daily",
        ],
        "productive" => vec![
            "Include 1-2 easy recovery days per week",
            "Maintain 7-8 hours of sleep",
            "Active recovery (easy swimming/walking)",
        ],
        _ => vec![
            "Maintain current recovery routine",
            "7-9 hours of sleep per night",
            "Stay hydrated (2-3L water daily)",
        ],
    };

    serde_json::json!({
        "recommendation_type": "recovery",
        "priority": priority,
        "reasoning": reasoning,
        "recovery_status": recovery_status,
        "recommendations": recommendations,
        "recovery_actions": recovery_actions,
        "metrics": {
            "tsb": training_load.as_ref().map(|l| l.tsb),
            "ctl": training_load.as_ref().map(|l| l.ctl),
            "atl": training_load.as_ref().map(|l| l.atl),
            "hr_drift_detected": overtraining_signals.hr_drift_detected,
            "risk_level": match overtraining_signals.risk_level {
                RiskLevel::Low => "low",
                RiskLevel::Moderate => "moderate",
                RiskLevel::High => "high",
            },
        },
    })
}

/// Generate intensity recommendations using hard/easy pattern detection
fn generate_intensity_recommendations(activities: &[Activity]) -> serde_json::Value {
    use PatternDetector;

    // Detect hard/easy pattern
    let pattern = PatternDetector::detect_hard_easy_pattern(activities);

    let mut recommendations = Vec::new();
    let mut priority = "medium";
    let mut reasoning = String::from("Based on intensity distribution analysis");

    // Check if pattern was detected
    if !pattern.pattern_detected {
        recommendations.push(
            "Unable to detect clear intensity pattern - ensure heart rate data is available"
                .to_owned(),
        );
        return serde_json::json!({
            "recommendation_type": "intensity",
            "priority": "low",
            "reasoning": "Insufficient heart rate data for analysis",
            "recommendations": recommendations,
        });
    }

    // Analyze 80/20 principle adherence
    let easy_pct = pattern.easy_percentage;

    if easy_pct < 70.0 {
        recommendations.push(
            "Too much high-intensity training - add more easy/recovery runs (aim for 80% easy)"
                .to_owned(),
        );
        priority = "high";
        reasoning = format!("Only {easy_pct:.0}% easy training detected - risk of overtraining");
    } else if easy_pct > 90.0 {
        recommendations.push(
            "Mostly easy training - include 1-2 quality sessions per week for fitness gains"
                .to_owned(),
        );
        priority = "medium";
    } else {
        recommendations.push("Good intensity balance following 80/20 principle".to_owned());
    }

    // Check recovery adequacy
    if pattern.adequate_recovery {
        recommendations.push("Good recovery pattern between hard sessions".to_owned());
    } else {
        recommendations.push("Consider adding more recovery days between hard sessions".to_owned());
    }

    // Specific workout recommendations based on hard percentage
    let hard_pct = pattern.hard_percentage;
    if hard_pct < 10.0 {
        recommendations.push("Add quality work:".to_owned());
        recommendations
            .push("  • Interval training: 6x800m @ 5K pace with 2min recovery".to_owned());
        recommendations.push("  • Tempo run: 20-30min @ comfortably hard pace".to_owned());
    } else if hard_pct > 30.0 {
        recommendations.push("Reduce high-intensity frequency to 1-2 sessions per week".to_owned());
        if priority != "high" {
            priority = "high";
        }
    }

    // Provide intensity zones guidance
    let intensity_guidance = if easy_pct < 70.0 {
        vec![
            "Most runs should be conversational pace",
            "Hard efforts should feel genuinely hard (8-9/10 effort)",
            "Recovery runs should be very easy (5-6/10 effort)",
        ]
    } else {
        vec![
            "Maintain mostly easy training",
            "Quality sessions: intervals, tempo, or threshold",
            "Allow 48h recovery after hard sessions",
        ]
    };

    serde_json::json!({
        "recommendation_type": "intensity",
        "priority": priority,
        "reasoning": reasoning,
        "recommendations": recommendations,
        "intensity_guidance": intensity_guidance,
        "metrics": {
            "pattern_detected": pattern.pattern_detected,
            "pattern_description": pattern.pattern_description,
            "hard_percentage": pattern.hard_percentage,
            "easy_percentage": pattern.easy_percentage,
            "adequate_recovery": pattern.adequate_recovery,
        },
    })
}

/// Generate goal-specific recommendations using performance prediction
fn generate_goal_specific_recommendations(activities: &[Activity]) -> serde_json::Value {
    use HashMap;
    use PerformancePredictor;

    // Detect primary sport
    let mut sport_counts: HashMap<String, usize> = HashMap::new();
    for activity in activities {
        let sport = format!("{:?}", activity.sport_type());
        *sport_counts.entry(sport).or_insert(0) += 1;
    }

    let primary_sport = sport_counts
        .iter()
        .max_by_key(|(_, count)| *count)
        .map_or("Unknown", |(sport, _)| sport.as_str());

    // Find best recent performance for predictions
    let best_performance = activities
        .iter()
        .filter(|a| a.distance_meters().is_some() && a.duration_seconds() > 0)
        .filter_map(|a| {
            let distance = a.distance_meters()?;
            #[allow(clippy::cast_precision_loss)]
            let time_secs = a.duration_seconds() as f64;
            if distance > 3_000.0 && distance < 50_000.0 && time_secs > 0.0 {
                Some((distance, time_secs))
            } else {
                None
            }
        })
        .max_by(|a, b| {
            let pace_a = a.1 / (a.0 / METERS_PER_KILOMETER);
            let pace_b = b.1 / (b.0 / METERS_PER_KILOMETER);
            pace_b.partial_cmp(&pace_a).unwrap_or(Ordering::Equal)
        });

    let mut recommendations = Vec::new();
    let priority = "medium";
    let mut race_predictions = None;

    // Generate race time predictions if we have performance data
    if let Some((distance, time)) = best_performance {
        if let Ok(predictions) = PerformancePredictor::generate_race_predictions(distance, time) {
            race_predictions = Some(serde_json::json!({
                "based_on": format!("{:.1}km in {}", distance / METERS_PER_KILOMETER, PerformancePredictor::format_time(time)),
                "vdot": predictions.vdot,
                "race_times": predictions.predictions,
            }));

            recommendations.push(format!(
                "Your VDOT is {:.1} - use this to set appropriate training paces",
                predictions.vdot
            ));
        }
    }

    // Sport-specific goal recommendations
    if primary_sport.contains("Run") {
        recommendations.push("Build aerobic base with easy long runs".to_owned());
        recommendations.push("Add weekly quality: tempo run or interval session".to_owned());
        recommendations.push("Include race-pace intervals 4-6 weeks before goal race".to_owned());
        recommendations.push("Taper 10-14 days before race: reduce volume 30-50%".to_owned());
    } else if primary_sport.contains("Ride") {
        recommendations.push("Build FTP with structured threshold intervals".to_owned());
        recommendations.push("Include weekly hill repeats for strength".to_owned());
        recommendations.push("Long endurance rides on weekends (3-5 hours)".to_owned());
    } else {
        recommendations.push("Focus on consistent training to build aerobic base".to_owned());
        recommendations.push("Gradually increase training volume by 5-10% per week".to_owned());
    }

    serde_json::json!({
        "recommendation_type": "goal_specific",
        "priority": priority,
        "reasoning": "Based on recent performance and sport type",
        "primary_sport": primary_sport,
        "recommendations": recommendations,
        "race_predictions": race_predictions,
        "periodization_phases": [
            "Base Phase: Build aerobic foundation (4-8 weeks)",
            "Build Phase: Add tempo and threshold work (4-6 weeks)",
            "Peak Phase: Race-specific intensity (2-3 weeks)",
            "Taper: Reduce volume, maintain sharpness (1-2 weeks)",
        ],
    })
}

/// Generate nutrition recommendations based on recent activity
/// Calculate activity nutrition metrics (duration, calories, intensity)
fn calculate_nutrition_metrics(activity: &Activity) -> (f64, f64, &'static str) {
    use time_constants;

    let duration_hours = f64::from(
        u32::try_from(activity.duration_seconds().min(u64::from(u32::MAX))).unwrap_or(u32::MAX),
    ) / time_constants::SECONDS_PER_HOUR_F64;

    let calories_burned = f64::from(activity.calories().unwrap_or_else(|| {
        let duration_mins = u32::try_from(activity.duration_seconds() / 60).unwrap_or(u32::MAX);
        duration_mins * 10
    }));

    let intensity = activity.average_heart_rate().map_or(
        if duration_hours > 1.5 {
            "moderate"
        } else {
            "low"
        },
        |avg_hr| {
            let avg_hr_f64 = f64::from(avg_hr);
            if avg_hr_f64 > 160.0 {
                "high"
            } else if avg_hr_f64 > 130.0 {
                "moderate"
            } else {
                "low"
            }
        },
    );

    (duration_hours, calories_burned, intensity)
}

/// Calculate macronutrient needs based on workout intensity and duration
fn calculate_macronutrient_needs(intensity: &str, duration_hours: f64) -> (f64, f64, f64) {
    let protein_g = if intensity == "high" || duration_hours > 1.5 {
        30.0 + (duration_hours * 5.0).min(20.0)
    } else {
        20.0 + (duration_hours * 5.0).min(15.0)
    };

    let carbs_g = duration_hours * 70.0;
    let hydration_ml = duration_hours * 750.0;

    (protein_g, carbs_g, hydration_ml)
}

/// Build meal suggestions based on workout intensity
fn build_meal_suggestions(intensity: &str) -> Vec<serde_json::Value> {
    let mut suggestions = vec![
        serde_json::json!({
            "option": "Quick Recovery Shake",
            "description": "Protein shake with banana and honey",
            "protein_g": 25,
            "carbs_g": 50,
            "timing": "Immediate (0-15 min)"
        }),
        serde_json::json!({
            "option": "Greek Yogurt Bowl",
            "description": "200g Greek yogurt with granola, berries, and honey",
            "protein_g": 20,
            "carbs_g": 60,
            "timing": "Within 30 minutes"
        }),
        serde_json::json!({
            "option": "Recovery Meal",
            "description": "Grilled chicken with sweet potato and vegetables",
            "protein_g": 35,
            "carbs_g": 50,
            "timing": "Within 2 hours"
        }),
    ];

    if intensity == "high" {
        suggestions.push(serde_json::json!({
            "option": "Endurance Option",
            "description": "Pasta with lean meat sauce and mixed salad",
            "protein_g": 30,
            "carbs_g": 80,
            "timing": "Within 2 hours"
        }));
    }

    suggestions
}

fn generate_nutrition_recommendations(activities: &[Activity]) -> serde_json::Value {
    let most_recent = activities.iter().max_by_key(|a| a.start_date());

    if most_recent.is_none() {
        return serde_json::json!({
            "recommendation_type": "nutrition",
            "priority": "medium",
            "reasoning": "No recent activity data available",
            "recommendations": [
                "Maintain balanced nutrition with adequate protein (1.6-2.2g/kg body weight)",
                "Stay hydrated throughout the day (2-3 liters water)",
                "Eat regular meals with complex carbohydrates, lean protein, and healthy fats"
            ],
        });
    }

    let Some(activity) = most_recent else {
        return serde_json::json!({
            "recommendations": ["No recent activities found for nutrition analysis"],
        });
    };

    let (duration_hours, calories_burned, intensity) = calculate_nutrition_metrics(activity);
    let (protein_g, carbs_g, hydration_ml) =
        calculate_macronutrient_needs(intensity, duration_hours);

    let mut recommendations = vec![
        format!(
            "Within 30 minutes: Consume {:.0}g protein and {:.0}g carbohydrates for optimal recovery",
            protein_g,
            carbs_g * 0.5
        ),
        format!(
            "Rehydrate with {:.0}-{:.0}ml of water or electrolyte drink",
            hydration_ml,
            hydration_ml * 1.3
        ),
    ];

    if intensity == "high" || duration_hours > 1.0 {
        recommendations.push(
            "Follow up with a complete meal within 2 hours to fully replenish glycogen stores"
                .to_owned(),
        );
    }

    let meal_suggestions = build_meal_suggestions(intensity);

    let mut key_insights = vec![
        format!(
            "Activity burned approximately {:.0} calories",
            calories_burned
        ),
        format!("Workout intensity: {intensity} - adjust nutrition accordingly"),
    ];

    if duration_hours > 1.5 {
        key_insights
            .push("Extended duration activity - prioritize carbohydrate replenishment".to_owned());
    }

    serde_json::json!({
        "recommendation_type": "nutrition",
        "priority": if intensity == "high" { "high" } else { "medium" },
        "reasoning": format!(
            "Based on {:.1} hour {intensity} intensity {:?} with {:.0} calories burned",
            duration_hours,
            activity.sport_type(),
            calories_burned
        ),
        "recovery_window": "Critical recovery period: 0-2 hours post-workout",
        "key_insights": key_insights,
        "recommendations": recommendations,
        "meal_suggestions": meal_suggestions,
        "macronutrient_targets": {
            "protein_g": protein_g.round(),
            "carbohydrates_g": carbs_g.round(),
            "hydration_ml": hydration_ml.round(),
        },
        "activity_summary": {
            "name": &activity.name(),
            "type": &activity.sport_type(),
            "duration_minutes": activity.duration_seconds() / 60,
            "distance_km": activity.distance_meters().map(|d| (d / 1000.0).round()),
            "calories": calories_burned.round(),
        }
    })
}

/// Generate comprehensive recommendations combining all analyses
fn generate_comprehensive_recommendations(activities: &[Activity]) -> serde_json::Value {
    // Sort oldest-first — EMA calculation requires chronological order
    let mut sorted = activities.to_vec();
    sorted.sort_by_key(Activity::start_date);

    // Comprehensive analysis using all available modules
    let calculator = TrainingLoadCalculator::new();
    let training_load = calculator
        .calculate_training_load(&sorted, None, None, None, None, None)
        .ok();

    let volume_pattern = PatternDetector::detect_volume_progression(activities);
    let intensity_pattern = PatternDetector::detect_hard_easy_pattern(activities);
    let overtraining = PatternDetector::detect_overtraining_signals(activities);

    let mut recommendations = Vec::new();
    let mut priority = "medium";
    let mut key_insights = Vec::new();

    // Training load insights
    if let Some(load) = &training_load {
        if load.tsb < -10.0 {
            recommendations.push(format!(
                "Immediate recovery needed - TSB is {:.1} (overreaching zone)",
                load.tsb
            ));
            priority = "high";
            key_insights.push("Fatigue is accumulating faster than fitness".to_owned());
        } else if load.ctl > 80.0 {
            key_insights.push(format!("Strong fitness base (CTL: {:.1})", load.ctl));
        } else if load.ctl < 40.0 {
            key_insights.push("Building fitness - continue gradual progression".to_owned());
        }
    }

    // Volume management
    if volume_pattern.volume_spikes_detected {
        recommendations
            .push("Reduce volume next week - spike detected in recent training".to_owned());
        if priority == "medium" {
            priority = "high";
        }
        key_insights.push("Training volume increased too rapidly".to_owned());
    }

    // Intensity balance
    if intensity_pattern.pattern_detected {
        let hard_pct = intensity_pattern.hard_percentage;
        if hard_pct > 30.0 {
            recommendations
                .push("Too much high-intensity work - add more easy training days".to_owned());
        } else if hard_pct < 10.0 {
            recommendations.push("Include 1-2 quality sessions per week".to_owned());
        }

        if intensity_pattern.adequate_recovery {
            key_insights.push("Good recovery pattern between hard sessions".to_owned());
        }
    }

    // Overtraining checks
    if overtraining.hr_drift_detected {
        recommendations
            .push("Heart rate drift detected - prioritize recovery this week".to_owned());
        key_insights.push("Possible fatigue accumulation detected".to_owned());
    }

    // General best practices if no specific issues
    if recommendations.is_empty() {
        recommendations.push("Training load is balanced - maintain current approach".to_owned());
        recommendations.push("Continue following 80/20 intensity distribution".to_owned());
        recommendations.push("Monitor weekly volume changes (keep under 10% increase)".to_owned());
    }

    // Add general best practices
    recommendations.push("Include 1-2 complete rest days per week".to_owned());
    recommendations.push("Prioritize sleep quality (7-9 hours per night)".to_owned());

    serde_json::json!({
        "recommendation_type": "comprehensive",
        "priority": priority,
        "reasoning": "Holistic analysis of training load, volume, and intensity patterns",
        "key_insights": key_insights,
        "recommendations": recommendations,
        "training_summary": {
            "activities_analyzed": activities.len(),
            "ctl": training_load.as_ref().map(|l| l.ctl),
            "atl": training_load.as_ref().map(|l| l.atl),
            "tsb": training_load.as_ref().map(|l| l.tsb),
            "volume_spike_detected": volume_pattern.volume_spikes_detected,
            "intensity_pattern_detected": intensity_pattern.pattern_detected,
            "overtraining_signals": overtraining.hr_drift_detected || overtraining.performance_decline,
        },
        "core_principles": {
            "consistency": "Regular training beats sporadic hard efforts",
            "recovery": "Fitness improves during rest, not during training",
            "progression": "Increase volume gradually (10% rule)",
            "intensity": "Follow 80/20 rule (80% easy, 20% hard)",
        },
    })
}

/// Handle `generate_recommendations` tool - generate training recommendations
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn handle_generate_recommendations(
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
                    "generate_recommendations cancelled by user".to_owned(),
                ));
            }
        }

        let provider_name = request
            .parameters
            .get("provider")
            .and_then(|v| v.as_str())
            .map_or_else(default_provider, String::from);
        let user_uuid = parse_user_id_for_protocol(&request.user_id)?;
        let recommendation_type = request
            .parameters
            .get("recommendation_type")
            .and_then(|v| v.as_str())
            .unwrap_or("all");

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
                    "generate_recommendations cancelled before authentication".to_owned(),
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
                            "generate_recommendations cancelled before fetch".to_owned(),
                        ));
                    }
                }

                match provider
                    .get_activities(Some(DEFAULT_ACTIVITY_LIMIT), None)
                    .await
                {
                    Ok(activities) => {
                        // Report progress before generating recommendations
                        if let Some(reporter) = &request.progress_reporter {
                            reporter.report(
                                70.0,
                                Some(100.0),
                                Some("Generating training recommendations...".to_owned()),
                            );
                        }

                        // Try to use MCP sampling if available, otherwise use static analysis
                        let analysis = if let Some(sampling_peer) =
                            &executor.resources.sampling_peer
                        {
                            // Use MCP sampling (client's LLM) to generate personalized recommendations
                            match generate_recommendations_via_sampling(
                                sampling_peer,
                                &executor.resources,
                                &activities,
                                recommendation_type,
                            )
                            .await
                            {
                                Ok(llm_recommendations) => llm_recommendations,
                                Err(e) => {
                                    warn!("MCP sampling failed, falling back to static recommendations: {}", e);
                                    generate_training_recommendations(
                                        &activities,
                                        recommendation_type,
                                    )
                                }
                            }
                        } else {
                            // Fall back to static recommendations
                            generate_training_recommendations(&activities, recommendation_type)
                        };

                        // Report completion
                        if let Some(reporter) = &request.progress_reporter {
                            reporter.report(
                                100.0,
                                Some(100.0),
                                Some("Recommendations generated successfully".to_owned()),
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
                                map
                            }),
                        };

                        // Apply format transformation
                        Ok(apply_format_to_response(
                            result,
                            "recommendations",
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
