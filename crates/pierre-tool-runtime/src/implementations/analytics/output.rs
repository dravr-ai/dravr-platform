// ABOUTME: The shapes the analytics tools answer with, and the schemas derived from them
// ABOUTME: One module for the family so a client can see every analytics contract in one place
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Result types for the analytics tools.
//!
//! The payloads these describe were built with `json!` inside the handler
//! modules, which is why they could not declare an `outputSchema`: a literal
//! has no type to derive one from. They are platform-owned shapes — the
//! numbers come from `dravr_cageux`, but the projection is ours — so typing
//! them needs nothing from the science crate.
//!
//! Several of these tools answer with genuinely different shapes depending on
//! what the athlete asked for or what the data supported. Those are untagged
//! enums, one variant per shape, so the derived schema is an `anyOf` a client
//! can actually branch on rather than a union of every field anything might
//! send. Every variant carries a required field no other variant has, which
//! is what makes the arms distinguishable in practice — schemars emits
//! `anyOf` for an untagged enum, so the schema does not assert that for you.

use serde::Serialize;

// ----------------------------------------------------------------------------
// analyze_performance_trends
// ----------------------------------------------------------------------------

/// What `analyze_performance_trends` answers with.
///
/// One shape covers all seven of the handler's returns. Six of them are
/// degenerate — no activities, an unknown metric, too few points to regress,
/// a regression that failed — and differ from the real answer only in
/// carrying no `statistics`. Making that one field optional says exactly
/// that, where seven variants would have said it six times.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct PerformanceTrendsResult {
    /// The metric requested, echoed back.
    pub metric: String,
    /// The window requested, echoed back: week, month, quarter or year.
    pub timeframe: String,
    /// `improving`, `stable` or `declining` when the regression ran; otherwise
    /// why it did not — `no_data`, `invalid_metric`, `needs_more_data`,
    /// `insufficient_data` or `calculation_error`.
    pub trend: String,
    /// How many activities the answer is based on.
    pub activities_analyzed: usize,
    /// The regression itself. Absent when `trend` reports why there is none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub statistics: Option<TrendStatistics>,
    /// Plain-language readings of the trend, always at least one.
    pub insights: Vec<String>,
}

/// The linear regression behind a performance trend.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct TrendStatistics {
    /// Change in the metric per day.
    pub slope: f64,
    /// Share of variance the line explains, 0 to 1.
    pub r_squared: f64,
    /// Confidence in the trend. The same number as `r_squared`: the fit is
    /// what the confidence is, and both names are on the wire already.
    pub confidence: f64,
    /// Pearson correlation between the metric and time.
    pub correlation: f64,
    /// Standard error of the slope estimate.
    pub standard_error: f64,
    /// Significance of the slope. Absent when the regression could not
    /// produce one — too few points for the t-distribution to be defined.
    pub p_value: Option<f64>,
    /// Mean of every point in the window, for comparison against the fit.
    pub moving_average_7day: f64,
    /// First value in the window; absent when the window was empty.
    pub start_value: Option<f64>,
    /// Last value in the window; absent when the window was empty.
    pub end_value: Option<f64>,
    /// Change from first to last as a percentage. Absent when there are fewer
    /// than two points, or the first value was zero and the ratio undefined.
    pub percent_change: Option<f64>,
}

// ----------------------------------------------------------------------------
// detect_patterns
// ----------------------------------------------------------------------------

/// What `detect_patterns` answers with.
///
/// The tool takes a `pattern_type` and each detector reports something
/// different — a weekly schedule has training days, an overtraining check has
/// risk and warning signs — so this is genuinely five shapes rather than one
/// with many optional fields. Every variant carries a required field no other
/// variant has, so exactly one arm of the derived `anyOf` accepts any given
/// answer — `every_detect_patterns_shape_matches_exactly_one_arm` checks
/// that arm by arm, because `anyOf` alone would tolerate an overlap.
#[derive(Debug, Serialize, schemars::JsonSchema)]
#[serde(untagged)]
pub enum PatternsResult {
    /// Fewer than three activities: nothing to detect from.
    Insufficient(InsufficientPatternData),
    /// Which days of the week the athlete trains on.
    WeeklySchedule(Box<WeeklySchedulePatternResult>),
    /// Whether hard and easy days alternate.
    HardEasy(Box<HardEasyPatternResult>),
    /// Whether weekly volume is climbing, and whether it spiked.
    VolumeProgression(Box<VolumeProgressionResult>),
    /// Warning signs of accumulated fatigue.
    Overtraining(Box<OvertrainingResult>),
}

/// The answer when there is not enough history to detect anything.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct InsufficientPatternData {
    /// The pattern type requested, echoed back.
    pub pattern_type: String,
    /// How many activities were available. Fewer than three.
    pub activities_analyzed: usize,
    /// Empty: nothing was detected.
    pub patterns_detected: Vec<String>,
    /// Says what is missing.
    pub insights: Vec<String>,
    /// Always `insufficient_data`.
    pub confidence: String,
}

/// The answer for `pattern_type` `weekly_schedule`, the default.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct WeeklySchedulePatternResult {
    /// Always `weekly_schedule`.
    pub pattern_type: String,
    /// How often the athlete trains on each weekday, counted on their own
    /// civil clock rather than in UTC.
    pub preferred_training_days: Vec<DayFrequency>,
    /// Descriptions of what was detected; empty when training is variable.
    pub patterns_detected: Vec<String>,
    /// Plain-language readings, always at least one.
    pub insights: Vec<String>,
    /// How concentrated training is on particular days, 0 to 100.
    pub consistency_score: f64,
    /// Mean sessions per week over the window.
    pub avg_activities_per_week: f64,
    /// `high`, `medium` or `low`, from the consistency score.
    pub confidence: String,
}

/// How often one weekday is trained on.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct DayFrequency {
    /// The weekday.
    pub day: String,
    /// Sessions counted on it.
    pub frequency: u32,
}

/// The answer for `pattern_type` `training_blocks`.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct HardEasyPatternResult {
    /// Always `training_blocks`.
    pub pattern_type: String,
    /// Whether an alternating hard/easy structure was found.
    pub pattern_detected: bool,
    /// How the sessions split by intensity.
    pub intensity_distribution: IntensityDistribution,
    /// Whether easy days follow hard ones often enough.
    pub adequate_recovery: bool,
    /// The description when a pattern was found; empty when none was.
    pub patterns_detected: Vec<String>,
    /// Plain-language readings, always at least one.
    pub insights: Vec<String>,
    /// `medium` when a pattern was detected, `low` when not.
    pub confidence: String,
}

/// The hard/easy split of a training block.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct IntensityDistribution {
    /// Share of sessions that were hard, as a percentage.
    pub hard_percentage: f64,
    /// Share of sessions that were easy, as a percentage.
    pub easy_percentage: f64,
}

/// The answer for `pattern_type` `progression`.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct VolumeProgressionResult {
    /// Always `progression`.
    pub pattern_type: String,
    /// `increasing`, `decreasing` or `stable`.
    pub trend: String,
    /// Volume per week over the window, oldest first.
    pub weekly_volumes: Vec<f64>,
    /// The week numbers those volumes belong to, in the same order.
    pub week_numbers: Vec<u32>,
    /// Whether any week jumped sharply above the recent baseline.
    pub volume_spikes_detected: bool,
    /// Which weeks spiked.
    pub spike_weeks: Vec<u32>,
    /// The same readings as `insights`; both are on the wire already.
    pub patterns_detected: Vec<String>,
    /// Plain-language readings, always at least one.
    pub insights: Vec<String>,
    /// Always `medium`.
    pub confidence: String,
}

/// The answer for `pattern_type` `overtraining`.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct OvertrainingResult {
    /// Always `overtraining`.
    pub pattern_type: String,
    /// `low`, `moderate` or `high`.
    pub risk_level: String,
    /// The signs that fired; empty when none did.
    pub warning_signs: Vec<String>,
    /// The warning signs, or a note that none were found.
    pub insights: Vec<String>,
    /// Whether heart rate climbed for the same effort.
    pub hr_drift_detected: bool,
    /// Whether performance fell while training continued.
    pub performance_decline: bool,
    /// Whether hard efforts came too close together.
    pub insufficient_recovery: bool,
    /// Always `medium`.
    pub confidence: String,
    /// What to do about it, graded by risk level.
    pub recommendations: Vec<String>,
}

// ----------------------------------------------------------------------------
// calculate_metrics
// ----------------------------------------------------------------------------

/// What `calculate_metrics` answers with.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ActivityMetricsResult {
    /// Minutes per kilometre.
    pub pace: f64,
    /// Kilometres per hour.
    pub speed: f64,
    /// Effort relative to the athlete's maximum heart rate.
    pub intensity_score: f64,
    /// Distance covered per unit of effort.
    pub efficiency_score: f64,
    /// The maximum heart rate the intensity score was computed against.
    pub max_hr_used: f64,
    /// Where that maximum came from — given by the athlete, estimated from
    /// age, or a default — so the athlete can judge the number.
    pub max_hr_source: String,
    /// The inputs, echoed in the units the metrics were computed in.
    pub metrics_summary: MetricsInputSummary,
}

/// The activity the metrics were computed from.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct MetricsInputSummary {
    /// Distance in kilometres.
    pub distance_km: f64,
    /// Duration in minutes.
    pub duration_minutes: u64,
    /// Elevation gained in metres.
    pub elevation_meters: f64,
    /// Mean heart rate; absent when the activity carried none.
    pub average_heart_rate: Option<u32>,
}

// ----------------------------------------------------------------------------
// predict_performance
// ----------------------------------------------------------------------------

/// What `predict_performance` answers with.
///
/// Two shapes: a prediction, or a statement of why there isn't one. The
/// no-prediction shape carries an optional `error` because there are two ways
/// to reach it — no running history to predict from, and a VDOT computation
/// that failed on the history there was — and they differ only in whether
/// there is a fault to report. Modelling them as separate variants would put
/// one variant's required keys inside the other's, and then no client could
/// tell which it had been handed.
#[derive(Debug, Serialize, schemars::JsonSchema)]
#[serde(untagged)]
pub enum RacePredictionResult {
    /// A prediction was made.
    Predicted(Box<RacePredictionDetail>),
    /// No prediction could be made, and why.
    Unavailable(NoRacePrediction),
}

/// The answer when no race prediction could be made.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct NoRacePrediction {
    /// The sport requested, echoed back.
    pub target_sport: String,
    /// Why there is no prediction, in plain language.
    pub message: String,
    /// The fault, when a computation failed rather than there being nothing
    /// to compute from. Absent when the athlete simply has no running history.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Always empty here, and always an array.
    ///
    /// It used to be an empty OBJECT on the two no-history paths and an empty
    /// ARRAY on the failure path — the same key, two JSON types, so a client
    /// reading `predictions.length` got `undefined` for one of them. Typing
    /// the answer is what surfaced it; an array is what the successful shape
    /// sends, so an array is what the empty case sends now.
    pub predictions: Vec<RacePrediction>,
}

/// A race prediction, with the evidence it rests on.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct RacePredictionDetail {
    /// The sport requested, echoed back.
    pub target_sport: String,
    /// VDOT, Jack Daniels' aerobic-capacity number, rounded.
    pub vdot: f64,
    /// The effort the prediction was derived from.
    pub best_performance: BestPerformance,
    /// One prediction per standard race distance.
    pub predictions: Vec<RacePrediction>,
    /// How much to trust it, as a band rather than a number — it comes from
    /// the recency of the best effort and the volume behind it, neither of
    /// which supports a false precision like 0.62.
    pub confidence: String,
    /// How many running activities were considered.
    pub activities_analyzed: usize,
    /// What the prediction assumes, so the athlete can judge it.
    pub notes: Vec<String>,
}

/// The athlete's best recent effort, which sets the prediction.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct BestPerformance {
    /// Distance covered, in metres.
    pub distance_meters: f64,
    /// Time taken, in seconds.
    pub time_seconds: f64,
    /// Pace as `m:ss`, for reading rather than arithmetic.
    pub pace_min_km: String,
    /// RFC 3339 timestamp of the effort.
    pub date: String,
}

/// A predicted time at one race distance.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct RacePrediction {
    /// The race, by name: `5K`, `10K`, `Half Marathon`, `Marathon`.
    pub distance: String,
    /// That race in metres, so a client need not parse the name.
    pub distance_meters: f64,
    /// Predicted time in seconds, rounded.
    pub predicted_time_seconds: f64,
    /// The same time as `h:mm:ss`, for reading.
    pub predicted_time_formatted: String,
    /// The pace it implies, as `m:ss` per kilometre.
    pub predicted_pace_min_km: String,
}

// ----------------------------------------------------------------------------
// calculate_fitness_score
// ----------------------------------------------------------------------------

/// What `calculate_fitness_score` answers with.
#[derive(Debug, Serialize, schemars::JsonSchema)]
#[serde(untagged)]
pub enum FitnessScoreResult {
    /// A score, and the three components behind it.
    Scored(Box<FitnessScoreDetail>),
    /// No activities in the window, so no score.
    NoData(NoFitnessScore),
}

/// The answer when there is nothing in the window to score.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct NoFitnessScore {
    /// The window requested, echoed back.
    pub timeframe: String,
    /// Always zero: reported rather than omitted so a client charting the
    /// score over time has a point rather than a gap.
    pub fitness_score: i32,
    /// Always `Beginner`, the level a zero score classifies to.
    pub level: String,
    /// Which of the two empty cases this is — no activities at all, or none
    /// inside the window.
    pub message: String,
}

/// A fitness score and what produced it.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct FitnessScoreDetail {
    /// The window requested, echoed back.
    pub timeframe: String,
    /// The score, 0 to 100, rounded to a whole number.
    pub fitness_score: i32,
    /// Beginner, Intermediate, Advanced, Elite or Very High.
    pub level: String,
    /// Whether the score is rising, flat or falling across the window.
    pub trend: String,
    /// The three weighted parts of the score.
    pub components: FitnessComponents,
    /// The training-load numbers the CTL component came from.
    pub metrics: FitnessLoadMetrics,
    /// How many activities were in the window.
    pub activities_analyzed: usize,
    /// What each component means, so the number is readable without docs.
    pub interpretation: FitnessInterpretation,
}

/// The three components of a fitness score, each 0 to 100 and rounded.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct FitnessComponents {
    /// Chronic training load, normalised. Weighted 40%.
    pub ctl_score: f64,
    /// Share of weeks with three or more sessions. Weighted 30%.
    pub consistency_score: f64,
    /// Pace improvement across the window. Weighted 30%.
    pub performance_score: f64,
}

/// The training-load numbers behind the CTL component, rounded.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct FitnessLoadMetrics {
    /// Chronic training load — long-run fitness.
    pub ctl: f64,
    /// Acute training load — recent fatigue.
    pub atl: f64,
    /// Training stress balance, `ctl - atl`.
    pub tsb: f64,
}

/// Plain-language definitions of the three components.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct FitnessInterpretation {
    /// What CTL measures.
    pub ctl: String,
    /// What the consistency component measures.
    pub consistency: String,
    /// What the performance component measures.
    pub performance: String,
}
