// ABOUTME: The shapes the sleep and recovery tools answer with, and their derived schemas
// ABOUTME: Several embed dravr_cageux types whole, which is why cageux derives JsonSchema
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Result types for the five sleep and recovery tools.
//!
//! Three of these put `dravr_cageux` types on the wire whole —
//! `SleepQualityScore`, `HrvTrendAnalysis`, `RecoveryScore`,
//! `RestDayRecommendation`. Those are the science crate's shapes, so only it
//! can describe them; it derives `JsonSchema` as of v0.9.0 and these types
//! carry the derivation through rather than mirroring the fields, which would
//! be two structs describing one thing.

use chrono::{DateTime, Utc};
use pierre_intelligence::{
    recovery_calculator::{
        DataCompleteness, RecoveryCategory, RecoveryScore, RestDayRecommendation, TrainingReadiness,
    },
    sleep_analysis::{HrvRecoveryStatus, HrvTrendAnalysis, SleepQualityScore},
};
use serde::Serialize;

/// What `analyze_sleep_quality` answers with.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct SleepQualityResult {
    /// The night's quality score and its components, from the science crate.
    pub sleep_quality: SleepQualityScore,
    /// Heart-rate variability alongside it. Absent when the provider
    /// reported no RMSSD for the night — common on devices that do not
    /// measure it, so its absence is not a fault.
    pub hrv_analysis: Option<HrvTrendAnalysis>,
    /// The night this describes, as the provider dated it.
    pub analysis_date: DateTime<Utc>,
    /// The provider's own sleep score, when it publishes one. Kept beside
    /// ours so an athlete can see the two differ rather than wondering which
    /// number their watch showed.
    pub provider_score: Option<f64>,
}

/// Chronic and acute load with the balance between them.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct RecoveryTrainingLoad {
    /// Chronic training load — long-run fitness.
    pub ctl: f64,
    /// Acute training load — recent fatigue.
    pub atl: f64,
    /// Training stress balance, `ctl - atl`.
    pub tsb: f64,
}

/// Which providers the recovery score was assembled from.
///
/// Recovery is cross-provider by design: activities from one, sleep and HRV
/// from another. Naming both is what lets an athlete tell a low score from a
/// missing feed.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct RecoveryProvidersUsed {
    /// Where the training data came from.
    pub activity_provider: String,
    /// Where sleep and HRV came from. Absent when no sleep provider was
    /// named, in which case the score is TSB-only.
    pub sleep_provider: Option<String>,
}

/// What `calculate_recovery_score` answers with.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct RecoveryScoreResult {
    /// The score, its category, and the completeness of what fed it.
    pub recovery_score: RecoveryScore,
    /// The load numbers behind the TSB component.
    pub training_load: RecoveryTrainingLoad,
    /// WHOOP's own daily strain, when WHOOP is the sleep provider.
    pub whoop_daily_strain: Option<f32>,
    /// The sleep component, 0 to 100. Absent in TSB-only mode.
    pub sleep_quality_score: Option<f64>,
    /// The HRV component's verdict. Absent in TSB-only mode.
    pub hrv_status: Option<HrvRecoveryStatus>,
    /// Which feeds this was assembled from.
    pub providers_used: RecoveryProvidersUsed,
}

/// The recovery picture behind a rest-day call.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct RestDayRecoverySummary {
    /// Overall recovery, 0 to 100.
    pub overall_score: f64,
    /// Which band that falls in.
    pub category: RecoveryCategory,
    /// What the athlete is ready for.
    pub training_readiness: TrainingReadiness,
    /// How much of the picture was available.
    pub data_completeness: DataCompleteness,
    /// What was missing, in plain language. An empty list means nothing was.
    pub limitations: Vec<String>,
}

/// The individual signals behind a rest-day call.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct RestDayKeyFactors {
    /// Training stress balance.
    pub tsb: f64,
    /// The sleep component, 0 to 100; absent without a sleep provider.
    pub sleep_score: Option<f64>,
    /// Hours slept; absent without a sleep provider.
    pub sleep_hours: Option<f64>,
    /// The HRV verdict; absent without HRV.
    pub hrv_status: Option<HrvRecoveryStatus>,
}

/// What `suggest_rest_day` answers with.
///
/// The recommendation, plus the evidence for it. Both are on the wire so an
/// athlete can disagree with the call and still see what it was reasoning
/// from — a rest-day verdict with no visible basis is one they cannot argue
/// with.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct RestDayResult {
    /// The call, its confidence, and the reasons behind it.
    pub recommendation: RestDayRecommendation,
    /// The recovery picture it rests on.
    pub recovery_summary: RestDayRecoverySummary,
    /// The individual signals, so the athlete can weigh them themselves.
    pub key_factors: RestDayKeyFactors,
}

/// Sleep averages across the window, and the direction they are moving.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct SleepTrendSummary {
    /// Mean hours slept.
    pub average_duration_hours: f64,
    /// Mean sleep efficiency as a percentage.
    pub average_efficiency_percent: f64,
    /// Which way quality is moving: improving, stable or declining.
    pub quality_trend: String,
    /// Mean quality over the last seven nights.
    pub recent_7day_avg: f64,
    /// Mean quality over the seven before those, for comparison.
    pub previous_7day_avg: f64,
}

/// One night singled out of the window.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct SleepNight {
    /// The night, as the provider dated it.
    pub date: DateTime<Utc>,
    /// Its quality score.
    pub score: f64,
}

/// The best and worst nights in the window.
///
/// Both absent when the window held no scored nights — there is no best of
/// nothing, and reporting a zero-score night that does not exist would be
/// worse than saying nothing.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct SleepHighlights {
    /// The highest-scoring night.
    pub best_night: Option<SleepNight>,
    /// The lowest-scoring night.
    pub worst_night: Option<SleepNight>,
}

/// What `track_sleep_trends` answers with.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct SleepTrendsResult {
    /// The averages and their direction.
    pub trends: SleepTrendSummary,
    /// The extremes, for a coach to ask about.
    pub highlights: SleepHighlights,
    /// Plain-language readings of the window.
    pub insights: Vec<String>,
    /// How many scored nights the window held.
    pub data_points: usize,
}

/// The schedule being suggested.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct SleepScheduleRecommendation {
    /// Hours to aim for.
    pub target_hours: f64,
    /// When to go to bed, as `HH:MM` on the athlete's own clock.
    pub recommended_bedtime: String,
    /// When to get up, same clock.
    pub wake_time: String,
}

/// Why that schedule, rather than another.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct SleepScheduleRationale {
    /// The load the athlete is carrying.
    pub training_load: RecoveryTrainingLoad,
    /// What is coming, which is what shifts the target.
    pub upcoming_intensity: String,
}

/// What `optimize_sleep_schedule` answers with.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct SleepScheduleResult {
    /// The schedule itself.
    pub recommendations: SleepScheduleRecommendation,
    /// The reasoning, so the athlete can see it is not a generic eight hours.
    pub rationale: SleepScheduleRationale,
    /// Practical advice for actually hitting it.
    pub tips: Vec<String>,
}

// ============================================================================
// Payload builders
// ============================================================================
//
// The nested projections live here beside the types they build rather than in
// `inner.rs`, which is past the size ceiling and frozen — the same split as
// `goals_output` and `coaches_output`.

/// Assemble the `suggest_rest_day` answer from the call and its evidence.
#[must_use]
pub fn rest_day_payload(
    recommendation: RestDayRecommendation,
    recovery: &RecoveryScore,
    tsb: f64,
    sleep_score: Option<f64>,
    sleep_hours: Option<f64>,
    hrv_status: Option<HrvRecoveryStatus>,
) -> RestDayResult {
    RestDayResult {
        recommendation,
        recovery_summary: RestDayRecoverySummary {
            overall_score: recovery.overall_score,
            category: recovery.recovery_category,
            training_readiness: recovery.training_readiness,
            data_completeness: recovery.data_completeness,
            limitations: recovery.limitations.clone(),
        },
        key_factors: RestDayKeyFactors {
            tsb,
            sleep_score,
            sleep_hours,
            hrv_status,
        },
    }
}

/// Assemble the `optimize_sleep_schedule` answer.
#[must_use]
pub fn sleep_schedule_payload(
    target_hours: f64,
    recommended_bedtime: String,
    wake_time: &str,
    load: RecoveryTrainingLoad,
    upcoming_intensity: &str,
    tips: Vec<String>,
) -> SleepScheduleResult {
    SleepScheduleResult {
        recommendations: SleepScheduleRecommendation {
            target_hours,
            recommended_bedtime,
            wake_time: wake_time.to_owned(),
        },
        rationale: SleepScheduleRationale {
            training_load: load,
            upcoming_intensity: upcoming_intensity.to_owned(),
        },
        tips,
    }
}

/// Assemble the `calculate_recovery_score` answer.
#[must_use]
pub fn recovery_score_payload(
    recovery_score: RecoveryScore,
    load: RecoveryTrainingLoad,
    whoop_daily_strain: Option<f32>,
    sleep_quality_score: Option<f64>,
    hrv_status: Option<HrvRecoveryStatus>,
    activity_provider: String,
    sleep_provider: Option<String>,
) -> RecoveryScoreResult {
    RecoveryScoreResult {
        recovery_score,
        training_load: load,
        whoop_daily_strain,
        sleep_quality_score,
        hrv_status,
        providers_used: RecoveryProvidersUsed {
            activity_provider,
            sleep_provider,
        },
    }
}
