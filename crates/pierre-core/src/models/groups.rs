// ABOUTME: Coaching group models for multi-person AI coaching
// ABOUTME: Groups, members, invites, roles, and analytics types
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashMap;
use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Role within a coaching group
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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

/// When the group's AI coach replies in the bound channel chat
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupRespondMode {
    /// Reply to every member message (the original behavior)
    #[default]
    All,
    /// Reply only when the bot is explicitly addressed: an @-mention or a
    /// reply to one of the bot's messages. Unaddressed member messages are
    /// captured as ambient conversation context but trigger no reply.
    Mentions,
}

impl GroupRespondMode {
    /// String representation for database storage
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Mentions => "mentions",
        }
    }

    /// Parse from database string
    #[must_use]
    pub fn from_str_opt(s: &str) -> Option<Self> {
        match s {
            "all" => Some(Self::All),
            "mentions" => Some(Self::Mentions),
            _ => None,
        }
    }
}

impl fmt::Display for GroupRespondMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl fmt::Display for GroupRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What redeeming a group invite grants the joining user.
///
/// Separate from [`GroupRole`]: a `Coach` invite does not create a
/// membership row at all — it attaches the redeemer as the group's human
/// coach via `coaching_groups.coach_user_id`. Keeping this distinct from
/// the member role enum means a human coach never counts against
/// `max_members` and never appears in the athlete roster.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum GroupInviteKind {
    /// Standard athlete membership (the default for every existing invite)
    #[default]
    Member,
    /// Attaches the redeemer as the group's human coach (`coach_user_id`)
    Coach,
}

impl GroupInviteKind {
    /// String representation for database storage
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Member => "member",
            Self::Coach => "coach",
        }
    }

    /// Parse from a database string, falling back to `Member` for unknowns
    #[must_use]
    pub fn from_str_opt(s: &str) -> Option<Self> {
        match s {
            "member" => Some(Self::Member),
            "coach" => Some(Self::Coach),
            _ => None,
        }
    }
}

impl fmt::Display for GroupInviteKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A coaching group binding a coach persona to multiple athletes
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    /// Human professional coach (a Dravr user) attached to oversee this
    /// group. `None` until a coach redeems a coach-kind invite. Distinct
    /// from `coach_id`, which is the AI coach persona that answers chats:
    /// the human coach reads the roster through that persona, gated by the
    /// same per-member `peer_sharing_consent`.
    pub coach_user_id: Option<Uuid>,
    /// Whether peer data sharing is enabled for this group
    pub peer_data_sharing: bool,
    /// When the AI coach replies in the bound channel chat: every message
    /// or only explicitly-addressed ones. Serde defaults keep payloads
    /// written before the field existed deserializable.
    #[serde(default)]
    pub respond_mode: GroupRespondMode,
    /// Maximum allowed members
    pub max_members: i32,
    /// Whether the group is active
    pub is_active: bool,
    /// Channel platform when the group was bootstrapped from a messaging
    /// chat (Telegram group, Slack channel, Discord channel). `None` for
    /// REST-created (web/mobile) groups.
    pub channel_type: Option<String>,
    /// Channel-native chat identifier when bootstrapped from a messaging
    /// chat (Telegram `chat_id`, Slack `channel_id`, Discord `channel_id`).
    /// `None` for REST-created groups.
    pub channel_chat_id: Option<String>,
    /// When the group was created
    pub created_at: DateTime<Utc>,
    /// When the group was last updated
    pub updated_at: DateTime<Utc>,
}

/// A member within a coaching group
#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// Who spoke in a group transcript entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TranscriptSpeaker {
    /// A group member's own words (a coaching-turn message or ambient room
    /// chatter).
    Member,
    /// The AI coach's reply, attributed to the member it answered.
    Coach,
}

impl TranscriptSpeaker {
    /// String representation for database storage
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Member => "member",
            Self::Coach => "coach",
        }
    }

    /// Parse from database string
    #[must_use]
    pub fn from_str_opt(s: &str) -> Option<Self> {
        match s {
            "member" => Some(Self::Member),
            "coach" => Some(Self::Coach),
            _ => None,
        }
    }
}

/// One utterance in a group's shared room transcript — the surface-neutral
/// read model behind the group chat view and the ambient prompt block.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupTranscriptEntry {
    /// Unique entry identifier
    pub id: Uuid,
    /// Group whose room this entry belongs to
    pub group_id: Uuid,
    /// Tenant the writing conversation/session lives under (audit; reads are
    /// keyed on `group_id` because membership is cross-tenant)
    pub tenant_id: String,
    /// The member this entry is attributed to: the speaker for `member`
    /// rows, the member the coach answered for `coach` rows
    pub author_user_id: Uuid,
    /// Who spoke
    pub speaker: TranscriptSpeaker,
    /// The utterance text
    pub content: String,
    /// The member conversation the row was fanned out from; `None` for
    /// ambient room chatter captured outside any turn
    pub source_conversation_id: Option<String>,
    /// Provenance id of the source row (`chat_messages.id` for turn rows,
    /// the channel-native message id for ambient rows)
    pub source_message_id: Option<String>,
    /// When the utterance was recorded
    pub created_at: DateTime<Utc>,
    /// Author display name (populated from user profile on read, not stored)
    #[serde(default)]
    pub author_display_name: Option<String>,
}

/// Parameters for appending one entry to a group's shared transcript.
#[derive(Debug, Clone, Copy)]
pub struct NewGroupTranscriptEntry<'a> {
    /// Group whose room the entry joins
    pub group_id: &'a str,
    /// Tenant the writing conversation/session lives under
    pub tenant_id: &'a str,
    /// The member this entry is attributed to (see
    /// [`GroupTranscriptEntry::author_user_id`])
    pub author_user_id: Uuid,
    /// Who spoke
    pub speaker: TranscriptSpeaker,
    /// The utterance text
    pub content: &'a str,
    /// The member conversation the row was fanned out from, when any
    pub source_conversation_id: Option<&'a str>,
    /// Provenance id of the source row, when any
    pub source_message_id: Option<&'a str>,
}

/// An invite code for joining a group
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupInvite {
    /// Unique invite identifier
    pub id: Uuid,
    /// Group this invite belongs to
    pub group_id: Uuid,
    /// Tenant for multi-tenant isolation
    pub tenant_id: String,
    /// The invite code (8-char alphanumeric)
    pub code: String,
    /// What redeeming this invite grants — athlete membership (default)
    /// or attachment as the group's human coach.
    pub kind: GroupInviteKind,
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
pub struct UpdateGroupRequest {
    /// Updated name
    pub name: Option<String>,
    /// Updated description
    pub description: Option<String>,
    /// Updated coach persona ID
    pub coach_id: Option<String>,
    /// Updated max members
    pub max_members: Option<i32>,
    /// Toggle peer data sharing
    pub peer_data_sharing: Option<bool>,
    /// Change when the AI coach replies in the bound channel chat
    pub respond_mode: Option<GroupRespondMode>,
    /// Toggle active status
    pub is_active: Option<bool>,
}

/// Request to join a group via invite code
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JoinGroupRequest {
    /// The invite code
    pub invite_code: String,
}

// ============================================================================
// Response / Summary Types
// ============================================================================

/// Lightweight group summary for list views
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    /// Total weekly active duration in seconds across all activities.
    /// Surfaces training volume for HR/duration-only sources (WHOOP,
    /// indoor trainers, treadmill sessions) that report no GPS distance
    /// — without this the group card renders such weeks as `0.0 km`
    /// and the LLM has no visibility into the athlete's actual workload.
    pub weekly_duration_seconds: i64,
    /// Primary sport type
    pub primary_sport: Option<String>,
    /// VDOT (running fitness estimator)
    pub vdot: Option<f64>,
    /// Risk level for overtraining
    pub overtraining_risk: OvertrainingRiskLevel,
    /// Days since last activity
    pub days_since_last_activity: Option<i32>,
    /// Most recent activity date observed per connected provider.
    /// Lets the LLM see "Strava last seen 33 days ago, WHOOP today"
    /// and stop attributing a quiet provider to "not synced" excuses.
    /// Keyed by provider slug (`strava`, `whoop`, `sciotte`, ...).
    pub last_activity_per_provider: HashMap<String, DateTime<Utc>>,
    /// Compact list of recent activities (last 7 days, newest first).
    /// Lets the LLM answer sub-week questions ("Saturday vs Sunday",
    /// "longest ride this week") that aggregate fields alone cannot
    /// support. Empty for members without `peer_sharing_consent` or
    /// without activity data.
    pub recent_activities: Vec<RosterActivity>,
    /// Provider slugs whose connection flipped to `needs_reauth`/`revoked` for this member
    /// (an OAuth token refresh died non-recoverably). Lets the group coach name the dead
    /// provider ("Phil's WHOOP needs reconnecting") instead of treating it as merely quiet.
    /// Empty when all of the member's connections are healthy.
    pub needs_reauth_providers: Vec<String>,
    /// True when this member's activities came from a cache that was stale and
    /// could not be freshened within the turn's refresh budget. The group
    /// context renderer turns this into a directive: call
    /// `get_group_member_activities` for fresh data before answering about
    /// this member, and never read activity recency in a stale snapshot as
    /// connection health. Distinct from both "quiet" (fresh snapshot, no
    /// recent training) and "broken" ([`Self::needs_reauth_providers`]).
    #[serde(default)]
    pub served_stale: bool,
    /// When this snapshot was computed
    pub computed_at: DateTime<Utc>,
}

/// Compact per-activity record shared across consenting group members.
///
/// Surfaced in the group context so the LLM can answer sub-week
/// questions ("what did Philippe do this weekend?") that the weekly
/// aggregates alone cannot support. Field set is deliberately minimal —
/// no GPS, no streams, no per-second sensor data — to limit both token
/// cost and the surface area of data shared between peers. Members
/// who haven't opted in via `peer_sharing_consent` never appear here.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RosterActivity {
    /// Workout start time. Date-only sources (e.g. Strava-mirror scrapes)
    /// render at midnight UTC of the workout day.
    pub start: DateTime<Utc>,
    /// Sport label as rendered for the LLM (e.g. `TrailRunning`, `Ride`,
    /// `WHOOP run`). Mirrors the serialization of `Activity::sport_type`.
    pub sport: String,
    /// Optional kilometers when the source carries GPS distance.
    /// `None` for HR/duration-only sources like WHOOP.
    pub distance_km: Option<f64>,
    /// Activity duration in minutes — always available.
    pub duration_minutes: i64,
    /// User-facing name when the provider supplies one
    /// (e.g. `"🤭"`, `"Peanut butter 🤤"`). Empty when missing.
    pub name: String,
    /// Optional city name when the provider supplies it (Strava, Garmin).
    /// `None` for HR-only / treadmill / non-GPS sources. Surfaced so the LLM
    /// can answer "endroit entre vous deux" questions without fabricating.
    pub city: Option<String>,
    /// Optional starting latitude when the source carries GPS. Paired with
    /// `start_longitude`. `None` for HR-only sources. Lets the LLM do
    /// midpoint / distance math when both members have coordinates.
    pub start_latitude: Option<f64>,
    /// Optional starting longitude. See [`Self::start_latitude`].
    pub start_longitude: Option<f64>,
    /// Optional total ascent in meters when the source carries it (Strava,
    /// sciotte, Coros all parse it). `None` for sources without elevation
    /// (HR-only, treadmill). Surfaced so the LLM can answer "combien de
    /// dénivelé pour vous deux" without claiming it lacks a shared total.
    pub elevation_gain_m: Option<f64>,
}

/// Overtraining risk level for a member
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
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
#[serde(rename_all = "snake_case")]
pub enum MemberFlag {
    /// TSB below threshold, high fatigue
    Overreaching,
    /// TSB positive, ready for hard training
    FreshForm,
    /// Set a personal record recently
    PersonalRecord,
    /// Form far below the athlete's chronic fitness (deepest fatigue band)
    DeepFatigue,
    /// No activity for extended period
    Inactive,
    /// Weekly volume dropped significantly from baseline
    VolumeDrop,
}

/// Formatted group summary block for LLM injection
#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// Health flag for a group member needing attention
#[derive(Debug, Clone, Serialize, Deserialize)]
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
