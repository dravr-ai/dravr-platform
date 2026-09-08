// ABOUTME: Handler for generate_recommendations tool with AI and static analysis
// ABOUTME: Generates training plan, recovery, intensity, goal-specific, and nutrition recommendations
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::implementations::analytics::recommendations_output::{
    CorePrinciples, RacePredictionSummary, RecommendationMetrics, RecommendationsResult,
    TrainingSummary, WeekStructure,
};
use crate::protocol::format::{apply_format_typed, extract_output_format};
use crate::protocol::provider_helpers::resolve_provider_for_request;
use crate::protocol::{UniversalRequest, UniversalResponse, UniversalToolExecutor};
use crate::protocols::ProtocolError;
use crate::runtime::ToolRuntime;
use chrono::{Duration, Utc};
use pierre_config::constants::limits::METERS_PER_KILOMETER;
use pierre_core::civil_time::resolve_zone;
use pierre_core::errors::AppResult;
use pierre_core::models::Activity;
use pierre_core::uuid_utils::parse_user_id_for_protocol;
use pierre_intelligence::physiological_constants::api_limits::DEFAULT_ACTIVITY_LIMIT;
use pierre_intelligence::training_load::TrainingLoad;
use pierre_intelligence::{
    AlgorithmConfig, FormBand, PatternDetector, PerformancePredictor, RiskLevel,
    TrainingLoadCalculator,
};
use pierre_mcp_schema::{Content, CreateMessageRequest, ModelPreferences, PromptMessage};
use pierre_mcp_transport::sampling_peer::SamplingPeer;
use pierre_providers::deduplication::{dedupe_and_report, DedupConfig};

const ACTIVITY_SUMMARY_PLACEHOLDER: &str = "{activity_summary}";
const RECOMMENDATION_TYPE_PLACEHOLDER: &str = "{recommendation_type}";
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
    resources: &dyn ToolRuntime,
    activities: &[Activity],
    recommendation_type: &str,
) -> AppResult<RecommendationsResult> {
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

    // Validate the model's reply against the shape this tool declares, and
    // fall back when it does not fit. Passing an arbitrary `Value` through
    // made whatever the client's LLM emitted the tool's answer unchecked —
    // a contract no outputSchema could state, and third-party output landing
    // in an athlete's coaching context unread.
    Ok(sampled_or_wrapped(
        &result.content.text,
        recommendation_type,
    ))
}

/// Take the model's reply when it fits the declared shape, and wrap it as
/// prose when it does not.
///
/// Public within the crate so the wrapping has a test. The model is the one
/// input this tool does not control, and "it fit" versus "it did not" is the
/// whole of what an `outputSchema` promises here.
#[must_use]
pub fn sampled_or_wrapped(response_text: &str, recommendation_type: &str) -> RecommendationsResult {
    let mut sampled =
        serde_json::from_str::<RecommendationsResult>(response_text).unwrap_or_else(|_| {
            base_recommendations(
                recommendation_type,
                "medium",
                "Generated via MCP sampling".to_owned(),
                vec![response_text.to_owned()],
            )
        });
    // Ours either way: a model that names itself something else is not
    // allowed to hide that a model wrote this.
    sampled.source = Some("mcp_sampling".to_owned());
    sampled
}

/// A metrics block with nothing measured, for a mode to fill the fields it
/// does measure and leave the rest absent.
const fn empty_metrics() -> RecommendationMetrics {
    RecommendationMetrics {
        avg_activities_per_week: None,
        consistency_score: None,
        volume_spike_detected: None,
        ctl: None,
        atl: None,
        tsb: None,
        hr_drift_detected: None,
        risk_level: None,
        pattern_detected: None,
        pattern_description: None,
        hard_percentage: None,
        easy_percentage: None,
        adequate_recovery: None,
    }
}

/// The four fields every mode fills, so a new mode cannot forget one.
pub(super) fn base_recommendations(
    recommendation_type: &str,
    priority: &str,
    reasoning: String,
    recommendations: Vec<String>,
) -> RecommendationsResult {
    RecommendationsResult {
        recommendation_type: recommendation_type.to_owned(),
        priority: priority.to_owned(),
        reasoning,
        recommendations,
        source: None,
        suggested_structure: Vec::new(),
        recovery_status: None,
        recovery_actions: Vec::new(),
        intensity_guidance: Vec::new(),
        primary_sport: None,
        race_predictions: None,
        periodization_phases: Vec::new(),
        recovery_window: None,
        key_insights: Vec::new(),
        meal_suggestions: Vec::new(),
        macronutrient_targets: None,
        activity_summary: None,
        training_summary: None,
        core_principles: None,
        metrics: None,
    }
}

/// Generate personalized training recommendations
fn generate_training_recommendations(
    activities: &[Activity],
    recommendation_type: &str,
    algorithm_config: &AlgorithmConfig,
    user_timezone: Option<&str>,
) -> RecommendationsResult {
    if activities.is_empty() {
        return base_recommendations(
            recommendation_type,
            "medium",
            "No recent training data available".to_owned(),
            vec!["Start with 2-3 easy activities per week to build base fitness".to_owned()],
        );
    }

    // Filter to last 4 weeks for recommendation generation
    let four_weeks_ago = Utc::now() - Duration::days(28);
    let recent_activities: Vec<_> = activities
        .iter()
        .filter(|a| a.start_date() >= four_weeks_ago)
        .cloned()
        .collect();

    if recent_activities.is_empty() {
        return base_recommendations(
            recommendation_type,
            "high",
            "No training activity in the last 4 weeks".to_owned(),
            vec!["Resume training gradually - start with 2-3 easy sessions per week".to_owned()],
        );
    }

    match recommendation_type {
        "training_plan" => generate_training_plan_recommendations(
            &recent_activities,
            algorithm_config,
            user_timezone,
        ),
        "recovery" => generate_recovery_recommendations(&recent_activities, algorithm_config),
        "intensity" => generate_intensity_recommendations(&recent_activities),
        "goal_specific" => {
            generate_goal_specific_recommendations(&recent_activities, algorithm_config)
        }
        "nutrition" => {
            super::recommendations_nutrition::generate_nutrition_recommendations(&recent_activities)
        }
        _ => generate_comprehensive_recommendations(&recent_activities, algorithm_config),
    }
}

/// Generate weekly training plan recommendations using training load analysis
fn generate_training_plan_recommendations(
    activities: &[Activity],
    algorithm_config: &AlgorithmConfig,
    user_timezone: Option<&str>,
) -> RecommendationsResult {
    // Analyze volume progression to detect spikes
    let volume_pattern = PatternDetector::detect_volume_progression(activities);
    // The athlete's civil clock. Only the consistency score is read from this
    // schedule and a uniform shift leaves it alone — but an athlete who trains
    // some evenings at 19:00 and some at 21:00 has one local habit split across
    // two UTC days, which reads as inconsistency they do not have (registre#252).
    let weekly_schedule =
        PatternDetector::detect_weekly_schedule(activities, resolve_zone(user_timezone));

    // Sort oldest-first — EMA calculation requires chronological order
    let mut sorted = activities.to_vec();
    sorted.sort_by_key(Activity::start_date);

    // Calculate training load metrics
    let calculator = TrainingLoadCalculator::from_config(algorithm_config.clone());
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
        vec![WeekStructure {
            focus: "Build frequency".to_owned(),
            sessions_per_week_min: 3,
            sessions_per_week_max: 3,
            key_workouts: vec![
                "Easy run".to_owned(),
                "Tempo run".to_owned(),
                "Long run".to_owned(),
            ],
        }]
    } else if weekly_schedule.avg_activities_per_week <= 5.0 {
        vec![WeekStructure {
            focus: "Balanced training".to_owned(),
            sessions_per_week_min: 4,
            sessions_per_week_max: 4,
            key_workouts: vec![
                "2 easy runs".to_owned(),
                "1 quality session (intervals/tempo)".to_owned(),
                "1 long run".to_owned(),
            ],
        }]
    } else {
        vec![WeekStructure {
            focus: "High volume management".to_owned(),
            sessions_per_week_min: 5,
            sessions_per_week_max: 6,
            key_workouts: vec![
                "Mostly easy runs (80%)".to_owned(),
                "1-2 quality sessions".to_owned(),
                "1 long run".to_owned(),
            ],
        }]
    };

    RecommendationsResult {
        suggested_structure,
        metrics: Some(RecommendationMetrics {
            avg_activities_per_week: Some(weekly_schedule.avg_activities_per_week),
            consistency_score: Some(weekly_schedule.consistency_score),
            volume_spike_detected: Some(volume_pattern.volume_spikes_detected),
            ctl: training_load.as_ref().map(|l| l.ctl),
            atl: training_load.as_ref().map(|l| l.atl),
            ..empty_metrics()
        }),
        ..base_recommendations("training_plan", priority, reasoning, recommendations)
    }
}

/// Helper to process TSB status and add recommendations
fn process_tsb_recommendations(
    load: &TrainingLoad,
    recommendations: &mut Vec<String>,
    priority: &mut &str,
    recovery_status: &mut &str,
    reasoning: &mut String,
) -> FormBand {
    let band = FormBand::from_tsb(load.tsb, load.ctl);
    let recovery_days = TrainingLoadCalculator::recommend_recovery_days(load.tsb, load.ctl);

    match band {
        FormBand::InsufficientHistory => {
            recommendations.push(
                "Not enough chronic training history to read form yet - keep logging sessions"
                    .to_owned(),
            );
            *recovery_status = "insufficient_history";
            "CTL is too low to express TSB as a share of fitness, so form cannot be judged"
                .clone_into(reasoning);
        }
        FormBand::DeepFatigue => {
            recommendations.push(format!(
                "Form is deep relative to your fitness (TSB {:.1} on CTL {:.0}) - favor {recovery_days} lighter day(s)",
                load.tsb, load.ctl
            ));
            *priority = "high";
            *recovery_status = "deep_fatigue";
            *reasoning = format!(
                "TSB {:.1} is beyond -30% of CTL {:.0} - the deepest fatigue band relative to this athlete's own chronic load",
                load.tsb, load.ctl
            );
        }
        FormBand::HeavyBlock => {
            recommendations.push(
                "Deep end of a productive block - hold the load and keep recovery honest"
                    .to_owned(),
            );
            *recovery_status = "heavy_block";
        }
        FormBand::Productive | FormBand::Balanced => {
            recommendations
                .push("Good training zone - maintain current load with recovery days".to_owned());
            *recovery_status = "productive";
        }
        FormBand::Fresh => {
            recommendations.push("Well-recovered - ready for quality training".to_owned());
            *recovery_status = "fresh";
        }
        FormBand::Detraining => {
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

    band
}

/// Concrete recovery actions for each form band.
///
/// Keyed on [`FormBand`] rather than the `recovery_status` string so the two
/// cannot drift: a status this function does not recognise used to fall through
/// to the generic "stay hydrated" list, which is how deep-fatigue athletes were
/// handed maintenance advice next to `priority: "high"`.
const fn recovery_actions_for(band: FormBand) -> &'static [&'static str] {
    match band {
        FormBand::DeepFatigue => &[
            "Take complete rest days",
            "Focus on sleep quality (8-9 hours)",
            "Light stretching or yoga only",
            "Monitor resting heart rate daily",
        ],
        FormBand::HeavyBlock => &[
            "Protect the easy days - keep them genuinely easy",
            "Focus on sleep quality (8-9 hours)",
            "Hold the block, but add a recovery day before the next quality session",
        ],
        FormBand::Productive | FormBand::Balanced => &[
            "Include 1-2 easy recovery days per week",
            "Maintain 7-8 hours of sleep",
            "Active recovery (easy swimming/walking)",
        ],
        FormBand::Fresh | FormBand::Detraining | FormBand::InsufficientHistory => &[
            "Maintain current recovery routine",
            "7-9 hours of sleep per night",
            "Stay hydrated (2-3L water daily)",
        ],
    }
}

/// Generate recovery recommendations using TSB and overtraining signals
fn generate_recovery_recommendations(
    activities: &[Activity],
    algorithm_config: &AlgorithmConfig,
) -> RecommendationsResult {
    // Sort oldest-first — EMA calculation requires chronological order
    let mut sorted = activities.to_vec();
    sorted.sort_by_key(Activity::start_date);

    // Calculate TSB (Training Stress Balance)
    let calculator = TrainingLoadCalculator::from_config(algorithm_config.clone());
    let training_load = calculator
        .calculate_training_load(&sorted, None, None, None, None, None)
        .ok();

    // Detect overtraining signals
    let overtraining_signals = PatternDetector::detect_overtraining_signals(activities);

    let mut recommendations = Vec::new();
    let mut priority = "medium";
    let mut recovery_status = "unknown";
    let mut reasoning = String::from("Based on training stress balance analysis");

    // TSB-based recovery recommendations (highest priority). The band drives
    // both the narrative above and the concrete actions below, so they cannot
    // disagree about how fatigued the athlete is.
    let form_band = training_load
        .as_ref()
        .map_or(FormBand::InsufficientHistory, |load| {
            process_tsb_recommendations(
                load,
                &mut recommendations,
                &mut priority,
                &mut recovery_status,
                &mut reasoning,
            )
        });

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

    // Provide recovery-specific tips for the band the narrative just reported
    let recovery_actions = recovery_actions_for(form_band);

    RecommendationsResult {
        recovery_status: Some(recovery_status.to_owned()),
        recovery_actions: recovery_actions.iter().map(|a| (*a).to_owned()).collect(),
        metrics: Some(RecommendationMetrics {
            tsb: training_load.as_ref().map(|l| l.tsb),
            ctl: training_load.as_ref().map(|l| l.ctl),
            atl: training_load.as_ref().map(|l| l.atl),
            hr_drift_detected: Some(overtraining_signals.hr_drift_detected),
            risk_level: Some(
                match overtraining_signals.risk_level {
                    RiskLevel::Low => "low",
                    RiskLevel::Moderate => "moderate",
                    RiskLevel::High => "high",
                }
                .to_owned(),
            ),
            ..empty_metrics()
        }),
        ..base_recommendations("recovery", priority, reasoning, recommendations)
    }
}

/// Generate intensity recommendations using hard/easy pattern detection
fn generate_intensity_recommendations(activities: &[Activity]) -> RecommendationsResult {
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
        return base_recommendations(
            "intensity",
            "low",
            "Insufficient heart rate data for analysis".to_owned(),
            recommendations,
        );
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

    RecommendationsResult {
        intensity_guidance: intensity_guidance
            .into_iter()
            .map(ToOwned::to_owned)
            .collect(),
        metrics: Some(RecommendationMetrics {
            pattern_detected: Some(pattern.pattern_detected),
            pattern_description: Some(pattern.pattern_description.clone()),
            hard_percentage: Some(pattern.hard_percentage),
            easy_percentage: Some(pattern.easy_percentage),
            adequate_recovery: Some(pattern.adequate_recovery),
            ..empty_metrics()
        }),
        ..base_recommendations("intensity", priority, reasoning, recommendations)
    }
}

/// Generate goal-specific recommendations using performance prediction
fn generate_goal_specific_recommendations(
    activities: &[Activity],
    algorithm_config: &AlgorithmConfig,
) -> RecommendationsResult {
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
        if let Ok(predictions) =
            PerformancePredictor::generate_race_predictions(distance, time, algorithm_config)
        {
            race_predictions = Some(RacePredictionSummary {
                based_on: format!(
                    "{:.1}km in {}",
                    distance / METERS_PER_KILOMETER,
                    PerformancePredictor::format_time(time)
                ),
                vdot: predictions.vdot,
                race_times: predictions.predictions.clone(),
            });

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

    RecommendationsResult {
        primary_sport: Some(primary_sport.to_owned()),
        race_predictions,
        periodization_phases: vec![
            "Base Phase: Build aerobic foundation (4-8 weeks)".to_owned(),
            "Build Phase: Add tempo and threshold work (4-6 weeks)".to_owned(),
            "Peak Phase: Race-specific intensity (2-3 weeks)".to_owned(),
            "Taper: Reduce volume, maintain sharpness (1-2 weeks)".to_owned(),
        ],
        ..base_recommendations(
            "goal_specific",
            priority,
            "Based on recent performance and sport type".to_owned(),
            recommendations,
        )
    }
}

/// Generate comprehensive recommendations combining all analyses
fn generate_comprehensive_recommendations(
    activities: &[Activity],
    algorithm_config: &AlgorithmConfig,
) -> RecommendationsResult {
    // Sort oldest-first — EMA calculation requires chronological order
    let mut sorted = activities.to_vec();
    sorted.sort_by_key(Activity::start_date);

    // Comprehensive analysis using all available modules
    let calculator = TrainingLoadCalculator::from_config(algorithm_config.clone());
    let training_load = calculator
        .calculate_training_load(&sorted, None, None, None, None, None)
        .ok();

    let volume_pattern = PatternDetector::detect_volume_progression(activities);
    let intensity_pattern = PatternDetector::detect_hard_easy_pattern(activities);
    let overtraining = PatternDetector::detect_overtraining_signals(activities);

    let mut recommendations = Vec::new();
    let mut priority = "medium";
    let mut key_insights = Vec::new();

    // Training load insights. Form and chronic base are independent readings —
    // banding them in one else-if chain hid the fitness insight from exactly the
    // athletes carrying the most fatigue.
    if let Some(load) = &training_load {
        let form_pct = FormBand::form_pct(load.tsb, load.ctl);
        match FormBand::from_form_pct(form_pct) {
            FormBand::DeepFatigue => {
                if let Some(pct) = form_pct {
                    recommendations.push(format!(
                        "Form is {pct:.0}% of fitness - past the productive zone; favor recovery over added volume"
                    ));
                }
                priority = "high";
                key_insights.push("Fatigue is accumulating faster than fitness".to_owned());
            }
            FormBand::HeavyBlock => {
                key_insights
                    .push("Deep end of a productive block - keep recovery honest".to_owned());
            }
            _ => {}
        }

        if load.ctl > 80.0 {
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

    RecommendationsResult {
        key_insights,
        training_summary: Some(TrainingSummary {
            activities_analyzed: activities.len(),
            ctl: training_load.as_ref().map(|l| l.ctl),
            atl: training_load.as_ref().map(|l| l.atl),
            tsb: training_load.as_ref().map(|l| l.tsb),
            volume_spike_detected: volume_pattern.volume_spikes_detected,
            intensity_pattern_detected: intensity_pattern.pattern_detected,
            overtraining_signals: overtraining.hr_drift_detected
                || overtraining.performance_decline,
        }),
        core_principles: Some(CorePrinciples {
            consistency: "Regular training beats sporadic hard efforts".to_owned(),
            recovery: "Fitness improves during rest, not during training".to_owned(),
            progression: "Increase volume gradually (10% rule)".to_owned(),
            intensity: "Follow 80/20 rule (80% easy, 20% hard)".to_owned(),
        }),
        ..base_recommendations(
            "comprehensive",
            priority,
            "Holistic analysis of training load, volume, and intensity patterns".to_owned(),
            recommendations,
        )
    }
}

/// Handle `generate_recommendations` tool - generate training recommendations
#[must_use]
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
                    Ok(raw_activities) => {
                        // Recommendations are sensitive to consistency + volume
                        // — both are inflated by fragment duplicates. Collapse
                        // before recommendation logic runs.
                        let (activities, _fragment_report) =
                            dedupe_and_report(&raw_activities, &DedupConfig::default());
                        // Report progress before generating recommendations
                        if let Some(reporter) = &request.progress_reporter {
                            reporter.report(
                                70.0,
                                Some(100.0),
                                Some("Generating training recommendations...".to_owned()),
                            );
                        }

                        // The athlete's own zone, for the weekly-schedule
                        // histogram behind the consistency score (registre#252).
                        let user_timezone = executor
                            .resources
                            .repos()
                            .users
                            .get_global(user_uuid)
                            .await
                            .ok()
                            .flatten()
                            .and_then(|user| user.timezone);

                        // Try to use MCP sampling if available, otherwise use static analysis
                        let analysis = if let Some(sampling_peer) =
                            &executor.resources.sampling_peer()
                        {
                            // Use MCP sampling (client's LLM) to generate personalized recommendations
                            match generate_recommendations_via_sampling(
                                sampling_peer,
                                &*executor.resources,
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
                                        &executor.cageux_config().algorithms,
                                        user_timezone.as_deref(),
                                    )
                                }
                            }
                        } else {
                            // Fall back to static recommendations
                            generate_training_recommendations(
                                &activities,
                                recommendation_type,
                                &executor.cageux_config().algorithms,
                                user_timezone.as_deref(),
                            )
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
                        };

                        apply_format_typed(result, analysis, output_format)
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
