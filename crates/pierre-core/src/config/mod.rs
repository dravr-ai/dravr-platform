// ABOUTME: Configuration types shared across the workspace
// ABOUTME: Contains FitnessConfig, FitnessLevel, PostgresPoolConfig, and SocialInsightsConfig
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/// PostgreSQL connection pool configuration
pub mod database;

/// Fitness-specific configuration for training zones, thresholds, and sport parameters
pub mod fitness;

/// User profile configuration including FitnessLevel
pub mod profiles;

/// Social insights configuration for milestones, streaks, and relevance scoring
pub mod social;

pub use fitness::FitnessConfig;
