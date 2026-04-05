// ABOUTME: Coach data types shared across crates for AI persona definitions
// ABOUTME: Includes coach structs, categories, visibility, and request/response types
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Activity data requirements for coach startup context assembly
///
/// Specifies exactly what activity data a coach needs pre-fetched
/// before the first conversation turn, enabling deterministic data
/// retrieval instead of relying on LLM interpretation of natural language.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ActivityDataRequirements {
    /// Number of activities to fetch
    pub count: u32,

    /// Sport types to filter by (empty = all types)
    /// Values match Strava sport types: Run, Ride, Swim, etc.
    #[serde(default)]
    pub sport_types: Vec<String>,

    /// Lookback time frame (e.g., "16w", "90d", "3m")
    /// Parsed as: number + unit where unit is w(eeks), d(ays), m(onths)
    #[serde(default)]
    pub time_frame: Option<String>,

    /// Data detail level: "summary" (default) or "detailed"
    /// Detailed mode includes lap splits, GPS data — needed for interval analysis
    #[serde(default = "ActivityDataRequirements::default_mode")]
    pub mode: String,

    /// Output format: "toon" (token-efficient, default) or "json"
    #[serde(default = "ActivityDataRequirements::default_format")]
    pub format: String,

    /// Analysis type for data sufficiency guidance.
    /// Values: `general_overview`, `weekly_summary`, `trend_analysis`,
    /// `race_preparation`, `recovery_assessment`
    #[serde(default = "ActivityDataRequirements::default_analysis_type")]
    pub analysis_type: String,
}

impl ActivityDataRequirements {
    fn default_mode() -> String {
        "summary".to_owned()
    }

    fn default_format() -> String {
        "toon".to_owned()
    }

    fn default_analysis_type() -> String {
        "general_overview".to_owned()
    }

    /// Parse `time_frame` string into seconds from now
    ///
    /// Supports: "16w" (weeks), "90d" (days), "3m" (months, 30 days each)
    #[must_use]
    pub fn time_frame_seconds(&self) -> Option<i64> {
        let tf = self.time_frame.as_ref()?;
        let tf = tf.trim();
        if tf.len() < 2 {
            return None;
        }

        let (num_str, unit) = tf.split_at(tf.len() - 1);
        let num: i64 = num_str.parse().ok()?;

        let seconds_per_day: i64 = 86_400;
        match unit {
            "d" => Some(num * seconds_per_day),
            "w" => Some(num * 7 * seconds_per_day),
            "m" => Some(num * 30 * seconds_per_day),
            _ => None,
        }
    }
}

/// Structured data requirements for coach startup context assembly
///
/// Defines what data should be pre-fetched before the coach conversation starts,
/// separating deterministic data fetching from creative LLM analysis.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DataRequirements {
    /// Activity data to pre-fetch
    #[serde(default)]
    pub activities: Option<ActivityDataRequirements>,

    /// Whether to also fetch the athlete profile
    #[serde(default)]
    pub athlete_profile: bool,
}

/// Prerequisites required to use a coach
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CoachPrerequisites {
    /// Required OAuth providers (e.g., strava, garmin)
    #[serde(default)]
    pub providers: Vec<String>,

    /// Minimum number of activities required
    #[serde(default)]
    pub min_activities: u32,

    /// Required activity types (e.g., Run, Ride, Swim)
    #[serde(default)]
    pub activity_types: Vec<String>,
}

/// Coach visibility for access control
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CoachVisibility {
    /// Only visible to the owner
    #[default]
    Private,
    /// Visible to all users in the tenant
    Tenant,
    /// Visible across all tenants (super-admin only)
    Global,
}

/// Coach publish status for Store workflow
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PublishStatus {
    /// Not submitted for review (default)
    #[default]
    Draft,
    /// Submitted and waiting for admin approval
    PendingReview,
    /// Approved and visible in Store
    Published,
    /// Rejected by admin (reason provided)
    Rejected,
}

impl PublishStatus {
    /// Convert to database string representation
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::PendingReview => "pending_review",
            Self::Published => "published",
            Self::Rejected => "rejected",
        }
    }

    /// Parse from database string representation
    #[must_use]
    pub fn parse(s: &str) -> Self {
        match s {
            "pending_review" => Self::PendingReview,
            "published" => Self::Published,
            "rejected" => Self::Rejected,
            _ => Self::Draft,
        }
    }

    /// Check if coach is visible in the Store
    #[must_use]
    pub const fn is_published(&self) -> bool {
        matches!(self, Self::Published)
    }
}

impl CoachVisibility {
    /// Convert to database string representation
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Private => "private",
            Self::Tenant => "tenant",
            Self::Global => "global",
        }
    }

    /// Parse from database string representation
    #[must_use]
    pub fn parse(s: &str) -> Self {
        match s {
            "tenant" => Self::Tenant,
            "global" => Self::Global,
            _ => Self::Private,
        }
    }
}

/// Coach category for organization
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CoachCategory {
    /// Training and workout focused coaches
    Training,
    /// Nutrition and diet focused coaches
    Nutrition,
    /// Recovery and rest focused coaches
    Recovery,
    /// Recipe and meal planning focused coaches
    Recipes,
    /// Mobility, stretching, and yoga focused coaches
    Mobility,
    /// Analysis and insights focused coaches
    Analysis,
    /// User-defined custom category
    #[default]
    Custom,
}

impl CoachCategory {
    /// Convert to database string representation
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Training => "training",
            Self::Nutrition => "nutrition",
            Self::Recovery => "recovery",
            Self::Recipes => "recipes",
            Self::Mobility => "mobility",
            Self::Analysis => "analysis",
            Self::Custom => "custom",
        }
    }

    /// Parse from database string representation (case-insensitive)
    #[must_use]
    pub fn parse(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "training" => Self::Training,
            "nutrition" => Self::Nutrition,
            "recovery" => Self::Recovery,
            "recipes" => Self::Recipes,
            "mobility" => Self::Mobility,
            "analysis" => Self::Analysis,
            _ => Self::Custom,
        }
    }

    /// Human-readable display name for UI
    #[must_use]
    pub const fn display_name(&self) -> &'static str {
        match self {
            Self::Training => "Training",
            Self::Nutrition => "Nutrition",
            Self::Recovery => "Recovery",
            Self::Recipes => "Recipes",
            Self::Mobility => "Mobility",
            Self::Analysis => "Analysis",
            Self::Custom => "Custom",
        }
    }
}

/// A Coach is a custom AI persona with a system prompt
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Coach {
    /// Unique identifier
    pub id: Uuid,
    /// User who created the coach (admin user for system coaches)
    pub user_id: Uuid,
    /// Tenant for multi-tenancy isolation
    pub tenant_id: String,
    /// Display title for the coach
    pub title: String,
    /// Optional description explaining the coach's purpose
    pub description: Option<String>,
    /// System prompt that shapes AI responses
    pub system_prompt: String,
    /// Category for organization
    pub category: CoachCategory,
    /// Tags for filtering and search (stored as JSON array)
    pub tags: Vec<String>,
    /// Sample prompts for quick-start suggestions (stored as JSON array)
    #[serde(default)]
    pub sample_prompts: Vec<String>,
    /// Estimated token count of system prompt
    pub token_count: u32,
    /// Creation timestamp
    pub created_at: DateTime<Utc>,
    /// Last update timestamp
    pub updated_at: DateTime<Utc>,
    /// Whether this is a system coach (admin-created)
    #[serde(default)]
    pub is_system: bool,
    /// Visibility level for the coach
    #[serde(default)]
    pub visibility: CoachVisibility,
    /// Prerequisites required to use this coach (providers, activities, etc.)
    #[serde(default)]
    pub prerequisites: CoachPrerequisites,
    /// ID of the coach this was forked from (None for original coaches)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub forked_from: Option<String>,
    /// Maximum tool call iterations for this coach (overrides global config)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tool_iterations: Option<i32>,
    /// Query auto-sent on first message to provide analysis context
    #[serde(skip_serializing_if = "Option::is_none")]
    pub startup_query: Option<String>,
    /// Structured data requirements for deterministic activity pre-fetching
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_requirements: Option<DataRequirements>,

    // -- Structured sections (populated for system coaches and structured user coaches) --
    /// Coach purpose/description extracted from ## Purpose section
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
    /// Usage scenarios extracted from ## When to Use section (not counted in tokens)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub when_to_use: Option<String>,
    /// Core AI instructions extracted from ## Instructions section
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    /// Sample questions extracted from ## Example Inputs section
    #[serde(skip_serializing_if = "Option::is_none")]
    pub example_inputs: Option<String>,
    /// Response style guidance extracted from ## Example Outputs section
    #[serde(skip_serializing_if = "Option::is_none")]
    pub example_outputs: Option<String>,
    /// Success definition extracted from ## Success Criteria section
    #[serde(skip_serializing_if = "Option::is_none")]
    pub success_criteria: Option<String>,
    /// Origin of this coach: "contremaitre" (git-managed), "seed" (seeded from files), "custom" (user/admin created)
    #[serde(default = "default_source")]
    pub source: String,
}

fn default_source() -> String {
    "custom".to_owned()
}

/// Token estimation constant: average characters per token for system prompts
const CHARS_PER_TOKEN: usize = 4;

impl Coach {
    /// Calculate section-aware token count when structured fields are available.
    ///
    /// Counted sections: `purpose`, `instructions`, `example_inputs`, `example_outputs`, `success_criteria`.
    /// Not counted: `when_to_use` (UI-only metadata).
    /// Falls back to `system_prompt` length when no structured sections are present.
    #[must_use]
    pub fn compute_token_count(&self) -> u32 {
        if self.instructions.is_some() || self.purpose.is_some() {
            let total_chars = self.purpose.as_ref().map_or(0, String::len)
                + self.instructions.as_ref().map_or(0, String::len)
                + self.example_inputs.as_ref().map_or(0, String::len)
                + self.example_outputs.as_ref().map_or(0, String::len)
                + self.success_criteria.as_ref().map_or(0, String::len);
            #[allow(clippy::cast_possible_truncation)]
            let count = (total_chars / CHARS_PER_TOKEN) as u32;
            count
        } else {
            #[allow(clippy::cast_possible_truncation)]
            let count = (self.system_prompt.len() / CHARS_PER_TOKEN) as u32;
            count
        }
    }
}

/// Coach with computed context-dependent fields for list responses
///
/// Per-user preferences (`is_favorite`, `is_active`, `use_count`, `last_used_at`) are
/// sourced from the `coach_assignments` table, not the `coaches` table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoachListItem {
    /// The coach data
    #[serde(flatten)]
    pub coach: Coach,
    /// Whether this coach is assigned to the current user (computed from query)
    pub is_assigned: bool,
    /// Whether this coach is marked as favorite by the current user
    pub is_favorite: bool,
    /// Whether this coach is currently active for the user
    pub is_active: bool,
    /// Number of times the current user has used this coach
    pub use_count: u32,
    /// Last time the current user used this coach
    pub last_used_at: Option<DateTime<Utc>>,
}

/// Request to create a new coach
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateCoachRequest {
    /// Display title for the coach
    pub title: String,
    /// Optional description explaining the coach's purpose
    pub description: Option<String>,
    /// System prompt that shapes AI responses.
    /// When `instructions` is provided, `system_prompt` is derived from it at the DB layer.
    pub system_prompt: String,
    /// Category for organization
    #[serde(default)]
    pub category: CoachCategory,
    /// Tags for filtering and search
    #[serde(default)]
    pub tags: Vec<String>,
    /// Sample prompts for quick-start suggestions
    #[serde(default)]
    pub sample_prompts: Vec<String>,
    /// Query auto-sent on first message to provide analysis context
    #[serde(default)]
    pub startup_query: Option<String>,
    /// Structured data requirements for deterministic activity pre-fetching
    #[serde(default)]
    pub data_requirements: Option<DataRequirements>,

    // -- Structured sections (optional, for marketplace-quality coaches) --
    /// Coach purpose (from ## Purpose section)
    #[serde(default)]
    pub purpose: Option<String>,
    /// Usage scenarios (from ## When to Use section)
    #[serde(default)]
    pub when_to_use: Option<String>,
    /// Core AI instructions (from ## Instructions section).
    /// When provided, this overrides `system_prompt` for runtime use.
    #[serde(default)]
    pub instructions: Option<String>,
    /// Sample questions (from ## Example Inputs section)
    #[serde(default)]
    pub example_inputs: Option<String>,
    /// Response style guidance (from ## Example Outputs section)
    #[serde(default)]
    pub example_outputs: Option<String>,
    /// Success definition (from ## Success Criteria section)
    #[serde(default)]
    pub success_criteria: Option<String>,
}

/// Request to update an existing coach
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateCoachRequest {
    /// New display title (if provided)
    pub title: Option<String>,
    /// New description (if provided)
    pub description: Option<String>,
    /// New system prompt (if provided)
    pub system_prompt: Option<String>,
    /// New category (if provided)
    pub category: Option<CoachCategory>,
    /// New tags (if provided)
    pub tags: Option<Vec<String>>,
    /// New sample prompts (if provided)
    pub sample_prompts: Option<Vec<String>>,
    /// New startup query (if provided, set to empty string to clear)
    pub startup_query: Option<String>,
    /// New data requirements (if provided)
    pub data_requirements: Option<DataRequirements>,

    // -- Structured sections (optional) --
    /// New `purpose` (if provided)
    pub purpose: Option<String>,
    /// New `when_to_use` (if provided)
    pub when_to_use: Option<String>,
    /// New `instructions` (if provided). When set, also updates `system_prompt`.
    pub instructions: Option<String>,
    /// New `example_inputs` (if provided)
    pub example_inputs: Option<String>,
    /// New `example_outputs` (if provided)
    pub example_outputs: Option<String>,
    /// New `success_criteria` (if provided)
    pub success_criteria: Option<String>,
}

/// Filter options for listing coaches
#[derive(Debug, Clone, Default)]
pub struct ListCoachesFilter {
    /// Filter by category
    pub category: Option<CoachCategory>,
    /// Filter to favorites only
    pub favorites_only: bool,
    /// Maximum number of results
    pub limit: Option<u32>,
    /// Offset for pagination
    pub offset: Option<u32>,
    /// Include system coaches (default: true)
    pub include_system: bool,
    /// Include hidden coaches (default: false)
    pub include_hidden: bool,
}

impl ListCoachesFilter {
    /// Create a filter with sensible defaults (include system coaches, exclude hidden)
    #[must_use]
    pub fn with_defaults() -> Self {
        Self {
            include_system: true,
            include_hidden: false,
            ..Default::default()
        }
    }
}

/// Coach assignment info
#[derive(Debug, Clone, serde::Serialize)]
pub struct CoachAssignment {
    /// User ID
    pub user_id: String,
    /// User email (for display)
    pub user_email: Option<String>,
    /// When assigned
    pub assigned_at: String,
    /// Who assigned
    pub assigned_by: Option<String>,
}

/// Store admin statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoreAdminStats {
    /// Number of coaches pending review
    pub pending_count: u32,
    /// Number of published coaches
    pub published_count: u32,
    /// Number of rejected coaches
    pub rejected_count: u32,
    /// Total installs across all published coaches
    pub total_installs: u32,
    /// Rejection rate as percentage
    pub rejection_rate: f64,
}

/// Request to create a system coach
pub struct CreateSystemCoachRequest {
    /// Display title
    pub title: String,
    /// Description
    pub description: Option<String>,
    /// System prompt
    pub system_prompt: String,
    /// Category
    pub category: CoachCategory,
    /// Tags
    pub tags: Vec<String>,
    /// Sample prompts for quick-start suggestions
    pub sample_prompts: Vec<String>,
    /// Visibility
    pub visibility: CoachVisibility,
}

/// A snapshot of a coach at a specific version
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoachVersion {
    /// Unique identifier for this version
    pub id: String,
    /// Reference to the coach
    pub coach_id: String,
    /// Version number (incremented on each update)
    pub version: i32,
    /// Content hash at this version (SHA-256 of serialized content)
    pub content_hash: String,
    /// Full content snapshot as JSON
    pub content_snapshot: serde_json::Value,
    /// Summary of what changed in this version
    pub change_summary: Option<String>,
    /// When this version was created
    pub created_at: DateTime<Utc>,
    /// User who created this version
    pub created_by: Option<Uuid>,
}
