// ABOUTME: Group coaching REST routes for group CRUD, membership, invites, and analytics
// ABOUTME: Generic over SocialCtx + MiddlewareCtx; mounted by the composition root
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Group coaching routes (CRUD + membership + invites)
//!
//! This module handles REST endpoints for coaching group management:
//! group CRUD, membership, invites, and join/leave flows. All endpoints
//! require JWT authentication. Admin-only operations check the caller's
//! `GroupRole`.
//!
//! The three analytics endpoints (`/api/groups/{id}/{stats,report,health}`)
//! live in the sibling [`crate::group_analytics`] module and are merged
//! into the same `/api/groups` mount by the composition root.

use tracing::{field, info, warn, Span};

use axum::{
    extract::{Path, State},
    http::StatusCode,
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

use pierre_auth::auth::AuthResult;
use pierre_core::errors::{AppError, ErrorCode};
use pierre_core::models::groups::{
    CoachingGroup, CreateGroupRequest, GroupAggregateStats, GroupHealthFlag, GroupInvite,
    GroupInviteKind, GroupMember, GroupRespondMode, GroupRole, GroupSummary, GroupWeeklyReport,
    JoinGroupRequest, UpdateGroupRequest,
};
use pierre_core::models::TenantId;
use pierre_groups::strategies::tier::tier_strategy_for;
use pierre_middleware::AuthenticatedUser;
use pierre_runtime_context::{MiddlewareCtx, SocialCtx};

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
    /// Coach persona ID (the AI coach that answers chats)
    pub coach_id: String,
    /// Owner user ID
    pub owner_id: String,
    /// Human coach user ID, if one is attached (`None` otherwise)
    pub coach_user_id: Option<String>,
    /// Whether peer data sharing is enabled
    pub peer_data_sharing: bool,
    /// When the AI coach replies in the bound channel chat
    pub respond_mode: GroupRespondMode,
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
            coach_user_id: g.coach_user_id.map(|u| u.to_string()),
            peer_data_sharing: g.peer_data_sharing,
            respond_mode: g.respond_mode,
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

/// Response for listing the groups a user is the human coach of
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct CoachedGroupsResponse {
    /// Groups the user coaches
    pub groups: Vec<GroupResponse>,
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
    /// When the invite was created
    pub created_at: String,
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

/// Response for group creation permissions check
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct GroupPermissionsResponse {
    /// Whether the current user can create groups
    pub can_create: bool,
    /// Current group creation policy for the tenant
    pub policy: String,
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
    /// What the invite grants: athlete membership (default) or coach
    /// attachment. Omitted → `member`.
    #[serde(default)]
    pub kind: GroupInviteKind,
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
    pub fn routes<C: SocialCtx + MiddlewareCtx>(resources: Arc<C>) -> Router {
        Router::new()
            // Group CRUD
            .route("/api/groups", post(Self::handle_create_group::<C>))
            .route("/api/groups", get(Self::handle_list_my_groups::<C>))
            .route("/api/groups/{group_id}", get(Self::handle_get_group::<C>))
            .route(
                "/api/groups/{group_id}",
                put(Self::handle_update_group::<C>),
            )
            .route(
                "/api/groups/{group_id}",
                delete(Self::handle_delete_group::<C>),
            )
            // Membership
            .route(
                "/api/groups/{group_id}/members",
                get(Self::handle_list_members::<C>),
            )
            .route(
                "/api/groups/{group_id}/members/{user_id}",
                delete(Self::handle_remove_member::<C>),
            )
            .route(
                "/api/groups/{group_id}/members/{user_id}/role",
                put(Self::handle_update_role::<C>),
            )
            .route(
                "/api/groups/{group_id}/members/me/consent",
                put(Self::handle_update_peer_consent::<C>),
            )
            // Human coach attachment
            .route(
                "/api/groups/coached",
                get(Self::handle_list_coached_groups::<C>),
            )
            .route(
                "/api/groups/{group_id}/coach",
                delete(Self::handle_remove_coach::<C>),
            )
            // Invites
            .route(
                "/api/groups/{group_id}/invites",
                post(Self::handle_create_invite::<C>),
            )
            .route(
                "/api/groups/{group_id}/invites",
                get(Self::handle_list_invites::<C>),
            )
            .route(
                "/api/groups/{group_id}/invites/{invite_id}",
                delete(Self::handle_deactivate_invite::<C>),
            )
            // Permissions
            .route(
                "/api/groups/permissions",
                get(Self::handle_get_permissions::<C>),
            )
            // Join / Leave
            .route(
                "/api/groups/join",
                post(Self::handle_join_by_invite_code::<C>),
            )
            .route(
                "/api/groups/{group_id}/leave",
                post(Self::handle_leave_group::<C>),
            )
            // Analytics endpoints (`/stats`, `/report`, `/health`) require
            // a fitness-snapshot builder that needs an owned
            // `Arc<dyn ToolRuntime>` from the composition root —
            // `pierre-server` mounts them separately alongside this router.
            .with_state(resources)
    }

    // ========================================================================
    // Helpers
    // ========================================================================

    /// Extract tenant ID from auth claims
    fn get_tenant_id(auth: &AuthResult) -> Result<TenantId, AppError> {
        auth.active_tenant_id
            .map(TenantId::from_uuid)
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
    async fn check_create_group_permission<C: SocialCtx + MiddlewareCtx>(
        resources: &Arc<C>,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> Result<(), AppError> {
        // Check tenant role — admins/owners always allowed
        let user_role = match resources
            .repos()
            .tenants
            .get_user_role(user_id, tenant_id)
            .await
        {
            Ok(role) => role,
            Err(e) => {
                warn!(
                    %user_id, %tenant_id, error = %e,
                    "Failed to read tenant role during group-creation permission check; \
                     proceeding without admin shortcut and falling back to policy evaluation"
                );
                None
            }
        };

        let is_tenant_admin = user_role
            .as_deref()
            .is_some_and(|r| r == "owner" || r == "admin");

        if is_tenant_admin {
            return Ok(());
        }

        // For regular users, check group_creation_policy via admin config
        // Default policy: admins_only (most restrictive)
        let policy = resources
            .admin_config_get("group_creation_policy", Some(&tenant_id.to_string()))
            .await
            .and_then(|v| v.as_str().map(ToOwned::to_owned))
            .unwrap_or_else(|| "admins_only".to_owned());

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
    async fn require_admin<C: SocialCtx + MiddlewareCtx>(
        resources: &Arc<C>,
        group_id: &str,
        user_id: Uuid,
    ) -> Result<GroupMember, AppError> {
        let member = resources
            .repos()
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
    async fn require_owner<C: SocialCtx + MiddlewareCtx>(
        resources: &Arc<C>,
        group_id: &str,
        user_id: Uuid,
    ) -> Result<GroupMember, AppError> {
        let member = resources
            .repos()
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
    async fn require_member<C: SocialCtx + MiddlewareCtx>(
        resources: &Arc<C>,
        group_id: &str,
        user_id: Uuid,
    ) -> Result<GroupMember, AppError> {
        resources
            .repos()
            .groups
            .get_member(group_id, user_id)
            .await?
            .ok_or_else(|| AppError::not_found("You are not a member of this group"))
    }

    // ========================================================================
    // Group CRUD handlers
    // ========================================================================

    /// POST /api/groups — Create a new coaching group
    #[tracing::instrument(
        skip(resources, auth, body),
        fields(
            route = "groups_create",
            user_id = field::Empty,
            tenant_id = field::Empty,
            group_id = field::Empty,
        )
    )]
    async fn handle_create_group<C: SocialCtx + MiddlewareCtx>(
        State(resources): State<Arc<C>>,
        auth: AuthenticatedUser,
        Json(body): Json<CreateGroupRequest>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let tenant_id = Self::get_tenant_id(&auth)?;

        // Record IDs on the span so the NotifyLayer can attribute the
        // group.created event without re-passing tenant/user fields.
        let span = Span::current();
        span.record("user_id", field::display(&auth.user_id));
        span.record("tenant_id", field::display(&tenant_id));

        // Check group creation permission:
        // 1. Tenant admins/owners always allowed
        // 2. Regular users need group_creation_policy = "everyone"
        Self::check_create_group_permission(&resources, auth.user_id, tenant_id).await?;

        if body.name.trim().is_empty() {
            return Err(AppError::invalid_input("Group name must not be empty"));
        }

        // Resolve the tenant's plan tier (the handler owns tenant-plan
        // access). The per-group member cap is passed into
        // GroupService::create_group, which owns the actual creation, the
        // member-count clamp, and the Starter rejection (cap 0 →
        // PermissionDenied). Group creation + owner auto-membership live in
        // one place — the service — so this handler stays a thin HTTP layer.
        let plan = resources.repos().tenants.get_by_id(tenant_id).await?.plan;
        let tier_cap =
            i32::try_from(tier_strategy_for(&plan).max_members_per_group()).unwrap_or(i32::MAX);

        let created = resources
            .group_service()
            .create_group(&body, auth.user_id, tenant_id, tier_cap)
            .await?;

        // notify: a new coaching group exists. group_id is the freshly
        // minted Uuid from `created`; record on the span so future child
        // spans inherit it.
        Span::current().record("group_id", field::display(&created.id));
        info!(
            target: "notify",
            event = "group.created",
            group_id = %created.id,
            "coaching group created"
        );

        let response: GroupResponse = created.into();
        Ok((StatusCode::CREATED, Json(response)).into_response())
    }

    /// GET /api/groups/permissions — Check if the current user can create groups
    async fn handle_get_permissions<C: SocialCtx + MiddlewareCtx>(
        State(resources): State<Arc<C>>,
        auth: AuthenticatedUser,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let tenant_id = Self::get_tenant_id(&auth)?;

        // Check tenant role — admins/owners always allowed
        let user_role = match resources
            .repos()
            .tenants
            .get_user_role(auth.user_id, tenant_id)
            .await
        {
            Ok(role) => role,
            Err(e) => {
                warn!(
                    user_id = %auth.user_id, %tenant_id, error = %e,
                    "Failed to read tenant role while computing group permissions; \
                     reporting non-admin and applying configured policy"
                );
                None
            }
        };

        let is_tenant_admin = user_role
            .as_deref()
            .is_some_and(|r| r == "owner" || r == "admin");

        // Retrieve the group_creation_policy from admin config
        let policy = resources
            .admin_config_get("group_creation_policy", Some(&tenant_id.to_string()))
            .await
            .and_then(|v| v.as_str().map(ToOwned::to_owned))
            .unwrap_or_else(|| "admins_only".to_owned());

        let can_create = is_tenant_admin || policy == "everyone";

        let response = GroupPermissionsResponse { can_create, policy };

        Ok((StatusCode::OK, Json(response)).into_response())
    }

    /// GET /api/groups — List groups the current user belongs to
    async fn handle_list_my_groups<C: SocialCtx + MiddlewareCtx>(
        State(resources): State<Arc<C>>,
        auth: AuthenticatedUser,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();

        let groups = resources.group_service().list_groups(auth.user_id).await?;

        let response = ListGroupsResponse {
            total: groups.len(),
            groups,
            metadata: Self::build_metadata(),
        };

        Ok((StatusCode::OK, Json(response)).into_response())
    }

    /// GET `/api/groups/:group_id` — Get a single group by ID
    async fn handle_get_group<C: SocialCtx + MiddlewareCtx>(
        State(resources): State<Arc<C>>,
        auth: AuthenticatedUser,
        Path(group_id): Path<String>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let tenant_id = Self::get_tenant_id(&auth)?;

        // Verify caller is a member
        Self::require_member(&resources, &group_id, auth.user_id).await?;

        let group = resources
            .group_service()
            .get_group(&group_id, tenant_id)
            .await?
            .ok_or_else(|| AppError::not_found(format!("Group {group_id}")))?;

        let response: GroupResponse = group.into();
        Ok((StatusCode::OK, Json(response)).into_response())
    }

    /// PUT `/api/groups/:group_id` — Update group settings (admin/owner only)
    async fn handle_update_group<C: SocialCtx + MiddlewareCtx>(
        State(resources): State<Arc<C>>,
        auth: AuthenticatedUser,
        Path(group_id): Path<String>,
        Json(body): Json<UpdateGroupRequest>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let tenant_id = Self::get_tenant_id(&auth)?;

        // Verify admin/owner role
        Self::require_admin(&resources, &group_id, auth.user_id).await?;

        let updated = resources
            .group_service()
            .update_group(&group_id, tenant_id, &body)
            .await?
            .ok_or_else(|| AppError::not_found(format!("Group {group_id}")))?;

        let response: GroupResponse = updated.into();
        Ok((StatusCode::OK, Json(response)).into_response())
    }

    /// DELETE `/api/groups/:group_id` — Soft-delete a group (owner only)
    async fn handle_delete_group<C: SocialCtx + MiddlewareCtx>(
        State(resources): State<Arc<C>>,
        auth: AuthenticatedUser,
        Path(group_id): Path<String>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let tenant_id = Self::get_tenant_id(&auth)?;

        // Only owner can delete
        Self::require_owner(&resources, &group_id, auth.user_id).await?;

        let deleted = resources
            .group_service()
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
    async fn handle_list_members<C: SocialCtx + MiddlewareCtx>(
        State(resources): State<Arc<C>>,
        auth: AuthenticatedUser,
        Path(group_id): Path<String>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();

        // Verify caller is a member
        Self::require_member(&resources, &group_id, auth.user_id).await?;

        let members = resources.group_service().list_members(&group_id).await?;

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
    async fn handle_remove_member<C: SocialCtx + MiddlewareCtx>(
        State(resources): State<Arc<C>>,
        auth: AuthenticatedUser,
        Path((group_id, target_user_id)): Path<(String, String)>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();

        // Verify admin/owner role
        Self::require_admin(&resources, &group_id, auth.user_id).await?;

        let target_uuid = Uuid::parse_str(&target_user_id)
            .map_err(|_| AppError::invalid_input("Invalid user_id format"))?;

        // Cannot remove the owner
        let target_member = resources
            .repos()
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
            .group_service()
            .remove_member(&group_id, target_uuid)
            .await?;

        if !removed {
            return Err(AppError::not_found("Member not found"));
        }

        Ok((StatusCode::NO_CONTENT, ()).into_response())
    }

    /// PUT `/api/groups/:group_id/members/:user_id/role` — Update member role (admin/owner only)
    async fn handle_update_role<C: SocialCtx + MiddlewareCtx>(
        State(resources): State<Arc<C>>,
        auth: AuthenticatedUser,
        Path((group_id, target_user_id)): Path<(String, String)>,
        Json(body): Json<UpdateRoleBody>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();

        let caller_member = Self::require_admin(&resources, &group_id, auth.user_id).await?;

        let target_uuid = Uuid::parse_str(&target_user_id)
            .map_err(|_| AppError::invalid_input("Invalid user_id format"))?;

        // Cannot change the owner's role via this endpoint. The owner is protected
        // exactly as in `handle_remove_member` (which rejects removing the owner):
        // without this check a mere group admin could demote the owner to member.
        // Ownership changes go through the dedicated transfer flow, not a role edit.
        let target_member = resources
            .repos()
            .groups
            .get_member(&group_id, target_uuid)
            .await?
            .ok_or_else(|| AppError::not_found("Target member not found"))?;

        if target_member.role == GroupRole::Owner {
            return Err(AppError::new(
                ErrorCode::PermissionDenied,
                "Cannot change the group owner's role",
            ));
        }

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
            .group_service()
            .update_member_role(&group_id, target_uuid, body.role)
            .await?;

        if !updated {
            return Err(AppError::not_found("Member not found"));
        }

        // Fetch updated member to return
        let member = resources
            .repos()
            .groups
            .get_member(&group_id, target_uuid)
            .await?
            .ok_or_else(|| AppError::internal("Failed to fetch updated member"))?;

        let response: MemberResponse = member.into();
        Ok((StatusCode::OK, Json(response)).into_response())
    }

    /// PUT `/api/groups/:group_id/members/me/consent` — Update own peer sharing consent
    async fn handle_update_peer_consent<C: SocialCtx + MiddlewareCtx>(
        State(resources): State<Arc<C>>,
        auth: AuthenticatedUser,
        Path(group_id): Path<String>,
        Json(body): Json<UpdatePeerConsentBody>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();

        // Verify caller is a member
        Self::require_member(&resources, &group_id, auth.user_id).await?;

        let updated = resources
            .repos()
            .groups
            .update_peer_sharing_consent(&group_id, auth.user_id, body.consent)
            .await?;

        if !updated {
            return Err(AppError::not_found("Membership not found"));
        }

        // Fetch updated member
        let member = resources
            .repos()
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
    async fn handle_create_invite<C: SocialCtx + MiddlewareCtx>(
        State(resources): State<Arc<C>>,
        auth: AuthenticatedUser,
        Path(group_id): Path<String>,
        Json(body): Json<CreateInviteBody>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
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

        let created = resources
            .group_service()
            .create_invite(
                group_uuid,
                auth.user_id,
                tenant_id,
                body.expires_in_days,
                body.max_uses,
                body.kind,
            )
            .await?;

        let response: InviteResponse = created.into();
        Ok((StatusCode::CREATED, Json(response)).into_response())
    }

    /// GET `/api/groups/:group_id/invites` — List invites for a group (admin/owner only)
    async fn handle_list_invites<C: SocialCtx + MiddlewareCtx>(
        State(resources): State<Arc<C>>,
        auth: AuthenticatedUser,
        Path(group_id): Path<String>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();

        // Verify admin/owner role
        Self::require_admin(&resources, &group_id, auth.user_id).await?;

        let invites = resources.group_service().list_invites(&group_id).await?;

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
    async fn handle_deactivate_invite<C: SocialCtx + MiddlewareCtx>(
        State(resources): State<Arc<C>>,
        auth: AuthenticatedUser,
        Path((group_id, invite_id)): Path<(String, String)>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();

        // Verify admin/owner role
        Self::require_admin(&resources, &group_id, auth.user_id).await?;

        // Scope the deactivation to the group in the path: require_admin only
        // proves the caller administers `group_id`, so the repo update must also
        // filter by `group_id` to stop a group admin deactivating another
        // group's invite (IDOR).
        let deactivated = resources
            .repos()
            .groups
            .deactivate_invite(&group_id, &invite_id)
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
    #[tracing::instrument(
        skip(resources, auth, body),
        fields(
            route = "groups_join",
            user_id = field::Empty,
            tenant_id = field::Empty,
            group_id = field::Empty,
        )
    )]
    async fn handle_join_by_invite_code<C: SocialCtx + MiddlewareCtx>(
        State(resources): State<Arc<C>>,
        auth: AuthenticatedUser,
        Json(body): Json<JoinGroupRequest>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        Span::current().record("user_id", field::display(&auth.user_id));
        // Caller's tenant is not used — the invite's tenant (group's tenant) is used instead

        if body.invite_code.trim().is_empty() {
            return Err(AppError::invalid_input("Invite code must not be empty"));
        }

        // Resolve the invite's tenant (= group's tenant) for the cross-tenant
        // join: membership is created under the group's tenant, not the
        // caller's home tenant. The handler reads the invite once for this
        // tenant resolution + notification span fields; GroupService::join_group
        // is the single implementation of the membership business logic
        // (invite validity, capacity, already-a-member guard, invite-use
        // increment, member insert).
        let invite = resources
            .repos()
            .groups
            .get_invite_by_code(&body.invite_code)
            .await?
            .ok_or_else(|| AppError::not_found("Invalid or expired invite code"))?;

        let group_tenant_id = TenantId::parse_str(&invite.tenant_id)
            .map_err(|e| AppError::internal(format!("Invalid invite tenant: {e}")))?;

        // Coach invites attach the redeemer as the group's human coach; member
        // invites add an athlete. Dispatch on the invite kind.
        match invite.kind {
            GroupInviteKind::Member => {
                let created = resources
                    .group_service()
                    .join_group(&body.invite_code, auth.user_id, group_tenant_id)
                    .await?;

                // notify: user joined a group. The invite's tenant carries the
                // group's tenant — record both so the Slack ping reflects the
                // group context, not the caller's home tenant.
                let span = Span::current();
                span.record("tenant_id", field::display(&group_tenant_id));
                span.record("group_id", field::display(&invite.group_id));
                info!(
                    target: "notify",
                    event = "group.joined",
                    group_id = %invite.group_id,
                    "user joined coaching group"
                );

                let response: MemberResponse = created.into();
                Ok((StatusCode::CREATED, Json(response)).into_response())
            }
            GroupInviteKind::Coach => {
                // Coach eligibility: the redeemer must be a roster-managing
                // coach (`manages_roster`) or a platform admin — the same gate
                // the `/api/roster` endpoints use.
                let user = resources
                    .repos()
                    .users
                    .get_global(auth.user_id)
                    .await?
                    .ok_or_else(|| AppError::not_found("User not found"))?;
                if !user.manages_roster && !user.is_admin {
                    return Err(AppError::new(
                        ErrorCode::PermissionDenied,
                        "Only roster-managing coaches can join a group as its coach",
                    ));
                }

                // Cross-tenant rule (v1): a human coach must belong to the
                // group's tenant. Athlete membership is cross-tenant by design,
                // but coach attachment is tenant-scoped to match the
                // tenant-scoped roster/coach model.
                let caller_tenant = Self::get_tenant_id(&auth)?;
                if caller_tenant != group_tenant_id {
                    return Err(AppError::new(
                        ErrorCode::PermissionDenied,
                        "A coach must belong to the group's tenant to join it",
                    ));
                }

                let group = resources
                    .group_service()
                    .redeem_coach_invite(&body.invite_code, auth.user_id, group_tenant_id)
                    .await?;

                let span = Span::current();
                span.record("tenant_id", field::display(&group_tenant_id));
                span.record("group_id", field::display(&invite.group_id));
                // Reuses the catalogued `group.joined` event (a coach
                // redeeming a coach-kind invite is still a join); the message
                // distinguishes the coach case for operators.
                info!(
                    target: "notify",
                    event = "group.joined",
                    group_id = %invite.group_id,
                    "coach joined coaching group"
                );

                let response: GroupResponse = group.into();
                Ok((StatusCode::CREATED, Json(response)).into_response())
            }
        }
    }

    /// POST `/api/groups/:group_id/leave` — Leave a group
    async fn handle_leave_group<C: SocialCtx + MiddlewareCtx>(
        State(resources): State<Arc<C>>,
        auth: AuthenticatedUser,
        Path(group_id): Path<String>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();

        let member = Self::require_member(&resources, &group_id, auth.user_id).await?;

        // Owner cannot leave — must transfer ownership or delete the group
        if member.role == GroupRole::Owner {
            return Err(AppError::new(
                ErrorCode::PermissionDenied,
                "Owner cannot leave the group. Transfer ownership or delete the group instead.",
            ));
        }

        let left = resources
            .group_service()
            .leave_group(&group_id, auth.user_id)
            .await?;

        if !left {
            return Err(AppError::internal("Failed to remove membership"));
        }

        Ok((StatusCode::NO_CONTENT, ()).into_response())
    }

    // ========================================================================
    // Human coach handlers
    // ========================================================================

    /// GET /api/groups/coached — List groups the caller is the human coach of
    async fn handle_list_coached_groups<C: SocialCtx + MiddlewareCtx>(
        State(resources): State<Arc<C>>,
        auth: AuthenticatedUser,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();

        let groups = resources
            .group_service()
            .list_coached_groups(auth.user_id)
            .await?;

        let group_responses: Vec<GroupResponse> =
            groups.into_iter().map(GroupResponse::from).collect();

        let response = CoachedGroupsResponse {
            total: group_responses.len(),
            groups: group_responses,
            metadata: Self::build_metadata(),
        };

        Ok((StatusCode::OK, Json(response)).into_response())
    }

    /// DELETE `/api/groups/:group_id/coach` — Detach the human coach (admin/owner only)
    async fn handle_remove_coach<C: SocialCtx + MiddlewareCtx>(
        State(resources): State<Arc<C>>,
        auth: AuthenticatedUser,
        Path(group_id): Path<String>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let tenant_id = Self::get_tenant_id(&auth)?;

        // Verify admin/owner role
        Self::require_admin(&resources, &group_id, auth.user_id).await?;

        let cleared = resources
            .group_service()
            .set_group_coach(&group_id, None, tenant_id)
            .await?;

        if !cleared {
            return Err(AppError::not_found(format!("Group {group_id}")));
        }

        Ok((StatusCode::NO_CONTENT, ()).into_response())
    }
}
