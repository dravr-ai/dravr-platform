// ABOUTME: Mobility domain types for stretching exercises and yoga poses
// ABOUTME: Data models for recovery training recommendations and activity-muscle mappings
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ============================================================================
// Stretching Exercise Types
// ============================================================================

/// Category of stretching exercise
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum StretchingCategory {
    /// Held positions for flexibility improvement
    #[default]
    Static,
    /// Movement-based stretches for warmup
    Dynamic,
    /// Proprioceptive neuromuscular facilitation
    Pnf,
    /// Bouncing or momentum-based stretches
    Ballistic,
}

impl StretchingCategory {
    /// Convert to database string representation
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Static => "static",
            Self::Dynamic => "dynamic",
            Self::Pnf => "pnf",
            Self::Ballistic => "ballistic",
        }
    }

    /// Parse from database string representation
    #[must_use]
    pub fn parse(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "dynamic" => Self::Dynamic,
            "pnf" => Self::Pnf,
            "ballistic" => Self::Ballistic,
            // Default to Static for unrecognized values
            _ => Self::Static,
        }
    }
}

/// Difficulty level for exercises and poses
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DifficultyLevel {
    /// Suitable for beginners with no prior experience
    #[default]
    Beginner,
    /// Requires some practice and flexibility
    Intermediate,
    /// For experienced practitioners
    Advanced,
}

impl DifficultyLevel {
    /// Convert to database string representation
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Beginner => "beginner",
            Self::Intermediate => "intermediate",
            Self::Advanced => "advanced",
        }
    }

    /// Parse from database string representation
    #[must_use]
    pub fn parse(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "intermediate" => Self::Intermediate,
            "advanced" => Self::Advanced,
            // Default to Beginner for unrecognized values
            _ => Self::Beginner,
        }
    }
}

/// A stretching exercise for recovery and flexibility
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StretchingExercise {
    /// Unique identifier
    pub id: String,
    /// Exercise name
    pub name: String,
    /// Detailed description
    pub description: String,
    /// Category of stretch
    pub category: StretchingCategory,
    /// Difficulty level
    pub difficulty: DifficultyLevel,
    /// Primary muscles targeted
    pub primary_muscles: Vec<String>,
    /// Secondary muscles involved
    pub secondary_muscles: Vec<String>,
    /// Hold duration in seconds
    pub duration_seconds: u32,
    /// Number of repetitions (for dynamic stretches)
    pub repetitions: Option<u32>,
    /// Number of sets
    pub sets: u32,
    /// Activity types this stretch is recommended for
    pub recommended_for_activities: Vec<String>,
    /// Conditions where this stretch should be avoided
    pub contraindications: Vec<String>,
    /// Step-by-step instructions
    pub instructions: Vec<String>,
    /// Form cues and tips
    pub cues: Vec<String>,
    /// Optional image URL
    pub image_url: Option<String>,
    /// Optional video URL
    pub video_url: Option<String>,
    /// Creation timestamp
    pub created_at: DateTime<Utc>,
    /// Last update timestamp
    pub updated_at: DateTime<Utc>,
}

/// Filter options for listing stretching exercises
#[derive(Debug, Clone, Default)]
pub struct ListStretchingFilter {
    /// Filter by category
    pub category: Option<StretchingCategory>,
    /// Filter by difficulty
    pub difficulty: Option<DifficultyLevel>,
    /// Filter by muscle group
    pub muscle_group: Option<String>,
    /// Filter by activity type
    pub activity_type: Option<String>,
    /// Maximum number of results
    pub limit: Option<u32>,
    /// Offset for pagination
    pub offset: Option<u32>,
}

// ============================================================================
// Yoga Pose Types
// ============================================================================

/// Category of yoga pose
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum YogaCategory {
    /// Standing poses for strength and grounding
    #[default]
    Standing,
    /// Seated poses for hip opening and forward folds
    Seated,
    /// Lying face-up for relaxation and stretching
    Supine,
    /// Lying face-down for backbends
    Prone,
    /// Head below heart for circulation
    Inversion,
    /// Single-leg or arm balance poses
    Balance,
    /// Spinal rotation poses
    Twist,
}

impl YogaCategory {
    /// Convert to database string representation
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Standing => "standing",
            Self::Seated => "seated",
            Self::Supine => "supine",
            Self::Prone => "prone",
            Self::Inversion => "inversion",
            Self::Balance => "balance",
            Self::Twist => "twist",
        }
    }

    /// Parse from database string representation
    #[must_use]
    pub fn parse(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "seated" => Self::Seated,
            "supine" => Self::Supine,
            "prone" => Self::Prone,
            "inversion" => Self::Inversion,
            "balance" => Self::Balance,
            "twist" => Self::Twist,
            // Default to Standing for unrecognized values
            _ => Self::Standing,
        }
    }
}

/// Type of yoga pose focus
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum YogaPoseType {
    /// Primary focus on flexibility and stretching
    #[default]
    Stretch,
    /// Primary focus on building strength
    Strength,
    /// Primary focus on balance and stability
    Balance,
    /// Primary focus on calming and restoration
    Relaxation,
    /// Primary focus on breath control
    Breathing,
}

impl YogaPoseType {
    /// Convert to database string representation
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Stretch => "stretch",
            Self::Strength => "strength",
            Self::Balance => "balance",
            Self::Relaxation => "relaxation",
            Self::Breathing => "breathing",
        }
    }

    /// Parse from database string representation
    #[must_use]
    pub fn parse(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "strength" => Self::Strength,
            "balance" => Self::Balance,
            "relaxation" => Self::Relaxation,
            "breathing" => Self::Breathing,
            // Default to Stretch for unrecognized values
            _ => Self::Stretch,
        }
    }
}

/// A yoga pose for recovery and flexibility
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YogaPose {
    /// Unique identifier
    pub id: String,
    /// English name of the pose
    pub english_name: String,
    /// Sanskrit name (optional)
    pub sanskrit_name: Option<String>,
    /// Detailed description
    pub description: String,
    /// Benefits of the pose
    pub benefits: Vec<String>,
    /// Category of pose
    pub category: YogaCategory,
    /// Difficulty level
    pub difficulty: DifficultyLevel,
    /// Type of pose focus
    pub pose_type: YogaPoseType,
    /// Primary muscles targeted
    pub primary_muscles: Vec<String>,
    /// Secondary muscles involved
    pub secondary_muscles: Vec<String>,
    /// Chakras associated (optional)
    pub chakras: Vec<String>,
    /// Hold duration in seconds
    pub hold_duration_seconds: u32,
    /// Breath guidance instructions
    pub breath_guidance: Option<String>,
    /// Activity types this pose is recommended for
    pub recommended_for_activities: Vec<String>,
    /// Recovery contexts (`post_cardio`, `rest_day`, morning)
    pub recommended_for_recovery: Vec<String>,
    /// Conditions where this pose should be avoided
    pub contraindications: Vec<String>,
    /// Step-by-step instructions
    pub instructions: Vec<String>,
    /// Easier variations
    pub modifications: Vec<String>,
    /// Harder variations
    pub progressions: Vec<String>,
    /// Alignment cues
    pub cues: Vec<String>,
    /// Poses that should precede this one
    pub warmup_poses: Vec<String>,
    /// Poses that should follow this one
    pub followup_poses: Vec<String>,
    /// Optional image URL
    pub image_url: Option<String>,
    /// Optional video URL
    pub video_url: Option<String>,
    /// Creation timestamp
    pub created_at: DateTime<Utc>,
    /// Last update timestamp
    pub updated_at: DateTime<Utc>,
}

/// Filter options for listing yoga poses
#[derive(Debug, Clone, Default)]
pub struct ListYogaFilter {
    /// Filter by category
    pub category: Option<YogaCategory>,
    /// Filter by difficulty
    pub difficulty: Option<DifficultyLevel>,
    /// Filter by pose type
    pub pose_type: Option<YogaPoseType>,
    /// Filter by muscle group
    pub muscle_group: Option<String>,
    /// Filter by activity type
    pub activity_type: Option<String>,
    /// Filter by recovery context
    pub recovery_context: Option<String>,
    /// Maximum number of results
    pub limit: Option<u32>,
    /// Offset for pagination
    pub offset: Option<u32>,
}

// ============================================================================
// Activity-Muscle Mapping
// ============================================================================

/// Mapping of activity type to muscle stress levels
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivityMuscleMapping {
    /// Unique identifier
    pub id: String,
    /// Activity type (running, cycling, etc.)
    pub activity_type: String,
    /// Primary muscles with stress levels (1-10)
    pub primary_muscles: HashMap<String, u8>,
    /// Secondary muscles with stress levels (1-10)
    pub secondary_muscles: HashMap<String, u8>,
    /// Recommended stretch categories
    pub recommended_stretch_categories: Vec<String>,
    /// Recommended yoga categories
    pub recommended_yoga_categories: Vec<String>,
    /// Creation timestamp
    pub created_at: DateTime<Utc>,
    /// Last update timestamp
    pub updated_at: DateTime<Utc>,
}
