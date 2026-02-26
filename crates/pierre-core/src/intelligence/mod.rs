// ABOUTME: Intelligence module re-exports for algorithm types and policies
// ABOUTME: Contains MaxHrAlgorithm and InsightSharingPolicy used by models
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/// Maximum heart rate estimation algorithms
pub mod algorithms;

/// User fitness profile domain types
mod fitness_profile;

/// Insight sharing policy for social features
mod insight_sharing_policy;

pub use fitness_profile::{FitnessLevel, TimeAvailability, UserFitnessProfile, UserPreferences};
pub use insight_sharing_policy::InsightSharingPolicy;
