// ABOUTME: Domain types for user fitness profiles, preferences, and time availability
// ABOUTME: Shared between database and intelligence crates to avoid layering violations
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use serde::{Deserialize, Serialize};

/// User fitness profile containing physical attributes and training preferences
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserFitnessProfile {
    /// Unique user identifier
    pub user_id: String,
    /// User's age in years
    pub age: Option<i32>,
    /// User's gender
    pub gender: Option<String>,
    /// User's weight in kilograms
    pub weight: Option<f64>,
    /// User's height in centimeters
    pub height: Option<f64>,
    /// Current fitness level
    pub fitness_level: FitnessLevel,
    /// List of sports the user primarily participates in
    pub primary_sports: Vec<String>,
    /// Months of training history
    pub training_history_months: i32,
    /// User's training preferences and constraints
    pub preferences: UserPreferences,
}

/// Fitness level classification
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FitnessLevel {
    /// New to training, building base fitness
    Beginner,
    /// Some training experience, consistent activity
    Intermediate,
    /// Experienced athlete with solid training background
    Advanced,
    /// Elite/professional level athlete
    Elite,
}

/// User preferences for training and analysis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserPreferences {
    /// Preferred units (metric/imperial)
    pub preferred_units: String,
    /// Areas the user wants to focus training on
    pub training_focus: Vec<String>,
    /// History of injuries to consider
    pub injury_history: Vec<String>,
    /// Available time for training
    pub time_availability: TimeAvailability,
}

/// Available time for training
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeAvailability {
    /// Total hours available per week
    pub hours_per_week: f64,
    /// Preferred days for training
    pub preferred_days: Vec<String>,
    /// Preferred session duration in minutes
    pub preferred_duration_minutes: Option<i32>,
}
