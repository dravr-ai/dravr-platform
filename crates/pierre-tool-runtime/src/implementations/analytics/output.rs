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

use pierre_core::models::{FormBand, FormInterpretation};
use pierre_fitness_compute::weather::WeatherDifficulty;
use serde::{Deserialize, Serialize};

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
    /// Seconds per kilometre. `duration_seconds / distance_km`, which is what
    /// the handler computes — a client rendering it as minutes is out by 60.
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
    /// The score before the recovery adjustment, present only when one ran.
    /// Zero either way here, and reported for the same reason `fitness_score`
    /// is: a client charting both lines wants a point, not a gap.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fitness_score_unadjusted: Option<i32>,
    /// The sleep-derived adjustment, present only when a sleep provider was
    /// named and answered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery_adjustment: Option<RecoveryAdjustment>,
    /// Which providers the answer was built from.
    pub providers_used: ProvidersUsed,
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
    /// The score before the recovery adjustment, present only when one ran.
    /// Both numbers ship so a client can show what sleep cost or bought.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fitness_score_unadjusted: Option<i32>,
    /// The sleep-derived adjustment, present only when a sleep provider was
    /// named and answered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery_adjustment: Option<RecoveryAdjustment>,
    /// Which providers the answer was built from.
    pub providers_used: ProvidersUsed,
}

/// What a night's sleep did to the score.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct RecoveryAdjustment {
    /// The sleep quality score the adjustment was derived from, 0 to 100.
    pub recovery_score: f64,
    /// The multiplier applied: 1.05 above 90, 1.0 above 70, 0.95 above 50,
    /// 0.90 below that.
    pub adjustment_factor: f64,
    /// The sleep provider that supplied the night.
    pub sleep_provider: String,
}

/// The providers behind an answer, named so a client can say where a number
/// came from rather than implying one source.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ProvidersUsed {
    /// The provider the activities came from.
    pub activity_provider: String,
    /// The sleep provider, when one was asked for and answered.
    pub sleep_provider: Option<String>,
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

// ----------------------------------------------------------------------------
// analyze_training_load
// ----------------------------------------------------------------------------

/// What `analyze_training_load` answers with.
#[derive(Debug, Serialize, schemars::JsonSchema)]
#[serde(untagged)]
pub enum TrainingLoadResult {
    /// The load numbers and what they mean for the next few days.
    Analyzed(Box<TrainingLoadDetail>),
    /// Nothing to analyse, or not enough history for the EMA to converge.
    NoData(NoTrainingLoad),
}

/// The answer when there is no load to report.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct NoTrainingLoad {
    /// The window requested, echoed back.
    pub timeframe: String,
    /// Which of the two cases this is — no activities at all, or too little
    /// history for the calculator to produce a load.
    pub message: String,
    /// Which providers were asked. Present on the empty answer too: "we
    /// looked at Garmin and found nothing" and "we looked at nothing" are
    /// different answers, and the athlete is entitled to tell them apart.
    pub providers_used: ProvidersUsed,
}

/// An athlete's training load and the reading of it.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct TrainingLoadDetail {
    /// The window requested, echoed back.
    pub timeframe: String,
    /// The load numbers themselves.
    pub load_metrics: LoadMetrics,
    /// Which form band the athlete is in, as a share of their own chronic
    /// load rather than an absolute TSB.
    pub form_band: FormBand,
    /// The band's own label, so a client need not map the enum itself.
    pub form_assessment: String,
    /// What the band says about tapering.
    pub taper_status: String,
    /// Periodization notes, empty when the load suggests none.
    pub periodization_suggestions: Vec<String>,
    /// Where this chronic load sits among training populations.
    pub training_zones: TrainingZone,
    /// Descriptive guidance for the next few days. Never injury risk.
    pub recommendations: Vec<String>,
    /// How many days of TSS history the load was computed over.
    pub activities_analyzed: usize,
    /// The shared reading of form, the same one `get_training_history` uses.
    pub interpretation: FormInterpretation,
    /// Sleep context, present only when a sleep provider was named and
    /// answered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery_context: Option<LoadRecoveryContext>,
    /// Which providers the answer was built from.
    pub providers_used: ProvidersUsed,
}

/// The training-load numbers, rounded.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct LoadMetrics {
    /// Chronic training load — long-run fitness.
    pub ctl: f64,
    /// Acute training load — recent fatigue.
    pub atl: f64,
    /// Training stress balance, `ctl - atl`.
    pub tsb: f64,
    /// Form as a percentage of the athlete's own CTL, which is how the band
    /// is decided. `None` when CTL is too small to divide by.
    pub tsb_pct_of_ctl: Option<f64>,
    /// Weekly TSS totals across the window, oldest week first.
    pub weekly_tss: Vec<WeeklyTss>,
}

/// One week's training stress.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct WeeklyTss {
    /// Weeks since the first day of history, starting at zero.
    pub week: i32,
    /// Total TSS in that week, rounded.
    pub total_tss: f64,
}

/// Where a chronic load sits among training populations.
///
/// `level` and `ctl_range` are decided together from the same CTL, so the
/// named band and the numbers behind it cannot disagree.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct TrainingZone {
    /// Beginner, Intermediate, Advanced, Elite or Very High.
    pub level: String,
    /// The CTL span that level covers, for the athlete to place themselves.
    pub ctl_range: String,
}

/// The sleep side of a load reading.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct LoadRecoveryContext {
    /// Overall sleep quality, 0 to 100.
    pub sleep_quality_score: f64,
    /// The analyser's word for that score.
    pub recovery_status: String,
    /// Whether the night carried an HRV reading at all — distinct from
    /// `hrv_rmssd` being absent, which could also mean the provider omitted
    /// it from a night it did return.
    pub hrv_available: bool,
    /// RMSSD in milliseconds, when the provider reported one.
    pub hrv_rmssd: Option<f64>,
    /// Hours slept.
    pub sleep_hours: f64,
    /// The sleep provider that supplied the night.
    pub sleep_provider: String,
}

// ----------------------------------------------------------------------------
// compare_activities
// ----------------------------------------------------------------------------

/// What `compare_activities` answers with.
///
/// One shape rather than three, discriminated by `comparison_type`. The three
/// modes overlap heavily and, worse, their required keys nest — a
/// `pr_comparison` with no history requires only what every other mode also
/// carries — so as separate untagged variants no client could tell them
/// apart. `comparison_type` is always present and says which mode ran; the
/// fields that mode does not fill are absent.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct CompareActivitiesResult {
    /// The activity that was compared.
    pub activity_id: String,
    /// Which comparison ran: `similar_activities`, `pr_comparison` or
    /// `specific_activity`. Read this before anything else.
    pub comparison_type: String,
    /// How many similar activities were found. Only `similar_activities`
    /// reports it, and it reports zero when there were none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comparison_count: Option<usize>,
    /// The sport, when there was a comparison to make. Absent on the empty
    /// answers, where nothing was compared.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sport_type: Option<String>,
    /// The activity compared against. Only `specific_activity`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comparison_activity_id: Option<String>,
    /// Its title, so the answer reads without a second call.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comparison_activity_name: Option<String>,
    /// Metric-by-metric comparison, for `similar_activities` and
    /// `specific_activity`. Which of `average` or `comparison` each row
    /// carries follows from the mode.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comparisons: Option<Vec<MetricComparison>>,
    /// Personal-record comparison, for `pr_comparison` only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pr_comparisons: Option<Vec<PersonalRecordComparison>>,
    /// Why the comparison could not run — a named activity that does not
    /// exist, for instance. Absent when it ran.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Plain-language readings, always at least one.
    pub insights: Vec<String>,
}

/// One metric compared against a baseline.
///
/// Every number is `f64`, including power and duration, which the untyped
/// payload sent as integers. Uniform rows are the point of typing them: a
/// client charting `current` should not have to handle two JSON number
/// shapes depending on which metric it landed on.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct MetricComparison {
    /// Which metric: `pace`, `heart_rate`, `distance`, `duration`,
    /// `elevation_gain` or `average_power`.
    pub metric: String,
    /// The value on the activity being compared.
    pub current: f64,
    /// The mean across the similar activities. Present for
    /// `similar_activities`; absent for `specific_activity`, which has a
    /// single `comparison` instead.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub average: Option<f64>,
    /// The value on the one activity compared against. Present for
    /// `specific_activity`; absent for `similar_activities`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comparison: Option<f64>,
    /// Difference as a percentage of the baseline.
    pub difference_percent: f64,
    /// Whether the difference is an improvement. Absent for metrics where
    /// better has no direction — distance and elevation are what the route
    /// was, not how well it went.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub improved: Option<bool>,
}

/// One metric measured against the athlete's own best.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct PersonalRecordComparison {
    /// Which metric: `distance`, `pace` or `average_power`.
    pub metric: String,
    /// The value on this activity.
    pub current: f64,
    /// The athlete's best for this metric in this sport.
    pub personal_record: f64,
    /// Whether this activity set it.
    pub is_record: bool,
    /// How close it came, as a percentage of the record. Only distance
    /// reports it — for pace, lower is better, so a percentage of the record
    /// would read backwards.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub percent_of_pr: Option<f64>,
}

// ----------------------------------------------------------------------------
// get_activity_intelligence
// ----------------------------------------------------------------------------

/// The numbers an intelligence analysis is grounded in.
///
/// Computed here, never taken from the model: an LLM asked to restate an
/// athlete's distance will sometimes get it wrong, and these are the figures
/// the summary is supposed to be about.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ActivityPerformanceMetrics {
    /// Distance in kilometres; absent for an activity with none.
    pub distance_km: Option<f64>,
    /// Moving time in minutes.
    pub duration_minutes: Option<f64>,
    /// Elevation gained in metres; absent when the device recorded none.
    pub elevation_meters: Option<f64>,
    /// Mean heart rate; absent without a strap.
    pub average_heart_rate: Option<u32>,
    /// Peak heart rate; absent without a strap.
    pub max_heart_rate: Option<u32>,
    /// Energy in kilocalories, as the provider estimated it.
    pub calories: Option<u32>,
}

/// The analysis itself — the part a language model may write.
///
/// `Deserialize` because it is parsed back OUT of a model's reply: the client
/// LLM is asked for this shape, and an answer that does not fit is wrapped
/// rather than passed through. Before that, whatever the model emitted became
/// the tool's answer unchecked, which is a contract no schema could state.
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ActivityIntelligence {
    /// One or two sentences an athlete reads first.
    pub summary: String,
    /// What stood out, one per entry.
    pub insights: Vec<String>,
    /// What to do next.
    pub recommendations: Vec<String>,
    /// Where the analysis came from: `deterministic` when this server wrote
    /// it, `mcp_sampling` when the client's model did. An athlete should be
    /// able to tell which, and so should a support engineer reading a trace.
    pub source: String,
}

/// What `get_activity_intelligence` answers with.
///
/// The identifying fields and the metrics are ours on both paths. Only
/// `intelligence` may come from a model, and only after it parses into the
/// declared shape.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ActivityIntelligenceResult {
    /// The activity analysed.
    pub activity_id: String,
    /// Its sport, as the provider classified it.
    pub activity_type: String,
    /// RFC 3339 timestamp of the analysis, not of the activity.
    pub timestamp: String,
    /// The analysis.
    pub intelligence: ActivityIntelligence,
    /// The figures the analysis is about, computed here.
    pub performance_metrics: ActivityPerformanceMetrics,
    /// Present when the activity asked for did not exist and the most recent
    /// one was analysed instead. Its absence means the caller got what it
    /// asked for, which is the distinction a client has to be able to make
    /// before it says "your ride" about someone's ride.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_selected: Option<AutoSelectedActivity>,
}

/// Why a different activity was analysed, and which.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct AutoSelectedActivity {
    /// What went wrong with the requested id.
    pub reason: String,
    /// The id actually analysed.
    pub selected_activity: String,
    /// Its name, so a client can say which ride it means.
    pub selected_activity_name: String,
    /// Its date as `YYYY-MM-DD`.
    pub selected_activity_date: String,
    /// The recent activities offered, so the caller can retry with a real id.
    pub available_activities: Vec<String>,
}

// ----------------------------------------------------------------------------
// analyze_weather_impact
// ----------------------------------------------------------------------------

/// The weather as reported, in the units the caller asked for.
///
/// Untagged over the two unit systems: metric carries Celsius and km/h,
/// imperial Fahrenheit and mph. Each arm requires a temperature key the other
/// does not have, which is what lets a client tell them apart — the tool does
/// NOT echo the unit choice inside this block.
#[derive(Debug, Serialize, schemars::JsonSchema)]
#[serde(untagged)]
pub enum WeatherReading {
    /// Celsius and kilometres per hour.
    Metric(MetricWeather),
    /// Fahrenheit and miles per hour.
    Imperial(ImperialWeather),
}

/// Weather in Celsius and km/h.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct MetricWeather {
    /// Air temperature in degrees Celsius.
    pub temperature_celsius: f32,
    /// Relative humidity as a percentage; absent when the model reported none.
    pub humidity_percentage: Option<f32>,
    /// Wind speed in kilometres per hour; absent likewise.
    pub wind_speed_kmh: Option<f32>,
    /// Sky and precipitation, in words.
    pub conditions: String,
}

/// Weather in Fahrenheit and mph.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ImperialWeather {
    /// Air temperature in degrees Fahrenheit, rounded.
    pub temperature_fahrenheit: f64,
    /// Relative humidity as a percentage; absent when the model reported none.
    pub humidity_percentage: Option<f32>,
    /// Wind speed in miles per hour, to one decimal; absent likewise.
    pub wind_speed_mph: Option<f64>,
    /// Sky and precipitation, in words.
    pub conditions: String,
}

/// How the weather bore on the effort.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct WeatherImpactAssessment {
    /// How much harder the conditions made it. The ENUM, not a string: it
    /// is `#[serde(rename_all = "lowercase")]`, so the wire carries "ideal",
    /// not the Rust name — and the schema lists the four bands a client can
    /// branch on.
    pub difficulty_level: WeatherDifficulty,
    /// Which conditions contributed, one per entry.
    pub impact_factors: Vec<String>,
    /// The adjustment to apply when comparing this effort with a fair-weather
    /// one, so a slow day in the heat is not read as lost fitness.
    pub performance_adjustment: f32,
}

/// What `analyze_weather_impact` answers with.
///
/// `weather` and `impact` are null together: without GPS coordinates there is
/// nothing to look up, and a disabled provider returns nothing to assess.
/// `note` says which of those it was, so a client can tell "we could not" from
/// "there was nothing to report".
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct WeatherImpactResult {
    /// The activity analysed.
    pub activity_id: String,
    /// Its title, so the answer reads without a second call.
    pub activity_name: String,
    /// The conditions at the start, or null.
    pub weather: Option<WeatherReading>,
    /// The assessment, or null.
    pub impact: Option<WeatherImpactAssessment>,
    /// Why there is no weather. Absent when there is.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// The unit system the numbers are in: `metric` or `imperial`.
    pub units: String,
}
