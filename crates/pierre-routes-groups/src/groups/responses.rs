// ABOUTME: Response bodies the group coaching routes serialize, the coach's invite list with each invite's funnel included
// ABOUTME: Re-exported from the groups module, so callers keep their groups:: paths
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::fmt;

use serde::{Deserialize, Serialize};

use pierre_core::models::groups::{
    GroupAggregateStats, GroupHealthFlag, GroupInvite, GroupMember, GroupWeeklyReport,
};
use pierre_services::group_invite_card::InviteFunnel;

use crate::group_response::GroupResponse;

// ============================================================================
// Response Types
// ============================================================================

/// Response for a group member
#[derive(Debug, Serialize, Deserialize)]
pub struct MemberResponse {
    /// Membership record ID
    pub id: String,
    /// Group ID
    pub group_id: String,
    /// User ID
    pub user_id: String,
    /// Role within the group
    pub role: String,
    /// Whether peer sharing consent is given
    pub peer_sharing_consent: bool,
    /// When consent was given
    pub consent_given_at: String,
    /// When the member joined
    pub joined_at: String,
    /// Display name (if available)
    pub display_name: Option<String>,
}

impl From<GroupMember> for MemberResponse {
    fn from(m: GroupMember) -> Self {
        Self {
            id: m.id.to_string(),
            group_id: m.group_id.to_string(),
            user_id: m.user_id.to_string(),
            role: m.role.as_str().to_owned(),
            peer_sharing_consent: m.peer_sharing_consent,
            consent_given_at: m.consent_given_at.to_rfc3339(),
            joined_at: m.joined_at.to_rfc3339(),
            display_name: m.display_name,
        }
    }
}

/// Response for listing members
#[derive(Debug, Serialize, Deserialize)]
pub struct ListMembersResponse {
    /// Active members
    pub members: Vec<MemberResponse>,
    /// Total count
    pub total: usize,
    /// Response metadata
    pub metadata: GroupMetadata,
}

/// Response for a group invite
#[derive(Serialize, Deserialize)]
pub struct InviteResponse {
    /// Invite ID
    pub id: String,
    /// Group ID
    pub group_id: String,
    /// The invite code
    pub code: String,
    /// What redeeming this invite grants ("member" or "coach")
    pub kind: String,
    /// User who created the invite
    pub created_by: String,
    /// When the invite expires (if ever)
    pub expires_at: Option<String>,
    /// Maximum uses (if limited)
    pub max_uses: Option<i32>,
    /// Current use count
    pub use_count: i32,
    /// Whether the invite is active
    pub is_active: bool,
    /// Whether redeeming it also approves a pending account
    pub approves_signup: bool,
    /// When the invite was created
    pub created_at: String,
}

impl fmt::Debug for InviteResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InviteResponse")
            .field("id", &self.id)
            .field("group_id", &self.group_id)
            .field("code", &"[REDACTED]")
            .field("kind", &self.kind)
            .field("created_by", &self.created_by)
            .field("expires_at", &self.expires_at)
            .field("max_uses", &self.max_uses)
            .field("use_count", &self.use_count)
            .field("is_active", &self.is_active)
            .field("approves_signup", &self.approves_signup)
            .field("created_at", &self.created_at)
            .finish()
    }
}

impl From<GroupInvite> for InviteResponse {
    fn from(inv: GroupInvite) -> Self {
        Self {
            id: inv.id.to_string(),
            group_id: inv.group_id.to_string(),
            code: inv.code,
            kind: inv.kind.as_str().to_owned(),
            created_by: inv.created_by.to_string(),
            expires_at: inv.expires_at.map(|dt| dt.to_rfc3339()),
            max_uses: inv.max_uses,
            use_count: inv.use_count,
            is_active: inv.is_active,
            approves_signup: inv.approves_signup,
            created_at: inv.created_at.to_rfc3339(),
        }
    }
}

/// One invite of a group as its coach's list reads it: the invite and how
/// far its link got (carnet#1650).
#[derive(Debug, Serialize)]
pub struct InviteWithFunnelResponse {
    /// The invite itself
    #[serde(flatten)]
    pub invite: InviteResponse,
    /// Opened, held and refused-full counts for this invite
    pub funnel: InviteFunnel,
}

/// Response for listing invites
#[derive(Debug, Serialize)]
pub struct ListInvitesResponse {
    /// Invites for this group
    pub invites: Vec<InviteWithFunnelResponse>,
    /// Total count
    pub total: usize,
    /// Response metadata
    pub metadata: GroupMetadata,
}

/// Response for a group owner becoming its coach (carnet#1650)
#[derive(Debug, Serialize)]
pub struct ClaimGroupCoachResponse {
    /// The group, with its coach attached
    pub group: GroupResponse,
}

/// Metadata for group API responses
#[derive(Debug, Serialize, Deserialize)]
pub struct GroupMetadata {
    /// Response timestamp
    pub timestamp: String,
    /// API version
    pub api_version: String,
}

/// Which members a group analytics response was computed over.
///
/// Every number, name and flag in `/stats`, `/report` and `/health` comes
/// from the members whose training the caller may see — those who share it
/// with them, plus the caller — so a client can say when that is fewer than
/// the group holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnalyticsScope {
    /// Members the response covers.
    pub visible_members: usize,
    /// Members the group holds.
    pub roster_members: usize,
}

/// Response wrapper for group aggregate stats
#[derive(Debug, Serialize, Deserialize)]
pub struct StatsResponse {
    /// Aggregate statistics
    pub stats: GroupAggregateStats,
    /// The members the stats cover
    pub scope: AnalyticsScope,
    /// Response metadata
    pub metadata: GroupMetadata,
}

/// Response wrapper for weekly report
#[derive(Debug, Serialize, Deserialize)]
pub struct WeeklyReportResponse {
    /// Weekly report data
    pub report: GroupWeeklyReport,
    /// The members the report covers
    pub scope: AnalyticsScope,
    /// Response metadata
    pub metadata: GroupMetadata,
}

/// Response wrapper for health flags
#[derive(Debug, Serialize, Deserialize)]
pub struct HealthFlagsResponse {
    /// Health flags for group members
    pub flags: Vec<GroupHealthFlag>,
    /// Total count
    pub total: usize,
    /// The members the flags were computed over
    pub scope: AnalyticsScope,
    /// Response metadata
    pub metadata: GroupMetadata,
}

/// Response for group creation permissions check
#[derive(Debug, Serialize)]
pub struct GroupPermissionsResponse {
    /// Whether the current user can create groups
    pub can_create: bool,
    /// Current group creation policy for the tenant
    pub policy: String,
    /// Whether the tenant's plan tier enables the weekly digest — the same
    /// [`GroupFeatureFlags::weekly_digest`](pierre_groups::strategies::tier::GroupFeatureFlags::weekly_digest)
    /// read that decides which tenants the digest scheduler sweeps. The group
    /// detail surfaces render the weekly report and health flags off this
    /// flag instead of deriving a tier of their own.
    pub weekly_digest: bool,
}
