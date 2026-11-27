// ABOUTME: Handler for generate_recommendations tool with AI and static analysis
// ABOUTME: Generates training plan, recovery, intensity, goal-specific, and nutrition recommendations
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::implementations::analytics::recommendations_output::{
    CorePrinciples, RacePredictionSummary, RecommendationMetrics, RecommendationsResult,
    TrainingSummary, WeekStructure,
};
use crate::implementations::stored_copy::{activities_or_stored, with_caveat, Reader, Served};
use crate::implementations::stored_physiology::{stored_hr_zones, stored_profile};
use crate::protocol::format::{apply_format_typed, extract_output_format};
use crate::protocol::provider_helpers::resolve_provider_for_request;
use crate::protocol::{UniversalRequest, UniversalResponse, UniversalToolExecutor};
use crate::protocols::ProtocolError;
use crate::read_subject::{self, WellnessInput};
use dravr_cageux::config::intelligence::AlgorithmConfig;
use dravr_cageux::pattern_detection::PatternDetector;
use dravr_cageux::performance_prediction::{PerformancePredictor, VdotReading, VdotSource};
use dravr_cageux::physiological_constants::api_limits::DEFAULT_ACTIVITY_LIMIT;
use dravr_cageux::training_load::TrainingLoad;
use dravr_cageux::training_load::{FormBand, RiskLevel, TrainingLoadCalculator, TrainingLoadLevel};
use dravr_cageux::zones::HrZones;
use dravr_tronc::dates::civil_clock::resolve_zone;
use dravr_tronc::dates::{Elapsed, Timestamp, Zone};
use pierre_config::constants::units::METERS_PER_KM;
use pierre_core::models::zones::HrZonesUnavailable;
use pierre_core::models::{Activity, TenantId};
use pierre_core::uuid_utils::parse_user_id_for_protocol;
use pierre_fitness_compute::AthleteInputs;
use pierre_providers::deduplication::{merge_duplicates, DedupConfig};
use pierre_services::athlete_vdot::resolve_athlete_vdot;

use super::recommendations_nutrition::{
    generate_nutrition_recommendations, load_nutrition_athlete, NutritionAthlete,
};

use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::pin::Pin;
use tracing::warn;

/// What the recommendation modes read about the athlete beyond their
/// sessions, thresholds and zones.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AthleteReads {
    /// What the nutrition mode reads.
    pub nutrition: NutritionAthlete,
    /// The athlete's one VDOT, from a performance only
    /// (`pierre_services::athlete_vdot::resolve_athlete_vdot`), which the
    /// goal-specific mode predicts race times from; `None` when no
    /// performance gives one.
    pub vdot: Option<VdotReading>,
}

/// The modes `generate_recommendations` answers in, the default first.
const RECOMMENDATION_TYPES: [&str; 6] = [
    "all",
    "training_plan",
    "recovery",
    "intensity",
    "goal_specific",
    "nutrition",
];

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
        figures_withheld: None,
        training_summary: None,
        core_principles: None,
        metrics: None,
        hr_zones_unavailable: None,
    }
}

/// Generate personalized training recommendations.
///
/// The `training_plan`, `recovery` and `all` modes read CTL, ATL and TSB off
/// `athlete`, the thresholds saved with `set_physiology`, so the form they
/// advise on is the form `analyze_training_load` reports for the same
/// sessions. The `recovery`, `intensity`, `nutrition` and `all` modes read a
/// session's heart rate against `hr_zones`, the athlete's own zones; without
/// them heart rate decides nothing and the result's `hr_zones_unavailable`
/// says why. Public so the metrics each mode reports have content coverage;
/// the production caller is `handle_generate_recommendations`.
#[must_use]
pub fn generate_training_recommendations(
    activities: &[Activity],
    recommendation_type: &str,
    athlete: &AthleteInputs,
    hr_zones: Result<&HrZones, &HrZonesUnavailable>,
    algorithm_config: &AlgorithmConfig,
    user_timezone: Option<&str>,
    reads: &AthleteReads,
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
    let four_weeks_ago = Timestamp::now() - Elapsed::days(28);
    let recent_activities: Vec<_> = activities
        .iter()
        .filter(|a| a.start_date() >= four_weeks_ago)
        .cloned()
        .collect();

    // The latest session is what the nutrition mode advises on; binding it
    // here is also the emptiness check, so that mode cannot be reached
    // without one.
    let Some(latest) = recent_activities.iter().max_by_key(|a| a.start_date()) else {
        return base_recommendations(
            recommendation_type,
            "high",
            "No training activity in the last 4 weeks".to_owned(),
            vec!["Resume training gradually - start with 2-3 easy sessions per week".to_owned()],
        );
    };

    let zones = hr_zones.ok();
    // The athlete's civil clock: the day their training load is read as of,
    // and the days their sessions are counted on.
    let zone = resolve_zone(user_timezone);
    let mut result = match recommendation_type {
        "training_plan" => {
            return generate_training_plan_recommendations(
                &recent_activities,
                athlete,
                algorithm_config,
                &zone,
            )
        }
        "goal_specific" => {
            return generate_goal_specific_recommendations(
                &recent_activities,
                reads.vdot.as_ref(),
                algorithm_config,
            )
        }
        "recovery" => generate_recovery_recommendations(
            &recent_activities,
            athlete,
            zones,
            algorithm_config,
            &zone,
        ),
        "intensity" => generate_intensity_recommendations(&recent_activities, zones),
        "nutrition" => generate_nutrition_recommendations(latest, zones, &reads.nutrition),
        // `all`: the handler refuses any type outside `RECOMMENDATION_TYPES`
        // before a provider is read, so nothing else reaches this arm.
        _ => generate_comprehensive_recommendations(
            &recent_activities,
            athlete,
            zones,
            algorithm_config,
            &zone,
        ),
    };
    // Every mode left here read heart rate against the athlete's zones.
    result.hr_zones_unavailable = hr_zones.err().map(HrZonesUnavailable::reason);
    result
}

/// Generate weekly training plan recommendations using training load analysis
fn generate_training_plan_recommendations(
    activities: &[Activity],
    athlete: &AthleteInputs,
    algorithm_config: &AlgorithmConfig,
    zone: &Zone,
) -> RecommendationsResult {
    // Analyze volume progression to detect spikes
    let volume_pattern = PatternDetector::detect_volume_progression(activities);
    // The athlete's civil clock. Only the consistency score is read from this
    // schedule and a uniform shift leaves it alone — but an athlete who trains
    // some evenings at 19:00 and some at 21:00 has one local habit split across
    // two UTC days, which reads as inconsistency they do not have (registre#252).
    let weekly_schedule = PatternDetector::detect_weekly_schedule(activities, zone);

    // Sort oldest-first — EMA calculation requires chronological order
    let mut sorted = activities.to_vec();
    sorted.sort_by_key(Activity::start_date);

    // Calculate training load metrics
    let calculator = TrainingLoadCalculator::as_of_today(
        algorithm_config.clone(),
        zone.clone(),
        Timestamp::now(),
    );
    let training_load = athlete.training_load(calculator, &sorted).ok();

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
    let band = FormBand::from_training_load(load);
    let recovery_days = TrainingLoadCalculator::recommend_recovery_days(load);

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
                load.tsb, load.form_ctl
            ));
            *priority = "high";
            *recovery_status = "deep_fatigue";
            *reasoning = format!(
                "TSB {:.1} is beyond -30% of CTL {:.0} - the deepest fatigue band relative to this athlete's own chronic load",
                load.tsb, load.form_ctl
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
/// to the generic maintenance list, which is how deep-fatigue athletes were
/// handed maintenance advice next to `priority: "high"`.
#[must_use]
pub const fn recovery_actions_for(band: FormBand) -> &'static [&'static str] {
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
        // Fluid is advised by thirst, not a daily volume: no universal drinking
        // rate fits, because sweat and renal water loss vary widely between
        // athletes (Hew-Butler et al. 2008).
        FormBand::Fresh | FormBand::Detraining | FormBand::InsufficientHistory => &[
            "Maintain current recovery routine",
            "7-9 hours of sleep per night",
            "Drink to thirst through the day",
        ],
    }
}

/// Generate recovery recommendations using TSB and overtraining signals
fn generate_recovery_recommendations(
    activities: &[Activity],
    athlete: &AthleteInputs,
    hr_zones: Option<&HrZones>,
    algorithm_config: &AlgorithmConfig,
    zone: &Zone,
) -> RecommendationsResult {
    // Sort oldest-first — EMA calculation requires chronological order
    let mut sorted = activities.to_vec();
    sorted.sort_by_key(Activity::start_date);

    // Calculate TSB (Training Stress Balance)
    let calculator = TrainingLoadCalculator::as_of_today(
        algorithm_config.clone(),
        zone.clone(),
        Timestamp::now(),
    );
    let training_load = athlete.training_load(calculator, &sorted).ok();

    // Detect overtraining signals
    let overtraining_signals = PatternDetector::detect_overtraining_signals(activities, hr_zones);

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
fn generate_intensity_recommendations(
    activities: &[Activity],
    hr_zones: Option<&HrZones>,
) -> RecommendationsResult {
    use PatternDetector;

    // Detect hard/easy pattern, a session's heart rate read on the athlete's zones
    let pattern = PatternDetector::detect_hard_easy_pattern(activities, hr_zones);

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

/// What a goal-specific prediction's VDOT rests on, in words, with "an
/// estimate" said when it is one.
fn vdot_basis(reading: &VdotReading) -> String {
    match reading.source {
        VdotSource::Stated { as_of } => {
            format!("your stated VDOT, as of {}", as_of.utc_date())
        }
        VdotSource::RecentRun {
            distance_meters,
            time_seconds,
            ..
        } => format!(
            "an estimate from your best run of the last 6 weeks, {:.1}km in {}",
            distance_meters / METERS_PER_KM,
            PerformancePredictor::format_time(time_seconds)
        ),
        VdotSource::ThresholdPace {
            threshold_pace_sec_per_km,
        } => format!(
            "an estimate from your saved threshold pace, {}/km",
            PerformancePredictor::format_time(threshold_pace_sec_per_km)
        ),
    }
}

/// Generate goal-specific recommendations, with race predictions from the
/// athlete's one VDOT (`vdot`) when there is one
fn generate_goal_specific_recommendations(
    activities: &[Activity],
    vdot: Option<&VdotReading>,
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

    // Ties go to the alphabetically first sport, as in `SportProfile`: a
    // `HashMap`'s order is random per process, and the primary sport is
    // shown to the athlete.
    let primary_sport = sport_counts
        .iter()
        .max_by(|a, b| a.1.cmp(b.1).then_with(|| b.0.cmp(a.0)))
        .map_or("Unknown", |(sport, _)| sport.as_str());

    let mut recommendations = Vec::new();
    let priority = "medium";
    let mut race_predictions = None;

    // Race time predictions from the athlete's VDOT, read from a performance
    // only: a VDOT the race-time model has no times for predicts nothing.
    if let Some(reading) = vdot {
        let race_times: BTreeMap<String, f64> =
            PerformancePredictor::predictions_for_vdot(reading.vdot, algorithm_config)
                .into_iter()
                .collect();
        if !race_times.is_empty() {
            let basis = vdot_basis(reading);
            recommendations.push(format!(
                "Your VDOT is {:.1} ({basis}) - use this to set appropriate training paces",
                reading.vdot
            ));
            race_predictions = Some(RacePredictionSummary {
                based_on: basis,
                vdot: reading.vdot,
                race_times,
            });
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
    athlete: &AthleteInputs,
    hr_zones: Option<&HrZones>,
    algorithm_config: &AlgorithmConfig,
    zone: &Zone,
) -> RecommendationsResult {
    // Sort oldest-first — EMA calculation requires chronological order
    let mut sorted = activities.to_vec();
    sorted.sort_by_key(Activity::start_date);

    // Comprehensive analysis using all available modules
    let calculator = TrainingLoadCalculator::as_of_today(
        algorithm_config.clone(),
        zone.clone(),
        Timestamp::now(),
    );
    let training_load = athlete.training_load(calculator, &sorted).ok();

    let volume_pattern = PatternDetector::detect_volume_progression(activities);
    let intensity_pattern = PatternDetector::detect_hard_easy_pattern(activities, hr_zones);
    let overtraining = PatternDetector::detect_overtraining_signals(activities, hr_zones);

    let mut recommendations = Vec::new();
    let mut priority = "medium";
    let mut key_insights = Vec::new();

    // Training load insights. Form and chronic base are independent readings —
    // banding them in one else-if chain hid the fitness insight from exactly the
    // athletes carrying the most fatigue.
    if let Some(load) = &training_load {
        let form_pct = load.form_pct();
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

        // The chronic base is placed in cageux's CTL bands.
        match TrainingLoadLevel::from_ctl(load.ctl) {
            TrainingLoadLevel::Elite | TrainingLoadLevel::VeryHigh => {
                key_insights.push(format!("Strong fitness base (CTL: {:.1})", load.ctl));
            }
            TrainingLoadLevel::Beginner | TrainingLoadLevel::Intermediate => {
                key_insights.push("Building fitness - continue gradual progression".to_owned());
            }
            TrainingLoadLevel::Advanced => {}
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
            overtraining_signals: overtraining.insufficient_recovery,
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
pub(crate) fn handle_generate_recommendations(
    executor: &UniversalToolExecutor,
    request: UniversalRequest,
) -> Pin<Box<dyn Future<Output = Result<UniversalResponse, ProtocolError>> + Send + '_>> {
    Box::pin(async move {
        use parse_user_id_for_protocol;
        use DEFAULT_ACTIVITY_LIMIT;

        let user_uuid = parse_user_id_for_protocol(&request.user_id)?;
        // An omitted type is `all`. A type outside the vocabulary is refused,
        // before any provider is read, instead of being answered with every
        // mode as though `all` had been asked for.
        let recommendation_type = request
            .parameters
            .get("recommendation_type")
            .and_then(|v| v.as_str())
            .unwrap_or("all");
        if !RECOMMENDATION_TYPES.contains(&recommendation_type) {
            return Err(ProtocolError::InvalidParameters(format!(
                "recommendation_type '{recommendation_type}' is not a mode this tool generates; \
                 use one of {}",
                RECOMMENDATION_TYPES.join(", ")
            )));
        }
        // The nutrition mode scales its amounts from the athlete's body weight
        // and reads their medical flag: their wellness, which a coach's read
        // shows only under their wellness grant, on any transport
        // (carnet#1387). Refused before anything is read; every other mode
        // reads their training alone.
        let withheld = if recommendation_type == "nutrition" {
            read_subject::wellness_withheld()
        } else {
            None
        };
        if let Some(withheld) = withheld {
            return Ok(read_subject::wellness_refusal(
                "generate_recommendations",
                WellnessInput::Mode {
                    argument: "recommendation_type",
                    value: "nutrition",
                },
                &withheld,
            ));
        }
        // Under the grant, the nutrition mode's read of their wellness is on
        // the athlete's record before it is made (carnet#1445).
        if recommendation_type == "nutrition" {
            read_subject::record_analytics_wellness_read(
                executor.resources.repos(),
                "generate_recommendations",
            )
            .await?;
        }
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

        // Extract output format parameter: "json" (default) or "toon"
        let output_format = extract_output_format(&request);

        // Only the nutrition mode prescribes amounts, so only it reads the
        // medical-flag gate and the stored weight. A flag that cannot be read
        // fails the call rather than releasing the figures.
        let nutrition_athlete = if recommendation_type == "nutrition" {
            let tenant = request
                .tenant_id
                .as_deref()
                .and_then(|t| TenantId::parse_str(t).ok());
            match load_nutrition_athlete(executor.resources.repos(), tenant, user_uuid).await {
                Ok(athlete) => athlete,
                Err(e) => {
                    return Ok(UniversalResponse {
                        success: false,
                        result: None,
                        error: Some(format!(
                            "Failed to read the athlete's nutrition profile: {e}"
                        )),
                        metadata: None,
                    })
                }
            }
        } else {
            NutritionAthlete::default()
        };

        // The profile set_physiology saved: the thresholds every training-load
        // surface scores sessions against, and what the athlete's VDOT is
        // resolved from.
        let profile = stored_profile(&executor.resources, request.tenant_id.as_deref(), user_uuid)
            .await
            .map_err(|e| {
                ProtocolError::InternalError(format!(
                    "generate_recommendations: reading the saved thresholds failed: {e}"
                ))
            })?;
        let athlete = AthleteInputs::from_profile(profile.as_ref());
        // The athlete's heart-rate zones, which decide how hard a session was.
        let hr_zones =
            stored_hr_zones(&executor.resources, request.tenant_id.as_deref(), user_uuid)
                .await
                .map_err(|e| {
                    ProtocolError::InternalError(format!(
                        "generate_recommendations: reading the athlete's zones failed: {e}"
                    ))
                })?;

        match executor
            .auth_service
            .create_authenticated_provider(&provider_name, user_uuid, request.tenant_id.as_deref())
            .await
        {
            Ok(provider) => {
                match activities_or_stored(
                    Reader::new(&executor.resources, user_uuid, request.tenant_id.as_deref()),
                    provider.as_ref(),
                    DEFAULT_ACTIVITY_LIMIT,
                )
                .await
                {
                    Ok(Served {
                        value: raw_activities,
                        caveat,
                    }) => {
                        // Recommendations are sensitive to consistency + volume
                        // — both are inflated by fragment duplicates. Collapse
                        // before recommendation logic runs.
                        let (activities, _fragment_report) =
                            merge_duplicates(raw_activities, &DedupConfig::default());

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

                        let algorithms = &executor.cageux_config().algorithms;
                        // Only the goal-specific mode predicts race times.
                        let vdot = (recommendation_type == "goal_specific")
                            .then(|| {
                                resolve_athlete_vdot(
                                    profile.as_ref(),
                                    &activities,
                                    Timestamp::now(),
                                    algorithms,
                                )
                            })
                            .flatten();
                        let analysis = generate_training_recommendations(
                            &activities,
                            recommendation_type,
                            &athlete,
                            hr_zones.as_ref(),
                            algorithms,
                            user_timezone.as_deref(),
                            &AthleteReads {
                                nutrition: nutrition_athlete,
                                vdot,
                            },
                        );

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

                        with_caveat(apply_format_typed(result, analysis, output_format), caveat)
                    }
                    Err(e) => {
                        warn!(error = %e, "activity read for recommendations failed");
                        Ok(UniversalResponse {
                            success: false,
                            result: None,
                            error: Some(format!(
                                "Failed to fetch activities: {}",
                                e.tool_message()
                            )),
                            metadata: None,
                        })
                    }
                }
            }
            Err(response) => Ok(*response),
        }
    })
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use pierre_core::models::{ActivityBuilder, SportType};

    use super::*;

    /// A 10 km run `days_ago` days back at a steady 5 min/km, averaging
    /// `avg_hr` bpm.
    fn run(days_ago: i64, avg_hr: u32) -> Activity {
        ActivityBuilder::new(
            format!("run_{days_ago}"),
            "Run",
            SportType::Run,
            Timestamp::now() - Elapsed::days(days_ago),
            3000,
            "test",
        )
        .distance_meters(10_000.0)
        .average_heart_rate(avg_hr)
        .build()
    }

    /// The training summary of a comprehensive read, on cageux's zones for
    /// resting 50 / max 190.
    fn summary(activities: &[Activity]) -> Result<TrainingSummary, Box<dyn Error>> {
        let zones = HrZones::for_athlete(None, 50, 190)?;
        let result = generate_comprehensive_recommendations(
            activities,
            &AthleteInputs::default(),
            Some(&zones),
            &AlgorithmConfig::default(),
            &Zone::utc(),
        );
        Ok(result
            .training_summary
            .ok_or("a comprehensive read carries its training summary")?)
    }

    #[test]
    fn the_overtraining_flag_reports_short_recovery() -> Result<(), Box<dyn Error>> {
        // Six hard days in a row at a steady heart rate and pace: short
        // recovery between hard efforts is the only signal they carry.
        let hard: Vec<Activity> = (0..6).map(|day| run(day, 170)).collect();
        assert!(
            summary(&hard)?.overtraining_signals,
            "bug: six consecutive hard days did not raise the overtraining flag"
        );
        let easy: Vec<Activity> = (0..6).map(|day| run(day, 120)).collect();
        assert!(
            !summary(&easy)?.overtraining_signals,
            "bug: six easy days raised the overtraining flag"
        );
        Ok(())
    }
}
