// ABOUTME: Coach data types shared across crates for AI persona definitions
// ABOUTME: Includes coach structs, categories, visibility, and request/response types
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

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
    /// System prompt that shapes AI responses
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
