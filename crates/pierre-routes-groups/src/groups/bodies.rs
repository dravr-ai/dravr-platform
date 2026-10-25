// ABOUTME: Request bodies and query parameters the group coaching routes deserialize
// ABOUTME: Re-exported from the groups module, so callers keep their groups:: paths
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use pierre_core::models::groups::{GroupInviteKind, GroupRole};

#[cfg(doc)]
use pierre_groups::creation_policy::may_coach_group;

// ============================================================================
// Request Types
// ============================================================================

/// Request to create a group invite
#[derive(Debug, Deserialize)]
pub struct CreateInviteBody {
    /// Number of days until the invite expires (None = never)
    pub expires_in_days: Option<i64>,
    /// Maximum number of uses (None = unlimited)
    pub max_uses: Option<i32>,
    /// What the invite grants: athlete membership (default) or agent
    /// attachment. Omitted → `member`.
    #[serde(default)]
    pub kind: GroupInviteKind,
    /// Whether the invite also approves the pending account that redeems
    /// it: a coach's signup code (carnet#1128). Only a coach with roster
    /// management may mint one, and it needs both caps.
    #[serde(default)]
    pub approves_signup: bool,
}

/// Request to create a coaching group — the onboarding group step's call
#[derive(Debug, Deserialize)]
pub struct CreateGroupBody {
    /// Group name
    pub name: String,
    /// Optional description
    #[serde(default)]
    pub description: Option<String>,
    /// The AI agent the group answers with, picked while creating the group.
    /// Required, with no fallback to the caller's own agent: that is no
    /// evidence of what the group's athletes need, and a coach who does not
    /// train has none. Optional only at deserialization, so a missing one is
    /// refused with the API's JSON error rather than the extractor's plain text.
    #[serde(default)]
    pub agent_id: Option<String>,
    /// Make the caller the group's human coach. Honoured only when they may
    /// coach it ([`may_coach_group`]); otherwise the group is still created
    /// and answers with no `coach_user_id`, which the client reads as coach
    /// access pending.
    #[serde(default)]
    pub coach_is_me: bool,
}

/// Request to update a member's role
#[derive(Debug, Deserialize)]
pub struct UpdateRoleBody {
    /// New role for the member
    pub role: GroupRole,
}

/// Request to update peer sharing consent
#[derive(Debug, Deserialize)]
pub struct UpdatePeerConsentBody {
    /// Whether the user consents to peer data sharing
    pub consent: bool,
}

/// Request body for the caller's own coach sharing, and its answer: whether
/// the group's human coach reads their training, plan and room messages.
#[derive(Debug, Serialize, Deserialize)]
pub struct CoachSharingBody {
    /// Whether the caller shares with the group's human coach
    pub consent: bool,
}

// ============================================================================
// Query Types
// ============================================================================

/// Optional query parameters for stats/report endpoints
#[derive(Debug, Deserialize, Default)]
pub struct PeriodQuery {
    /// Period start (ISO 8601). Defaults to 7 days ago.
    pub from: Option<DateTime<Utc>>,
    /// Period end (ISO 8601). Defaults to now.
    pub to: Option<DateTime<Utc>>,
}
