// ABOUTME: Thin bridge re-exporting the dravr-cageux sports-science engine for the platform
// ABOUTME: Surfaces dravr-cageux + pierre-core intelligence types under one pierre_intelligence path
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![deny(unsafe_code)]

//! # Pierre Intelligence
//!
//! Re-export surface for the `dravr-cageux` sports-science engine.
//!
//! Bundles the `dravr-cageux` engine and the `pierre-core` intelligence types
//! under one `pierre_intelligence::*` path. Platform-side computation that is
//! not part of the engine lives in the `pierre-fitness-compute` crate.

// Re-export pierre-core modules for path compatibility
pub use pierre_core::constants;
pub use pierre_core::errors;
pub use pierre_core::models;

// Re-export all public submodules from dravr-cageux
pub use dravr_cageux::activity_analyzer;
pub use dravr_cageux::algorithms;
pub use dravr_cageux::analysis_config;
pub use dravr_cageux::analyzer;
pub use dravr_cageux::config;
pub use dravr_cageux::goal_engine;
pub use dravr_cageux::insights;
pub use dravr_cageux::metrics;
pub use dravr_cageux::metrics_extractor;
pub use dravr_cageux::nutrition_calculator;
pub use dravr_cageux::pattern_detection;
pub use dravr_cageux::performance_analyzer;
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

// Types re-exported at the crate root so callers reach them by one path.
pub use dravr_cageux::activity_analyzer::ActivityAnalyzerTrait;
pub use dravr_cageux::analysis_config::{AnalysisConfig, AnalysisConfigError, ConfidenceLevel};
pub use dravr_cageux::analyzer::ActivityAnalyzer;
pub use dravr_cageux::config::intelligence::{
    AggressiveStrategy, AlgorithmConfig, AlgorithmParamsConfig, ConfigError, ConservativeStrategy,
    DefaultStrategy, IntelligenceConfig, IntelligenceStrategy,
};
pub use dravr_cageux::goal_engine::{
    AdjustmentType, AdvancedGoalEngine, GoalAdjustment, GoalDifficulty, GoalEngineTrait,
    GoalSuggestion,
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
pub use dravr_cageux::performance_analyzer::PerformanceAnalyzerTrait;
pub use dravr_cageux::performance_prediction::{PerformancePredictor, RacePredictions};
pub use dravr_cageux::recipes::{
    convert_to_grams, ConversionError, DietaryRestriction, IngredientDensity, IngredientUnit,
    MacroTargets, MealTiming, Recipe, RecipeConstraints, RecipeIngredient, SkillLevel,
    ValidatedNutrition,
};
pub use dravr_cageux::recommendation_engine::RecommendationEngineTrait;
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
    FormBand, OvertrainingRisk, RiskLevel, TrainingLoad, TrainingLoadCalculator, TssDataPoint,
};
pub use dravr_cageux::visitor::{
    DecouplingDetector, NormalizedPowerCalculator, StatsCollector, StreamStats, TimeSeriesExt,
    TimeSeriesVisitor,
};
