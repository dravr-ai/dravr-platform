// ABOUTME: Group coaching REST routes for group CRUD, membership, invites, and analytics
// ABOUTME: 18 endpoints wrapping GroupService for multi-person AI coaching management
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Group coaching routes
//!
//! This module handles REST endpoints for coaching group management:
//! group CRUD, membership, invites, and analytics. All endpoints require
//! JWT authentication. Admin-only operations check the caller's `GroupRole`.

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{delete, get, post, put},
    Json, Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
#[cfg(feature = "openapi")]
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    errors::{AppError, ErrorCode},
    mcp::resources::ServerResources,
    middleware::extract_auth_from_headers,
};
use pierre_auth::auth::AuthResult;
use pierre_core::models::groups::{
    CoachingGroup, CreateGroupRequest, GroupAggregateStats, GroupHealthFlag, GroupInvite,
    GroupMember, GroupRole, GroupSummary, GroupTrend, GroupWeeklyReport, JoinGroupRequest,
    UpdateGroupRequest,
};
use pierre_core::models::TenantId;

// ============================================================================
// Response Types
// ============================================================================

/// Response for a single coaching group
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct GroupResponse {
    /// Group ID
    pub id: String,
    /// Tenant ID for isolation
    pub tenant_id: String,
    /// Group name
    pub name: String,
    /// Optional description
    pub description: Option<String>,
    /// Coach persona ID
    pub coach_id: String,
    /// Owner user ID
    pub owner_id: String,
    /// Whether peer data sharing is enabled
    pub peer_data_sharing: bool,
    /// Maximum members allowed
    pub max_members: i32,
    /// Whether the group is active
    pub is_active: bool,
    /// When the group was created
    pub created_at: String,
    /// When the group was last updated
    pub updated_at: String,
}

impl From<CoachingGroup> for GroupResponse {
    fn from(g: CoachingGroup) -> Self {
        Self {
            id: g.id.to_string(),
            tenant_id: g.tenant_id,
            name: g.name,
            description: g.description,
            coach_id: g.coach_id,
            owner_id: g.owner_id.to_string(),
            peer_data_sharing: g.peer_data_sharing,
            max_members: g.max_members,
            is_active: g.is_active,
            created_at: g.created_at.to_rfc3339(),
            updated_at: g.updated_at.to_rfc3339(),
        }
    }
}

/// Response for listing groups (uses the model's lightweight summary)
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ListGroupsResponse {
    /// Groups the user belongs to
    pub groups: Vec<GroupSummary>,
    /// Total count
    pub total: usize,
    /// Response metadata
    pub metadata: GroupMetadata,
}

/// Response for a group member
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
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
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ListMembersResponse {
    /// Active members
    pub members: Vec<MemberResponse>,
    /// Total count
    pub total: usize,
    /// Response metadata
    pub metadata: GroupMetadata,
}

/// Response for a group invite
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct InviteResponse {
    /// Invite ID
    pub id: String,
    /// Group ID
    pub group_id: String,
    /// The invite code
    pub code: String,
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
    /// When the invite was created
    pub created_at: String,
}

impl From<GroupInvite> for InviteResponse {
    fn from(inv: GroupInvite) -> Self {
        Self {
            id: inv.id.to_string(),
            group_id: inv.group_id.to_string(),
            code: inv.code,
            created_by: inv.created_by.to_string(),
            expires_at: inv.expires_at.map(|dt| dt.to_rfc3339()),
            max_uses: inv.max_uses,
            use_count: inv.use_count,
            is_active: inv.is_active,
            created_at: inv.created_at.to_rfc3339(),
        }
    }
}

/// Response for listing invites
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ListInvitesResponse {
    /// Invites for this group
    pub invites: Vec<InviteResponse>,
    /// Total count
    pub total: usize,
    /// Response metadata
    pub metadata: GroupMetadata,
}

/// Metadata for group API responses
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct GroupMetadata {
    /// Response timestamp
    pub timestamp: String,
    /// API version
    pub api_version: String,
}

/// Response wrapper for group aggregate stats
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct StatsResponse {
    /// Aggregate statistics
    pub stats: GroupAggregateStats,
    /// Response metadata
    pub metadata: GroupMetadata,
}

/// Response wrapper for weekly report
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct WeeklyReportResponse {
    /// Weekly report data
    pub report: GroupWeeklyReport,
    /// Response metadata
    pub metadata: GroupMetadata,
}

/// Response wrapper for health flags
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct HealthFlagsResponse {
    /// Health flags for group members
    pub flags: Vec<GroupHealthFlag>,
    /// Total count
    pub total: usize,
    /// Response metadata
    pub metadata: GroupMetadata,
}

// ============================================================================
// Request Types
// ============================================================================

/// Request to create a group invite
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct CreateInviteBody {
    /// Number of days until the invite expires (None = never)
    pub expires_in_days: Option<i64>,
    /// Maximum number of uses (None = unlimited)
    pub max_uses: Option<i32>,
}

/// Request to update a member's role
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct UpdateRoleBody {
    /// New role for the member
    pub role: GroupRole,
}

/// Request to update peer sharing consent
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct UpdatePeerConsentBody {
    /// Whether the user consents to peer data sharing
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

// ============================================================================
// Routes
// ============================================================================

/// Group coaching route handler
pub struct GroupRoutes;

impl GroupRoutes {
    /// Create all group coaching routes
    pub fn routes(resources: Arc<ServerResources>) -> Router {
        Router::new()
            // Group CRUD
            .route("/api/groups", post(Self::handle_create_group))
            .route("/api/groups", get(Self::handle_list_my_groups))
            .route("/api/groups/{group_id}", get(Self::handle_get_group))
            .route("/api/groups/{group_id}", put(Self::handle_update_group))
            .route("/api/groups/{group_id}", delete(Self::handle_delete_group))
            // Membership
            .route(
                "/api/groups/{group_id}/members",
                get(Self::handle_list_members),
            )
            .route(
                "/api/groups/{group_id}/members/{user_id}",
                delete(Self::handle_remove_member),
            )
            .route(
                "/api/groups/{group_id}/members/{user_id}/role",
                put(Self::handle_update_role),
            )
            .route(
                "/api/groups/{group_id}/members/me/consent",
                put(Self::handle_update_peer_consent),
            )
            // Invites
            .route(
                "/api/groups/{group_id}/invites",
                post(Self::handle_create_invite),
            )
            .route(
                "/api/groups/{group_id}/invites",
                get(Self::handle_list_invites),
            )
            .route(
                "/api/groups/{group_id}/invites/{invite_id}",
                delete(Self::handle_deactivate_invite),
            )
            // Join / Leave
            .route("/api/groups/join", post(Self::handle_join_by_invite_code))
            .route(
                "/api/groups/{group_id}/leave",
                post(Self::handle_leave_group),
            )
            // Analytics
            .route("/api/groups/{group_id}/stats", get(Self::handle_get_stats))
            .route(
                "/api/groups/{group_id}/report",
                get(Self::handle_get_weekly_report),
            )
            .route(
                "/api/groups/{group_id}/health",
                get(Self::handle_get_health_flags),
            )
            .with_state(resources)
    }

    // ========================================================================
    // Helpers
    // ========================================================================

    /// Extract and authenticate user from authorization header or cookie
    async fn authenticate(
        headers: &HeaderMap,
        resources: &Arc<ServerResources>,
    ) -> Result<AuthResult, AppError> {
        extract_auth_from_headers(headers, resources).await
    }

    /// Extract tenant ID from auth claims
    fn get_tenant_id(auth: &AuthResult) -> Result<TenantId, AppError> {
        auth.active_tenant_id
            .map(TenantId::from)
            .ok_or_else(|| AppError::auth_invalid("No active tenant in session"))
    }

    /// Build metadata for responses
    fn build_metadata() -> GroupMetadata {
        GroupMetadata {
            timestamp: Utc::now().to_rfc3339(),
            api_version: "1.0".to_owned(),
        }
    }

    /// Check if the user has permission to create groups.
    ///
    /// Tenant admins/owners always have permission. Regular users are checked
    /// against the tenant's `group_creation_policy` config.
    ///
    /// # Errors
    ///
    /// Returns `PermissionDenied` if the user lacks group creation permission.
    async fn check_create_group_permission(
        resources: &Arc<ServerResources>,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> Result<(), AppError> {
        // Check tenant role — admins/owners always allowed
        let user_role = resources
            .repos
            .tenants
            .get_user_role(user_id, tenant_id)
            .await
            .unwrap_or(None);

        let is_tenant_admin = user_role
            .as_deref()
            .is_some_and(|r| r == "owner" || r == "admin");

        if is_tenant_admin {
            return Ok(());
        }

        // For regular users, check group_creation_policy via admin config
        // Default policy: admins_only (most restrictive)
        let policy = if let Some(ref config_service) = resources.admin_config {
            config_service
                .get_value("group_creation_policy", Some(&tenant_id.to_string()))
                .await
                .ok()
                .flatten()
                .and_then(|v| v.as_str().map(ToOwned::to_owned))
                .unwrap_or_else(|| "admins_only".to_owned())
        } else {
            "admins_only".to_owned()
        };

        match policy.as_str() {
            "everyone" => Ok(()),
            "admins_only" => Err(AppError::new(
                ErrorCode::PermissionDenied,
                "Group creation requires admin privileges. Contact your tenant administrator.",
            )),
            _ => Err(AppError::new(
                ErrorCode::PermissionDenied,
                "Group creation is not enabled for your account.",
            )),
        }
    }

    /// Verify the caller is an admin or owner of the given group.
    ///
    /// Returns the caller's membership record on success.
    async fn require_admin(
        resources: &Arc<ServerResources>,
        group_id: &str,
        user_id: Uuid,
    ) -> Result<GroupMember, AppError> {
        let member = resources
            .repos
            .groups
            .get_member(group_id, user_id)
            .await?
            .ok_or_else(|| AppError::not_found("Membership not found"))?;

        if !member.role.can_manage_members() {
            return Err(AppError::new(
                ErrorCode::PermissionDenied,
                "Admin or owner role required for this operation",
            ));
        }

        Ok(member)
    }

    /// Verify the caller is the owner of the given group.
    async fn require_owner(
        resources: &Arc<ServerResources>,
        group_id: &str,
        user_id: Uuid,
    ) -> Result<GroupMember, AppError> {
        let member = resources
            .repos
            .groups
            .get_member(group_id, user_id)
            .await?
            .ok_or_else(|| AppError::not_found("Membership not found"))?;

        if !member.role.can_delete_group() {
            return Err(AppError::new(
                ErrorCode::PermissionDenied,
                "Owner role required for this operation",
            ));
        }

        Ok(member)
    }

    /// Verify the caller is a member of the given group.
    async fn require_member(
        resources: &Arc<ServerResources>,
        group_id: &str,
        user_id: Uuid,
    ) -> Result<GroupMember, AppError> {
        resources
            .repos
            .groups
            .get_member(group_id, user_id)
            .await?
            .ok_or_else(|| AppError::not_found("You are not a member of this group"))
    }

    // ========================================================================
    // Group CRUD handlers
    // ========================================================================

    /// POST /api/groups — Create a new coaching group
    async fn handle_create_group(
        State(resources): State<Arc<ServerResources>>,
        headers: HeaderMap,
        Json(body): Json<CreateGroupRequest>,
    ) -> Result<Response, AppError> {
        let auth = Self::authenticate(&headers, &resources).await?;
        let tenant_id = Self::get_tenant_id(&auth)?;

        // Check group creation permission:
        // 1. Tenant admins/owners always allowed
        // 2. Regular users need group_creation_policy = "everyone"
        Self::check_create_group_permission(&resources, auth.user_id, tenant_id).await?;

        if body.name.trim().is_empty() {
            return Err(AppError::invalid_input("Group name must not be empty"));
        }

        let group = CoachingGroup {
            id: Uuid::new_v4(),
            tenant_id: tenant_id.to_string(),
            name: body.name.clone(),
            description: body.description.clone(),
            coach_id: body.coach_id.clone(),
            owner_id: auth.user_id,
            peer_data_sharing: false,
            max_members: body.max_members.unwrap_or(20).clamp(2, 100),
            is_active: true,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let created = resources
            .repos
            .groups
            .create_group(tenant_id, &group)
            .await?;

        // Auto-add owner as member with Owner role
        let owner_member = GroupMember {
            id: Uuid::new_v4(),
            group_id: created.id,
            user_id: auth.user_id,
            tenant_id: tenant_id.to_string(),
            role: GroupRole::Owner,
            peer_sharing_consent: false,
            consent_given_at: Utc::now(),
            joined_at: Utc::now(),
            left_at: None,
            display_name: None,
        };
        resources.repos.groups.add_member(&owner_member).await?;

        let response: GroupResponse = created.into();
        Ok((StatusCode::CREATED, Json(response)).into_response())
    }

    /// GET /api/groups — List groups the current user belongs to
    async fn handle_list_my_groups(
        State(resources): State<Arc<ServerResources>>,
        headers: HeaderMap,
    ) -> Result<Response, AppError> {
        let auth = Self::authenticate(&headers, &resources).await?;

        let groups = resources
            .repos
            .groups
            .list_groups_for_user(auth.user_id)
            .await?;

        let response = ListGroupsResponse {
            total: groups.len(),
            groups,
            metadata: Self::build_metadata(),
        };

        Ok((StatusCode::OK, Json(response)).into_response())
    }

    /// GET `/api/groups/:group_id` — Get a single group by ID
    async fn handle_get_group(
        State(resources): State<Arc<ServerResources>>,
        headers: HeaderMap,
        Path(group_id): Path<String>,
    ) -> Result<Response, AppError> {
        let auth = Self::authenticate(&headers, &resources).await?;
        let tenant_id = Self::get_tenant_id(&auth)?;

        // Verify caller is a member
        Self::require_member(&resources, &group_id, auth.user_id).await?;

        let group = resources
            .repos
            .groups
            .get_group(&group_id, tenant_id)
            .await?
            .ok_or_else(|| AppError::not_found(format!("Group {group_id}")))?;

        let response: GroupResponse = group.into();
        Ok((StatusCode::OK, Json(response)).into_response())
    }

    /// PUT `/api/groups/:group_id` — Update group settings (admin/owner only)
    async fn handle_update_group(
        State(resources): State<Arc<ServerResources>>,
        headers: HeaderMap,
        Path(group_id): Path<String>,
        Json(body): Json<UpdateGroupRequest>,
    ) -> Result<Response, AppError> {
        let auth = Self::authenticate(&headers, &resources).await?;
        let tenant_id = Self::get_tenant_id(&auth)?;

        // Verify admin/owner role
        Self::require_admin(&resources, &group_id, auth.user_id).await?;

        let updated = resources
            .repos
            .groups
            .update_group(&group_id, tenant_id, &body)
            .await?
            .ok_or_else(|| AppError::not_found(format!("Group {group_id}")))?;

        let response: GroupResponse = updated.into();
        Ok((StatusCode::OK, Json(response)).into_response())
    }

    /// DELETE `/api/groups/:group_id` — Soft-delete a group (owner only)
    async fn handle_delete_group(
        State(resources): State<Arc<ServerResources>>,
        headers: HeaderMap,
        Path(group_id): Path<String>,
    ) -> Result<Response, AppError> {
        let auth = Self::authenticate(&headers, &resources).await?;
        let tenant_id = Self::get_tenant_id(&auth)?;

        // Only owner can delete
        Self::require_owner(&resources, &group_id, auth.user_id).await?;

        let deleted = resources
            .repos
            .groups
            .delete_group(&group_id, tenant_id)
            .await?;

        if !deleted {
            return Err(AppError::not_found(format!("Group {group_id}")));
        }

        Ok((StatusCode::NO_CONTENT, ()).into_response())
    }

    // ========================================================================
    // Membership handlers
    // ========================================================================

    /// GET `/api/groups/:group_id/members` — List group members
    async fn handle_list_members(
        State(resources): State<Arc<ServerResources>>,
        headers: HeaderMap,
        Path(group_id): Path<String>,
    ) -> Result<Response, AppError> {
        let auth = Self::authenticate(&headers, &resources).await?;
        let tenant_id = Self::get_tenant_id(&auth)?;

        // Verify caller is a member
        Self::require_member(&resources, &group_id, auth.user_id).await?;

        let members = resources
            .repos
            .groups
            .list_members(&group_id, tenant_id)
            .await?;

        let member_responses: Vec<MemberResponse> =
            members.into_iter().map(MemberResponse::from).collect();

        let response = ListMembersResponse {
            total: member_responses.len(),
            members: member_responses,
            metadata: Self::build_metadata(),
        };

        Ok((StatusCode::OK, Json(response)).into_response())
    }

    /// DELETE `/api/groups/:group_id/members/:user_id` — Remove a member (admin/owner only)
    async fn handle_remove_member(
        State(resources): State<Arc<ServerResources>>,
        headers: HeaderMap,
        Path((group_id, target_user_id)): Path<(String, String)>,
    ) -> Result<Response, AppError> {
        let auth = Self::authenticate(&headers, &resources).await?;
        let tenant_id = Self::get_tenant_id(&auth)?;

        // Verify admin/owner role
        Self::require_admin(&resources, &group_id, auth.user_id).await?;

        let target_uuid = Uuid::parse_str(&target_user_id)
            .map_err(|_| AppError::invalid_input("Invalid user_id format"))?;

        // Cannot remove the owner
        let target_member = resources
            .repos
            .groups
            .get_member(&group_id, target_uuid)
            .await?
            .ok_or_else(|| AppError::not_found("Target member not found"))?;

        if target_member.role == GroupRole::Owner {
            return Err(AppError::new(
                ErrorCode::PermissionDenied,
                "Cannot remove the group owner",
            ));
        }

        let removed = resources
            .repos
            .groups
            .remove_member(&group_id, target_uuid, tenant_id)
            .await?;

        if !removed {
            return Err(AppError::not_found("Member not found"));
        }

        Ok((StatusCode::NO_CONTENT, ()).into_response())
    }

    /// PUT `/api/groups/:group_id/members/:user_id/role` — Update member role (admin/owner only)
    async fn handle_update_role(
        State(resources): State<Arc<ServerResources>>,
        headers: HeaderMap,
        Path((group_id, target_user_id)): Path<(String, String)>,
        Json(body): Json<UpdateRoleBody>,
    ) -> Result<Response, AppError> {
        let auth = Self::authenticate(&headers, &resources).await?;
        let tenant_id = Self::get_tenant_id(&auth)?;

        let caller_member = Self::require_admin(&resources, &group_id, auth.user_id).await?;

        let target_uuid = Uuid::parse_str(&target_user_id)
            .map_err(|_| AppError::invalid_input("Invalid user_id format"))?;

        // Only owner can promote to admin or transfer ownership
        if (body.role == GroupRole::Owner || body.role == GroupRole::Admin)
            && caller_member.role != GroupRole::Owner
        {
            return Err(AppError::new(
                ErrorCode::PermissionDenied,
                "Only the owner can promote members to admin or owner",
            ));
        }

        // Cannot change own role (use transfer ownership instead)
        if target_uuid == auth.user_id {
            return Err(AppError::invalid_input(
                "Cannot change your own role. Use ownership transfer instead.",
            ));
        }

        let updated = resources
            .repos
            .groups
            .update_member_role(&group_id, target_uuid, tenant_id, body.role)
            .await?;

        if !updated {
            return Err(AppError::not_found("Member not found"));
        }

        // Fetch updated member to return
        let member = resources
            .repos
            .groups
            .get_member(&group_id, target_uuid)
            .await?
            .ok_or_else(|| AppError::internal("Failed to fetch updated member"))?;

        let response: MemberResponse = member.into();
        Ok((StatusCode::OK, Json(response)).into_response())
    }

    /// PUT `/api/groups/:group_id/members/me/consent` — Update own peer sharing consent
    async fn handle_update_peer_consent(
        State(resources): State<Arc<ServerResources>>,
        headers: HeaderMap,
        Path(group_id): Path<String>,
        Json(body): Json<UpdatePeerConsentBody>,
    ) -> Result<Response, AppError> {
        let auth = Self::authenticate(&headers, &resources).await?;
        let tenant_id = Self::get_tenant_id(&auth)?;

        // Verify caller is a member
        Self::require_member(&resources, &group_id, auth.user_id).await?;

        let updated = resources
            .repos
            .groups
            .update_peer_sharing_consent(&group_id, auth.user_id, tenant_id, body.consent)
            .await?;

        if !updated {
            return Err(AppError::not_found("Membership not found"));
        }

        // Fetch updated member
        let member = resources
            .repos
            .groups
            .get_member(&group_id, auth.user_id)
            .await?
            .ok_or_else(|| AppError::internal("Failed to fetch updated member"))?;

        let response: MemberResponse = member.into();
        Ok((StatusCode::OK, Json(response)).into_response())
    }

    // ========================================================================
    // Invite handlers
    // ========================================================================

    /// POST `/api/groups/:group_id/invites` — Create an invite code (admin/owner only)
    async fn handle_create_invite(
        State(resources): State<Arc<ServerResources>>,
        headers: HeaderMap,
        Path(group_id): Path<String>,
        Json(body): Json<CreateInviteBody>,
    ) -> Result<Response, AppError> {
        let auth = Self::authenticate(&headers, &resources).await?;
        let tenant_id = Self::get_tenant_id(&auth)?;

        // Verify admin/owner role
        Self::require_admin(&resources, &group_id, auth.user_id).await?;

        let group_uuid = Uuid::parse_str(&group_id)
            .map_err(|_| AppError::invalid_input("Invalid group_id format"))?;

        // Validate expiry bounds
        if let Some(days) = body.expires_in_days {
            if !(1..=365).contains(&days) {
                return Err(AppError::invalid_input(
                    "expires_in_days must be between 1 and 365",
                ));
            }
        }

        // Validate max_uses bounds
        if let Some(max) = body.max_uses {
            if !(1..=1000).contains(&max) {
                return Err(AppError::invalid_input(
                    "max_uses must be between 1 and 1000",
                ));
            }
        }

        // Generate invite code
        let code = generate_invite_code();
        let expires_at = body
            .expires_in_days
            .map(|days| Utc::now() + chrono::Duration::days(days));

        let invite = GroupInvite {
            id: Uuid::new_v4(),
            group_id: group_uuid,
            tenant_id: tenant_id.to_string(),
            code,
            created_by: auth.user_id,
            expires_at,
            max_uses: body.max_uses,
            use_count: 0,
            is_active: true,
            created_at: Utc::now(),
        };

        let created = resources.repos.groups.create_invite(&invite).await?;

        let response: InviteResponse = created.into();
        Ok((StatusCode::CREATED, Json(response)).into_response())
    }

    /// GET `/api/groups/:group_id/invites` — List invites for a group (admin/owner only)
    async fn handle_list_invites(
        State(resources): State<Arc<ServerResources>>,
        headers: HeaderMap,
        Path(group_id): Path<String>,
    ) -> Result<Response, AppError> {
        let auth = Self::authenticate(&headers, &resources).await?;
        let tenant_id = Self::get_tenant_id(&auth)?;

        // Verify admin/owner role
        Self::require_admin(&resources, &group_id, auth.user_id).await?;

        let invites = resources
            .repos
            .groups
            .list_invites(&group_id, tenant_id)
            .await?;

        let invite_responses: Vec<InviteResponse> =
            invites.into_iter().map(InviteResponse::from).collect();

        let response = ListInvitesResponse {
            total: invite_responses.len(),
            invites: invite_responses,
            metadata: Self::build_metadata(),
        };

        Ok((StatusCode::OK, Json(response)).into_response())
    }

    /// DELETE `/api/groups/:group_id/invites/:invite_id` — Deactivate an invite (admin/owner only)
    async fn handle_deactivate_invite(
        State(resources): State<Arc<ServerResources>>,
        headers: HeaderMap,
        Path((group_id, invite_id)): Path<(String, String)>,
    ) -> Result<Response, AppError> {
        let auth = Self::authenticate(&headers, &resources).await?;
        let tenant_id = Self::get_tenant_id(&auth)?;

        // Verify admin/owner role
        Self::require_admin(&resources, &group_id, auth.user_id).await?;

        let deactivated = resources
            .repos
            .groups
            .deactivate_invite(&invite_id, tenant_id)
            .await?;

        if !deactivated {
            return Err(AppError::not_found(format!("Invite {invite_id}")));
        }

        Ok((StatusCode::NO_CONTENT, ()).into_response())
    }

    // ========================================================================
    // Join / Leave handlers
    // ========================================================================

    /// POST /api/groups/join — Join a group using an invite code
    async fn handle_join_by_invite_code(
        State(resources): State<Arc<ServerResources>>,
        headers: HeaderMap,
        Json(body): Json<JoinGroupRequest>,
    ) -> Result<Response, AppError> {
        let auth = Self::authenticate(&headers, &resources).await?;
        // Caller's tenant is not used — the invite's tenant (group's tenant) is used instead

        if body.invite_code.trim().is_empty() {
            return Err(AppError::invalid_input("Invite code must not be empty"));
        }

        let invite = resources
            .repos
            .groups
            .get_invite_by_code(&body.invite_code)
            .await?
            .ok_or_else(|| AppError::not_found("Invalid or expired invite code"))?;

        // Validate invite
        if !invite.is_active {
            return Err(AppError::invalid_input("This invite has been deactivated"));
        }
        if let Some(expires) = invite.expires_at {
            if expires < Utc::now() {
                return Err(AppError::invalid_input("This invite has expired"));
            }
        }
        if let Some(max) = invite.max_uses {
            if invite.use_count >= max {
                return Err(AppError::invalid_input(
                    "This invite has reached its use limit",
                ));
            }
        }

        // Use the invite's tenant (= group's tenant) for cross-tenant join
        let group_tenant_id: TenantId = invite
            .tenant_id
            .parse()
            .map_err(|e| AppError::internal(format!("Invalid invite tenant: {e}")))?;

        // Check group capacity
        let group = resources
            .repos
            .groups
            .get_group(&invite.group_id.to_string(), group_tenant_id)
            .await?
            .ok_or_else(|| AppError::not_found("Group not found"))?;

        let current_count = resources
            .repos
            .groups
            .count_members(&invite.group_id.to_string(), group_tenant_id)
            .await?;

        if current_count >= i64::from(group.max_members) {
            return Err(AppError::invalid_input("This group is full"));
        }

        // Check not already a member
        let existing = resources
            .repos
            .groups
            .get_member(&invite.group_id.to_string(), auth.user_id)
            .await?;

        if existing.is_some() {
            return Err(AppError::invalid_input(
                "You are already a member of this group",
            ));
        }

        // Create membership under the group's tenant
        let member = GroupMember {
            id: Uuid::new_v4(),
            group_id: invite.group_id,
            user_id: auth.user_id,
            tenant_id: group_tenant_id.to_string(),
            role: GroupRole::Member,
            peer_sharing_consent: false,
            consent_given_at: Utc::now(),
            joined_at: Utc::now(),
            left_at: None,
            display_name: None,
        };

        let created = resources.repos.groups.add_member(&member).await?;

        // Increment invite usage
        resources
            .repos
            .groups
            .increment_invite_use_count(&invite.id.to_string())
            .await?;

        let response: MemberResponse = created.into();
        Ok((StatusCode::CREATED, Json(response)).into_response())
    }

    /// POST `/api/groups/:group_id/leave` — Leave a group
    async fn handle_leave_group(
        State(resources): State<Arc<ServerResources>>,
        headers: HeaderMap,
        Path(group_id): Path<String>,
    ) -> Result<Response, AppError> {
        let auth = Self::authenticate(&headers, &resources).await?;
        let tenant_id = Self::get_tenant_id(&auth)?;

        let member = Self::require_member(&resources, &group_id, auth.user_id).await?;

        // Owner cannot leave — must transfer ownership or delete the group
        if member.role == GroupRole::Owner {
            return Err(AppError::new(
                ErrorCode::PermissionDenied,
                "Owner cannot leave the group. Transfer ownership or delete the group instead.",
            ));
        }

        let left = resources
            .repos
            .groups
            .remove_member(&group_id, auth.user_id, tenant_id)
            .await?;

        if !left {
            return Err(AppError::internal("Failed to remove membership"));
        }

        Ok((StatusCode::NO_CONTENT, ()).into_response())
    }

    // ========================================================================
    // Analytics handlers (placeholder — full integration requires fitness snapshots)
    // ========================================================================

    /// GET `/api/groups/:group_id/stats` — Get aggregate stats for a group
    async fn handle_get_stats(
        State(resources): State<Arc<ServerResources>>,
        headers: HeaderMap,
        Path(group_id): Path<String>,
        Query(_period): Query<PeriodQuery>,
    ) -> Result<Response, AppError> {
        let auth = Self::authenticate(&headers, &resources).await?;
        let tenant_id = Self::get_tenant_id(&auth)?;

        // Verify caller is a member
        Self::require_member(&resources, &group_id, auth.user_id).await?;

        // Compute stats from live member count (fitness snapshots not yet wired)
        let member_count = resources
            .repos
            .groups
            .count_members(&group_id, tenant_id)
            .await?;

        let stats = GroupAggregateStats {
            total_members: member_count,
            active_members: member_count,
            avg_weekly_volume_km: 0.0,
            avg_ctl: None,
            flagged_members: 0,
            weekly_trend: GroupTrend::default(),
        };

        let response = StatsResponse {
            stats,
            metadata: Self::build_metadata(),
        };

        Ok((StatusCode::OK, Json(response)).into_response())
    }

    /// GET `/api/groups/:group_id/report` — Get weekly report for a group
    async fn handle_get_weekly_report(
        State(resources): State<Arc<ServerResources>>,
        headers: HeaderMap,
        Path(group_id): Path<String>,
        Query(_period): Query<PeriodQuery>,
    ) -> Result<Response, AppError> {
        let auth = Self::authenticate(&headers, &resources).await?;
        let tenant_id = Self::get_tenant_id(&auth)?;

        // Verify admin/owner role for reports
        Self::require_admin(&resources, &group_id, auth.user_id).await?;

        let group = resources
            .repos
            .groups
            .get_group(&group_id, tenant_id)
            .await?
            .ok_or_else(|| AppError::not_found(format!("Group {group_id}")))?;

        let member_count = resources
            .repos
            .groups
            .count_members(&group_id, tenant_id)
            .await?;

        // Baseline report without fitness snapshots
        let stats = GroupAggregateStats {
            total_members: member_count,
            active_members: member_count,
            avg_weekly_volume_km: 0.0,
            avg_ctl: None,
            flagged_members: 0,
            weekly_trend: GroupTrend::default(),
        };

        let report = GroupWeeklyReport {
            summary: format!(
                "{} has {} member(s). Fitness snapshot integration pending.",
                group.name, member_count
            ),
            highlights: Vec::new(),
            concerns: Vec::new(),
            recommendations: Vec::new(),
            stats,
        };

        let response = WeeklyReportResponse {
            report,
            metadata: Self::build_metadata(),
        };

        Ok((StatusCode::OK, Json(response)).into_response())
    }

    /// GET `/api/groups/:group_id/health` — Get health flags for group members
    async fn handle_get_health_flags(
        State(resources): State<Arc<ServerResources>>,
        headers: HeaderMap,
        Path(group_id): Path<String>,
    ) -> Result<Response, AppError> {
        let auth = Self::authenticate(&headers, &resources).await?;
        let tenant_id = Self::get_tenant_id(&auth)?;

        // Verify admin/owner role for health flags
        Self::require_admin(&resources, &group_id, auth.user_id).await?;

        // Return empty flags until fitness snapshots are wired
        let response = HealthFlagsResponse {
            flags: Vec::new(),
            total: 0,
            metadata: Self::build_metadata(),
        };

        Ok((StatusCode::OK, Json(response)).into_response())
    }
}

/// Generate an 8-character alphanumeric invite code
fn generate_invite_code() -> String {
    use rand::Rng;
    const CHARSET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let mut rng = rand::thread_rng();
    (0..8)
        .map(|_| {
            let idx = rng.gen_range(0..CHARSET.len());
            CHARSET[idx] as char
        })
        .collect()
}
