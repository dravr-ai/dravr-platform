// ABOUTME: Coaching group models for multi-person AI coaching
// ABOUTME: Groups, members, invites, roles, and analytics types
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Role within a coaching group
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum GroupRole {
    /// Group creator with full control
    Owner,
    /// Elevated permissions (manage members, settings)
    Admin,
    /// Standard group member
    Member,
}

impl GroupRole {
    /// String representation for database storage
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Admin => "admin",
            Self::Member => "member",
        }
    }

    /// Parse from database string
    #[must_use]
    pub fn from_str_opt(s: &str) -> Option<Self> {
        match s {
            "owner" => Some(Self::Owner),
            "admin" => Some(Self::Admin),
            "member" => Some(Self::Member),
            _ => None,
        }
    }

    /// Whether this role can manage members (add/remove/promote)
    #[must_use]
    pub const fn can_manage_members(&self) -> bool {
        matches!(self, Self::Owner | Self::Admin)
    }

    /// Whether this role can modify group settings
    #[must_use]
    pub const fn can_modify_settings(&self) -> bool {
        matches!(self, Self::Owner | Self::Admin)
    }

    /// Whether this role can delete the group
    #[must_use]
    pub const fn can_delete_group(&self) -> bool {
        matches!(self, Self::Owner)
    }
}

impl fmt::Display for GroupRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A coaching group binding a coach persona to multiple athletes
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CoachingGroup {
    /// Unique group identifier
    pub id: Uuid,
    /// Tenant for multi-tenant isolation
    pub tenant_id: String,
    /// Human-readable group name
    pub name: String,
    /// Optional description of the group's purpose
    pub description: Option<String>,
    /// Coach persona assigned to this group
    pub coach_id: String,
    /// User who created and owns the group
    pub owner_id: Uuid,
    /// Whether peer data sharing is enabled for this group
    pub peer_data_sharing: bool,
    /// Maximum allowed members
    pub max_members: i32,
    /// Whether the group is active
    pub is_active: bool,
    /// When the group was created
    pub created_at: DateTime<Utc>,
    /// When the group was last updated
    pub updated_at: DateTime<Utc>,
}

/// A member within a coaching group
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct GroupMember {
    /// Unique membership record identifier
    pub id: Uuid,
    /// Group this membership belongs to
    pub group_id: Uuid,
    /// User who is a member
    pub user_id: Uuid,
    /// Tenant for multi-tenant isolation
    pub tenant_id: String,
    /// Role within the group
    pub role: GroupRole,
    /// Whether this member consents to peer data sharing
    pub peer_sharing_consent: bool,
    /// When consent was given (audit timestamp)
    pub consent_given_at: DateTime<Utc>,
    /// When the member joined
    pub joined_at: DateTime<Utc>,
    /// When the member left (None = still active)
    pub left_at: Option<DateTime<Utc>>,
    /// Display name (populated from user profile, not stored)
    #[serde(default)]
    pub display_name: Option<String>,
}

/// An invite code for joining a group
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct GroupInvite {
    /// Unique invite identifier
    pub id: Uuid,
    /// Group this invite belongs to
    pub group_id: Uuid,
    /// Tenant for multi-tenant isolation
    pub tenant_id: String,
    /// The invite code (8-char alphanumeric)
    pub code: String,
    /// User who created the invite
    pub created_by: Uuid,
    /// When the invite expires (None = never)
    pub expires_at: Option<DateTime<Utc>>,
    /// Maximum number of uses (None = unlimited)
    pub max_uses: Option<i32>,
    /// Current number of uses
    pub use_count: i32,
    /// Whether the invite is active
    pub is_active: bool,
    /// When the invite was created
    pub created_at: DateTime<Utc>,
}

// ============================================================================
// Request Types
// ============================================================================

/// Request to create a new coaching group
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CreateGroupRequest {
    /// Group name
    pub name: String,
    /// Optional description
    pub description: Option<String>,
    /// Coach persona to assign
    pub coach_id: String,
    /// Maximum members (defaults to 20)
    pub max_members: Option<i32>,
}

/// Request to update a coaching group
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UpdateGroupRequest {
    /// Updated name
    pub name: Option<String>,
    /// Updated description
    pub description: Option<String>,
    /// Updated max members
    pub max_members: Option<i32>,
    /// Toggle peer data sharing
    pub peer_data_sharing: Option<bool>,
    /// Toggle active status
    pub is_active: Option<bool>,
}

/// Request to join a group via invite code
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct JoinGroupRequest {
    /// The invite code
    pub invite_code: String,
}

// ============================================================================
// Response / Summary Types
// ============================================================================

/// Lightweight group summary for list views
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct GroupSummary {
    /// Group ID
    pub id: Uuid,
    /// Group name
    pub name: String,
    /// Optional description
    pub description: Option<String>,
    /// Coach persona ID
    pub coach_id: String,
    /// Number of active members
    pub member_count: i64,
    /// Whether the group is active
    pub is_active: bool,
    /// Whether peer data sharing is enabled
    pub peer_data_sharing: bool,
    /// The requesting user's role in this group
    pub my_role: GroupRole,
    /// When the group was created
    pub created_at: DateTime<Utc>,
}

// ============================================================================
// Analytics Types (used by pierre-groups strategies)
// ============================================================================

/// Pre-computed fitness snapshot for a group member
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct MemberFitnessSnapshot {
    /// Member's user ID
    pub user_id: Uuid,
    /// Display name
    pub display_name: String,
    /// Chronic Training Load
    pub ctl: Option<f64>,
    /// Acute Training Load
    pub atl: Option<f64>,
    /// Training Stress Balance (CTL - ATL)
    pub tsb: Option<f64>,
    /// Weekly volume in kilometers
    pub weekly_volume_km: f64,
    /// Previous week's volume in kilometers (for trend calculation)
    pub previous_week_volume_km: Option<f64>,
    /// Number of activities this week
    pub weekly_activity_count: i32,
    /// Primary sport type
    pub primary_sport: Option<String>,
    /// VDOT (running fitness estimator)
    pub vdot: Option<f64>,
    /// Risk level for overtraining
    pub overtraining_risk: OvertrainingRiskLevel,
    /// Days since last activity
    pub days_since_last_activity: Option<i32>,
    /// When this snapshot was computed
    pub computed_at: DateTime<Utc>,
}

/// Overtraining risk level for a member
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum OvertrainingRiskLevel {
    /// No risk detected
    #[default]
    Low,
    /// Moderate risk, monitor closely
    Moderate,
    /// High risk, recommend recovery
    High,
}

/// Token-efficient summary card for LLM system prompt injection
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct MemberSummaryCard {
    /// Member's user ID
    pub user_id: Uuid,
    /// Display name
    pub display_name: String,
    /// Compact text summary for LLM context
    pub summary_text: String,
    /// Estimated token count for this card
    pub estimated_tokens: usize,
    /// Detail level used
    pub detail_level: SummaryDetailLevel,
    /// Alert flags for this member
    pub flags: Vec<MemberFlag>,
}

/// Detail level for member summary generation
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum SummaryDetailLevel {
    /// Minimal one-line roster card (~50 tokens)
    Roster,
    /// Weekly digest with key workouts (~200 tokens)
    Weekly,
    /// Full activity details (~500+ tokens)
    Detailed,
}

/// Alert flag for a group member
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum MemberFlag {
    /// TSB below threshold, high fatigue
    Overreaching,
    /// TSB positive, ready for hard training
    FreshForm,
    /// Set a personal record recently
    PersonalRecord,
    /// Injury risk indicators
    InjuryRisk,
    /// No activity for extended period
    Inactive,
    /// Weekly volume dropped significantly from baseline
    VolumeDrop,
}

/// Formatted group summary block for LLM injection
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct GroupSummaryBlock {
    /// Full text to inject into system prompt
    pub text: String,
    /// Number of members included
    pub member_count: usize,
    /// Estimated total tokens
    pub estimated_tokens: usize,
}

/// Aggregate statistics for a coaching group
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct GroupAggregateStats {
    /// Total members in the group
    pub total_members: i64,
    /// Members active in the period
    pub active_members: i64,
    /// Average weekly volume in km
    pub avg_weekly_volume_km: f64,
    /// Average CTL across members
    pub avg_ctl: Option<f64>,
    /// Members with overtraining risk
    pub flagged_members: i64,
    /// Overall group trend
    pub weekly_trend: GroupTrend,
}

/// Direction of a group-level metric trend
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum GroupTrend {
    /// Volume/fitness improving
    Improving,
    /// Holding steady
    #[default]
    Stable,
    /// Volume/fitness declining
    Declining,
}

/// Comparison of one member against group norms
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct MemberGroupComparison {
    /// User being compared
    pub user_id: Uuid,
    /// Display name
    pub display_name: String,
    /// Percentile rank in the group (0-100)
    pub volume_percentile: f64,
    /// Percentile rank for CTL
    pub ctl_percentile: Option<f64>,
    /// Human-readable comparison text
    pub comparison_text: String,
}

/// Health flag for a group member needing attention
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct GroupHealthFlag {
    /// User who is flagged
    pub user_id: Uuid,
    /// Display name
    pub display_name: String,
    /// Type of flag
    pub flag_type: MemberFlag,
    /// Severity (informational, warning, critical)
    pub severity: HealthFlagSeverity,
    /// Human-readable detail
    pub detail: String,
}

/// Severity level for health flags
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum HealthFlagSeverity {
    /// Informational, no action needed
    Info,
    /// Should be monitored
    Warning,
    /// Requires immediate attention
    Critical,
}

/// Weekly report for a coaching group
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct GroupWeeklyReport {
    /// Summary text
    pub summary: String,
    /// Notable achievements
    pub highlights: Vec<String>,
    /// Concerns requiring attention
    pub concerns: Vec<String>,
    /// Recommended actions
    pub recommendations: Vec<String>,
    /// Stats for the reporting period
    pub stats: GroupAggregateStats,
}

/// Context passed to strategy traits for prompt building
#[derive(Debug, Clone)]
pub struct GroupContext {
    /// The coaching group
    pub group: CoachingGroup,
    /// Total active member count
    pub member_count: usize,
    /// Members active in the current period
    pub active_count: usize,
    /// Whether the requesting user is admin/owner
    pub requester_is_admin: bool,
    /// The requesting user's ID
    pub requester_user_id: Uuid,
}
