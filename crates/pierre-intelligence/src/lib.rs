// ABOUTME: Fitness intelligence bridge between dravr-cageux and pierre-core
// ABOUTME: Re-exports dravr-cageux algorithms with pierre-core type conversions
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![deny(unsafe_code)]

//! # Pierre Intelligence
//!
//! Bridge crate re-exporting `dravr-cageux` sports science engine
//! with `pierre-core` type conversions.

// Re-export pierre-core modules for path compatibility
pub use pierre_core::constants;
pub use pierre_core::errors;
pub use pierre_core::models;

/// Endurance Phase 1 latest-snapshot computation (`GET /api/v1/endurance/latest`).
pub mod latest_snapshot;
pub use latest_snapshot::{
    build_latest_snapshot, ActivitySection11Metrics, LatestSnapshot, LatestSnapshotActivityRow,
    DEFAULT_WINDOW_DAYS, MAX_WINDOW_DAYS,
};

/// Endurance Phase 2 daily training-history compute (`GET /api/v1/endurance/history`).
pub mod training_history_compute;
pub use training_history_compute::{
    compute_training_history, AthleteInputs, ACWR_ACUTE_DAYS, ACWR_CHRONIC_DAYS,
    ATL_WINDOW_DAYS as TH_ATL_WINDOW_DAYS, CTL_WINDOW_DAYS as TH_CTL_WINDOW_DAYS,
    FOSTER_WINDOW_DAYS, MAX_BACKFILL_DAYS as TH_MAX_BACKFILL_DAYS, RAMP_RATE_LOOKBACK_DAYS,
};

/// Endurance Phase 3 intervals.json builder (`GET /api/v1/endurance/intervals/{activity_id}`).
pub mod intervals;
pub use intervals::{build_intervals, IntervalRow, IntervalsExport};

/// Endurance Phase 3 GPX terrain analysis (`GET /api/v1/endurance/routes/{activity_id}`).
pub mod routes;
pub use routes::{
    build_route_summary, build_route_summary_from_streams, Climb, ClimbCategory, RouteSummary,
    TerrainSummary,
};

/// Endurance Phase 3 LT1 / LT2 threshold estimators.
pub mod threshold_estimation;
pub use threshold_estimation::{ThresholdEstimate, ThresholdInputs};

// Re-export all public submodules from dravr-cageux
pub use dravr_cageux::activity_analyzer;
pub use dravr_cageux::algorithms;
pub use dravr_cageux::analysis_config;
pub use dravr_cageux::analyzer;
pub use dravr_cageux::config;
pub use dravr_cageux::friend_activity_cache;
pub use dravr_cageux::goal_engine;
pub use dravr_cageux::insight_adapter;
pub use dravr_cageux::insights;
pub use dravr_cageux::metrics;
pub use dravr_cageux::metrics_extractor;
pub use dravr_cageux::nutrition_calculator;
pub use dravr_cageux::pattern_detection;
pub use dravr_cageux::performance_analyzer;
pub use dravr_cageux::performance_analyzer_v2;
pub use dravr_cageux::performance_prediction;
pub use dravr_cageux::physiological_constants;
pub use dravr_cageux::recipes;
pub use dravr_cageux::recommendation_engine;
pub use dravr_cageux::recovery_calculator;
pub use dravr_cageux::seasonality;
pub use dravr_cageux::sleep_analysis;
pub use dravr_cageux::statistical_analysis;
pub use dravr_cageux::training_load;
pub use dravr_cageux::types;
pub use dravr_cageux::visitor;

// Re-export intelligence types from dravr-cageux::types at crate root
// (excluding FitnessLevel/UserFitnessProfile which come from pierre-core)
pub use dravr_cageux::types::{
    ActivityInsights, ActivityIntelligence, AdvancedInsight, Anomaly, Confidence,
    ContextualFactors, ContextualWeeklyLoad, Goal, GoalStatus, GoalType, InsightSeverity,
    LocationContext, Milestone, PerformanceMetrics, PersonalRecord, ProgressReport,
    RecommendationPriority, RecommendationType, TimeFrame, TimeOfDay, TrainingRecommendation,
    TrendAnalysis, TrendDataPoint, TrendDirection, TrendIndicators, WeatherConditions,
    ZoneDistribution,
};

// Re-export fitness profile types from pierre-core
pub use pierre_core::intelligence::{
    FitnessLevel, TimeAvailability, UserFitnessProfile, UserPreferences,
};

// Re-export specific types at crate root for backward compatibility
pub use dravr_cageux::activity_analyzer::{ActivityAnalyzerTrait, AdvancedActivityAnalyzer};
pub use dravr_cageux::analysis_config::{AnalysisConfig, AnalysisConfigError, ConfidenceLevel};
pub use dravr_cageux::analyzer::ActivityAnalyzer;
pub use dravr_cageux::config::intelligence::{
    AggressiveStrategy, ConfigError, ConservativeStrategy, DefaultStrategy, IntelligenceConfig,
    IntelligenceStrategy,
};
pub use dravr_cageux::friend_activity_cache::{
    create_shared_cache, CacheConfig, CacheStats, DurationCategory, EffortLevel,
    FriendActivityCache, FriendActivitySummary, SharedFriendActivityCache,
};
pub use dravr_cageux::goal_engine::{
    AdjustmentType, AdvancedGoalEngine, GoalAdjustment, GoalDifficulty, GoalEngineTrait,
    GoalSuggestion,
};
pub use dravr_cageux::insight_adapter::{
    truncate_string, AdaptationResult, FitnessLevel as AdaptationFitnessLevel, InsightAdapter,
    UserTrainingContext,
};
pub use dravr_cageux::insights::Insight;
pub use dravr_cageux::metrics::{AdvancedMetrics, MetricsCalculator, ZoneAnalysis};
pub use dravr_cageux::metrics_extractor::{MetricSummary, MetricType, SafeMetricExtractor};
pub use dravr_cageux::nutrition_calculator::{
    calculate_carb_needs, calculate_daily_nutrition_needs, calculate_fat_needs,
    calculate_mifflin_st_jeor, calculate_nutrient_timing, calculate_protein_needs, calculate_tdee,
    ActivityLevel, DailyNutritionNeeds, DailyNutritionParams, Gender, MacroPercentages,
    NutrientTimingPlan, PostWorkoutNutrition, PreWorkoutNutrition, ProteinDistribution,
    TrainingGoal, WorkoutIntensity,
};
pub use dravr_cageux::pattern_detection::{
    HardEasyPattern, OvertrainingSignals, PatternDetector, VolumeProgressionPattern, VolumeTrend,
    WeeklySchedulePattern,
};
pub use dravr_cageux::performance_analyzer::{
    AdvancedPerformanceAnalyzer, PerformanceAnalyzerTrait,
};
pub use dravr_cageux::performance_analyzer_v2::{
    ActivityGoal, FitnessScore, PerformanceAnalyzerV2, PerformancePrediction, TrainingLoadAnalysis,
    WeeklyLoad,
};
pub use dravr_cageux::performance_prediction::{PerformancePredictor, RacePredictions};
pub use dravr_cageux::recipes::{
    convert_to_grams, ConversionError, DietaryRestriction, IngredientDensity, IngredientUnit,
    MacroTargets, MealTiming, Recipe, RecipeConstraints, RecipeIngredient, SkillLevel,
    ValidatedNutrition,
};
pub use dravr_cageux::recommendation_engine::{
    AdvancedRecommendationEngine, RecommendationEngineTrait,
};
pub use dravr_cageux::recovery_calculator::{
    RecoveryCalculator, RecoveryCategory, RecoveryComponents, RecoveryScore, RestDayRecommendation,
    TrainingReadiness,
};
pub use dravr_cageux::sleep_analysis::{
    HrvRecoveryStatus, HrvTrend, HrvTrendAnalysis, SleepAnalyzer, SleepData, SleepQualityCategory,
    SleepQualityScore,
};
pub use dravr_cageux::statistical_analysis::{
    RegressionResult, SignificanceLevel, StatisticalAnalyzer,
};
pub use dravr_cageux::training_load::{
    OvertrainingRisk, RiskLevel, TrainingLoad, TrainingLoadCalculator, TrainingStatus, TssDataPoint,
};
pub use dravr_cageux::visitor::{
    DecouplingDetector, NormalizedPowerCalculator, StatsCollector, StreamStats, TimeSeriesExt,
    TimeSeriesVisitor, ZoneBoundaries, ZoneDistributionResult, ZoneTimeCalculator,
};
