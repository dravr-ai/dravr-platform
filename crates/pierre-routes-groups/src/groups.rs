// ABOUTME: Group coaching REST routes for group CRUD, membership, invites, and analytics
// ABOUTME: Generic over GroupsCtx + MiddlewareCtx; mounted by the composition root
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

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{delete, get, post, put},
    Json, Router,
};
use dravr_tronc::dates::Timestamp;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{info, warn};
use uuid::Uuid;

use pierre_auth::auth::AuthResult;
use pierre_core::errors::{AppError, ErrorCode};
use pierre_core::models::groups::{
    CoachingGroup, CreateGroupRequest, GroupInviteKind, GroupMember, GroupRole, UpdateGroupRequest,
};
use pierre_core::models::TenantId;
use pierre_groups::creation_policy::{
    check_create_group_permission, is_tenant_group_admin, may_coach_group,
    policy_permits_group_creation, DEFAULT_GROUP_CREATION_POLICY, GROUP_CREATION_POLICY_KEY,
};
use pierre_groups::group_agent::require_athlete_facing;
use pierre_groups::strategies::tier::{tier_enables_digest, tier_strategy_for};
use pierre_middleware::AuthenticatedUser;
use pierre_runtime_context::{GroupsCtx, MiddlewareCtx};
use pierre_services::coach_access::{name_requested_group, CoachAccessNotices};
use pierre_services::coach_consents;
use pierre_services::group_coach_claim::claim_group_coach;
use pierre_services::group_invite_card;
use pierre_services::locale::resolve_user_locale;

use crate::group_response::GroupResponse;
use crate::group_update_access::authorize_group_update;
use crate::outside_roster::members_served_to;

mod bodies;
pub use bodies::{
    CoachSharingBody, CreateGroupBody, CreateInviteBody, PeriodQuery, UpdatePeerConsentBody,
    UpdateRoleBody,
};

mod responses;
pub use responses::{
    AnalyticsScope, ClaimGroupCoachResponse, CoachSharingResponse, GroupMetadata,
    GroupPermissionsResponse, HealthFlagsResponse, InviteResponse, InviteWithFunnelResponse,
    ListInvitesResponse, ListMembersResponse, MemberResponse, StatsResponse, WeeklyReportResponse,
};

// ============================================================================
// Routes
// ============================================================================

/// Group coaching route handler
#[derive(Debug)]
pub struct GroupRoutes;

impl GroupRoutes {
    /// Create all group coaching routes
    pub fn routes<C: GroupsCtx + MiddlewareCtx>(resources: Arc<C>) -> Router {
        Router::new()
            // Group CRUD
            .route("/api/groups", post(Self::handle_create_group::<C>))
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
            .route(
                "/api/groups/{group_id}/members/me/coach-sharing",
                get(Self::handle_get_coach_sharing::<C>),
            )
            .route(
                "/api/groups/{group_id}/members/me/coach-sharing",
                put(Self::handle_update_coach_sharing::<C>),
            )
            // Human coach attachment
            .route(
                "/api/groups/{group_id}/coach",
                delete(Self::handle_remove_coach::<C>),
            )
            // The owner with coach access becomes the coach of a coachless group
            .route(
                "/api/groups/{group_id}/coach",
                put(Self::handle_claim_coach::<C>),
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
            timestamp: Timestamp::now().to_rfc3339(),
            api_version: "1.0".to_owned(),
        }
    }

    /// `group` as `reader` reads it: its agent and human coach named, the
    /// agent's title in the reader's own locale.
    async fn group_response<C: GroupsCtx + MiddlewareCtx>(
        resources: &Arc<C>,
        group: CoachingGroup,
        reader: Uuid,
    ) -> Result<GroupResponse, AppError> {
        let repos = resources.repos();
        let locale = resolve_user_locale(repos.users.as_ref(), reader).await;
        GroupResponse::for_reader(repos, group, reader, &locale).await
    }

    /// Verify the caller is an admin or owner of the given group.
    ///
    /// Returns the caller's membership record on success.
    async fn require_admin<C: GroupsCtx + MiddlewareCtx>(
        resources: &Arc<C>,
        group_id: &str,
        user_id: Uuid,
    ) -> Result<GroupMember, AppError> {
        let member = resources
            .repos()
            .groups
            .get_member(group_id, user_id)
            .await?;
        Self::admin_membership(member)
    }

    /// Refuse a caller who does not manage a roster: only a coach whose
    /// coaching an admin granted may mint codes that approve accounts.
    async fn require_roster_manager<C: GroupsCtx + MiddlewareCtx>(
        resources: &Arc<C>,
        user_id: Uuid,
    ) -> Result<(), AppError> {
        let manages = resources
            .repos()
            .users
            .get_global(user_id)
            .await?
            .is_some_and(|user| user.manages_roster);
        if manages {
            return Ok(());
        }
        Err(AppError::new(
            ErrorCode::PermissionDenied,
            "Only a coach with coach access can create signup codes",
        ))
    }

    /// The caller's membership when it holds an admin or owner role, and the
    /// refusal [`Self::require_admin`] answers with when it does not.
    pub(crate) fn admin_membership(member: Option<GroupMember>) -> Result<GroupMember, AppError> {
        let member = member.ok_or_else(|| AppError::not_found("Membership not found"))?;

        if !member.role.can_manage_members() {
            return Err(AppError::new(
                ErrorCode::PermissionDenied,
                "Admin or owner role required for this operation",
            ));
        }

        Ok(member)
    }

    /// Verify the caller is the owner of the given group.
    async fn require_owner<C: GroupsCtx + MiddlewareCtx>(
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
    async fn require_member<C: GroupsCtx + MiddlewareCtx>(
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

    /// Verify the caller may read the group: a live member of an active
    /// group, or its human coach
    /// ([`CoachingGroupRepository::admits_member_or_coach`](pierre_database::repositories::CoachingGroupRepository::admits_member_or_coach)).
    ///
    /// The coach holds no membership row, yet sees the group's info and the
    /// member list its members already see, to link their `TrainingPeaks`
    /// athletes; every other surface keeps its member gate.
    async fn require_member_or_coach<C: GroupsCtx + MiddlewareCtx>(
        resources: &Arc<C>,
        group_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> Result<(), AppError> {
        resources
            .repos()
            .groups
            .admits_member_or_coach(group_id, user_id, tenant_id)
            .await?
            .ok_or_else(|| AppError::not_found("You are not a member of this group"))
    }

    // ========================================================================
    // Group CRUD handlers
    // ========================================================================

    /// POST /api/groups — Create a coaching group, its creator optionally as
    /// its human coach.
    ///
    /// Behind the same gates as `/group create`: the tenant's creation policy
    /// and the plan's per-group cap (zero refuses). The creator's group chat
    /// is opened by the client through `POST /api/chat/conversations` with
    /// the returned id, under the conversation quota every thread obeys.
    ///
    /// A creator who asks to coach the group without holding coach access
    /// gets it coachless; when their waiting coach-access request names no
    /// group, it names this one, so a grant attaches them to it.
    async fn handle_create_group<C: GroupsCtx + MiddlewareCtx>(
        State(resources): State<Arc<C>>,
        auth: AuthenticatedUser,
        Json(body): Json<CreateGroupBody>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let tenant_id = Self::get_tenant_id(&auth)?;
        let repos = resources.repos();

        let name = body.name.trim();
        if name.is_empty() {
            return Err(AppError::invalid_input("Group name must not be empty"));
        }

        check_create_group_permission(
            repos.tenants.as_ref(),
            auth.user_id,
            tenant_id,
            Self::group_creation_policy(&resources, tenant_id),
        )
        .await?;

        // The agent must be one the caller can read in this tenant, so a
        // stale or foreign id never makes a group nobody can talk to.
        let Some(agent_id) = body.agent_id else {
            return Err(AppError::invalid_input(
                "Choose the group's agent before creating it",
            ));
        };
        let agent = repos
            .agents
            .get_by_id(&agent_id, auth.user_id, tenant_id)
            .await?
            .ok_or_else(|| AppError::not_found(format!("Agent {agent_id}")))?;
        require_athlete_facing(&agent)?;

        // ADR-018: asking to coach never grants coaching. A caller without
        // the right gets the group, coachless, and the client says so.
        let coach = if body.coach_is_me {
            Some(
                repos
                    .users
                    .get_global(auth.user_id)
                    .await?
                    .ok_or_else(|| AppError::not_found("User not found"))?,
            )
        } else {
            None
        };
        let coach_user_id = coach
            .as_ref()
            .filter(|user| may_coach_group(user, tenant_id, tenant_id))
            .map(|_| auth.user_id);

        let plan = repos.tenants.get_by_id(tenant_id).await?.plan;
        let plan_tier = tier_strategy_for(&plan);

        let request = CreateGroupRequest {
            name: name.to_owned(),
            description: body.description,
            agent_id: agent.id.to_string(),
            max_members: None,
            coach_user_id,
        };
        // `group.created` is emitted by the service, once for every surface.
        let created = resources
            .group_service()
            .create_group(&request, auth.user_id, tenant_id, plan_tier.as_ref())
            .await?;

        // `coach_is_me` is sent only by the onboarding "Your group" step on
        // both surfaces, so this times a coach's setup from their signup
        // (carnet#739). `coach_attached` is false while coach access is
        // pending.
        if let Some(coach) = &coach {
            info!(
                target: "notify",
                event = "onboarding.coach_group_created",
                user_id = %auth.user_id,
                tenant_id = %tenant_id,
                group_id = %created.id,
                coach_attached = created.coach_user_id.is_some(),
                seconds_since_signup = Timestamp::now()
                    .signed_duration_since(coach.created_at)
                    .num_seconds()
                    .max(0),
                "coach created their group during onboarding"
            );
            // A coach-access request already waiting is the one this group's
            // step shows as sent, so a grant must attach the coach to it.
            if created.coach_user_id.is_none() {
                if let Err(e) =
                    name_requested_group(repos, auth.user_id, (created.id, tenant_id)).await
                {
                    warn!(
                        user_id = %auth.user_id,
                        group_id = %created.id,
                        error = %e,
                        "Could not name the new group on the coach's waiting coach access request"
                    );
                }
            }
        }

        let response = Self::group_response(&resources, created, auth.user_id).await?;
        Ok((StatusCode::CREATED, Json(response)).into_response())
    }

    /// The tenant's configured group-creation policy, when it has set one.
    async fn group_creation_policy<C: GroupsCtx + MiddlewareCtx>(
        resources: &Arc<C>,
        tenant_id: TenantId,
    ) -> Option<String> {
        resources
            .admin_config_get(GROUP_CREATION_POLICY_KEY, Some(&tenant_id.to_string()))
            .await
            .and_then(|v| v.as_str().map(ToOwned::to_owned))
    }

    /// GET /api/groups/permissions — Check if the current user can create groups
    async fn handle_get_permissions<C: GroupsCtx + MiddlewareCtx>(
        State(resources): State<Arc<C>>,
        auth: AuthenticatedUser,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let tenant_id = Self::get_tenant_id(&auth)?;

        // The policy is reported back whether or not it decides, so it is
        // read eagerly here rather than through the lazy create-path check.
        let is_tenant_admin =
            is_tenant_group_admin(resources.repos().tenants.as_ref(), auth.user_id, tenant_id)
                .await;
        let policy = Self::group_creation_policy(&resources, tenant_id)
            .await
            .unwrap_or_else(|| DEFAULT_GROUP_CREATION_POLICY.to_owned());

        let can_create = is_tenant_admin || policy_permits_group_creation(&policy).is_ok();

        // The tier flag the digest scheduler gates on, resolved from the
        // tenant's own plan so the clients paint the server's decision.
        let plan = resources.repos().tenants.get_by_id(tenant_id).await?.plan;
        let weekly_digest = tier_enables_digest(&plan);

        let response = GroupPermissionsResponse {
            can_create,
            policy,
            weekly_digest,
        };

        Ok((StatusCode::OK, Json(response)).into_response())
    }

    /// GET `/api/groups/:group_id` — Get a single group by ID
    async fn handle_get_group<C: GroupsCtx + MiddlewareCtx>(
        State(resources): State<Arc<C>>,
        auth: AuthenticatedUser,
        Path(group_id): Path<String>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let tenant_id = Self::get_tenant_id(&auth)?;

        Self::require_member_or_coach(&resources, &group_id, auth.user_id, tenant_id).await?;

        let group = resources
            .group_service()
            .get_group(&group_id, tenant_id)
            .await?
            .ok_or_else(|| AppError::not_found(format!("Group {group_id}")))?;

        let response = Self::group_response(&resources, group, auth.user_id).await?;
        Ok((StatusCode::OK, Json(response)).into_response())
    }

    /// PUT `/api/groups/:group_id` — Update group settings (admin/owner; the
    /// attached human coach may change the weekly digest mode only)
    async fn handle_update_group<C: GroupsCtx + MiddlewareCtx>(
        State(resources): State<Arc<C>>,
        auth: AuthenticatedUser,
        Path(group_id): Path<String>,
        Json(body): Json<UpdateGroupRequest>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let tenant_id = Self::get_tenant_id(&auth)?;

        authorize_group_update(&resources, &group_id, &auth, tenant_id, &body).await?;
        // Resolved in the scope the group's turns resolve it in — the
        // tenant's own agents and system agents — so an admin can still
        // resend another owner's in-tenant agent, while an id the runtime
        // could not run is refused and a coach-facing one never binds.
        if let Some(agent_id) = body.agent_id.as_deref() {
            let agent = resources
                .repos()
                .agents
                .get_in_tenant(agent_id, tenant_id)
                .await?
                .ok_or_else(|| AppError::not_found(format!("Agent {agent_id}")))?;
            require_athlete_facing(&agent)?;
        }

        let updated = resources
            .group_service()
            .update_group(&group_id, tenant_id, &body, Some(auth.user_id))
            .await?
            .ok_or_else(|| AppError::not_found(format!("Group {group_id}")))?;

        let response = Self::group_response(&resources, updated, auth.user_id).await?;
        Ok((StatusCode::OK, Json(response)).into_response())
    }

    /// DELETE `/api/groups/:group_id` — Soft-delete a group (owner only)
    async fn handle_delete_group<C: GroupsCtx + MiddlewareCtx>(
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
            .delete_group(&group_id, tenant_id, auth.user_id)
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
    async fn handle_list_members<C: GroupsCtx + MiddlewareCtx>(
        State(resources): State<Arc<C>>,
        auth: AuthenticatedUser,
        Path(group_id): Path<String>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let tenant_id = Self::get_tenant_id(&auth)?;

        Self::require_member_or_coach(&resources, &group_id, auth.user_id, tenant_id).await?;

        let members = resources.group_service().list_members(&group_id).await?;
        let members = members_served_to(&resources, &auth, &group_id, tenant_id, members).await?;

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
    async fn handle_remove_member<C: GroupsCtx + MiddlewareCtx>(
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
            .remove_member(&group_id, target_uuid, auth.user_id)
            .await?;

        if !removed {
            return Err(AppError::not_found("Member not found"));
        }

        Ok((StatusCode::NO_CONTENT, ()).into_response())
    }

    /// PUT `/api/groups/:group_id/members/:user_id/role` — Update member role (admin/owner only)
    async fn handle_update_role<C: GroupsCtx + MiddlewareCtx>(
        State(resources): State<Arc<C>>,
        auth: AuthenticatedUser,
        Path((group_id, target_user_id)): Path<(String, String)>,
        Json(body): Json<UpdateRoleBody>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        // A promotion outlives the support session that made it: the new
        // admin mints invites and manages members afterwards. A demotion
        // narrows who manages the group, so it is still served.
        if body.role != GroupRole::Member {
            auth.refuse_impersonation(
                resources.repos().admin_audit.as_ref(),
                "promote a group member",
            )
            .await?;
        }

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
    async fn handle_update_peer_consent<C: GroupsCtx + MiddlewareCtx>(
        State(resources): State<Arc<C>>,
        auth: AuthenticatedUser,
        Path(group_id): Path<String>,
        Json(body): Json<UpdatePeerConsentBody>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        // A consent is the athlete's own; a support session withdraws, never grants.
        if body.consent {
            auth.refuse_impersonation(resources.repos().admin_audit.as_ref(), "grant peer sharing")
                .await?;
        }

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

    /// GET `/api/groups/:group_id/members/me/coach-sharing` — whether the
    /// group's human coach reads the caller's training.
    ///
    /// The caller's own choice only: the member list peers read carries no
    /// member's coach sharing. It also says whether the caller is due the
    /// one-time notice of the coach's AI tools (EFVP decision 9.2b), which
    /// either answer through the PUT, from Dravr's own app, clears.
    async fn handle_get_coach_sharing<C: GroupsCtx + MiddlewareCtx>(
        State(resources): State<Arc<C>>,
        auth: AuthenticatedUser,
        Path(group_id): Path<String>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let member = Self::require_member(&resources, &group_id, auth.user_id).await?;
        let response = Self::coach_sharing_state(&resources, &auth, &group_id, member).await?;
        Ok((StatusCode::OK, Json(response)).into_response())
    }

    /// The caller's coach sharing as their membership `member` holds it, and
    /// whether the apps tell them once that the coach's AI tools read them.
    async fn coach_sharing_state<C: GroupsCtx + MiddlewareCtx>(
        resources: &Arc<C>,
        auth: &AuthResult,
        group_id: &str,
        member: GroupMember,
    ) -> Result<CoachSharingResponse, AppError> {
        let group = resources
            .group_service()
            .get_group(group_id, Self::get_tenant_id(auth)?)
            .await?
            .ok_or_else(|| AppError::not_found("You are not a member of this group"))?;
        let answered_at = resources
            .repos()
            .groups
            .assistant_notice_answered_at(group_id, auth.user_id)
            .await?;
        Ok(CoachSharingResponse {
            consent: member.coach_sharing_consent,
            assistant_notice_due: coach_consents::assistant_notice_due(
                &group,
                &member,
                answered_at,
            ),
        })
    }

    /// PUT `/api/groups/:group_id/members/me/coach-sharing` — the caller
    /// stops or resumes sharing their training with the group's human coach,
    /// the switch beside peer sharing in Group info (carnet#1168). Joining
    /// grants it (ADR-002); `/group consent coach yes|no` is the same write.
    /// Served in Dravr's own app it also answers the notice of the coach's
    /// AI tools, "Keep sharing" or "Stop sharing"; over an API key it does
    /// not.
    async fn handle_update_coach_sharing<C: GroupsCtx + MiddlewareCtx>(
        State(resources): State<Arc<C>>,
        auth: AuthenticatedUser,
        Path(group_id): Path<String>,
        Json(body): Json<CoachSharingBody>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        // A consent is the athlete's own; a support session withdraws, never grants.
        if body.consent {
            auth.refuse_impersonation(
                resources.repos().admin_audit.as_ref(),
                "grant coach sharing",
            )
            .await?;
        }
        Self::require_member(&resources, &group_id, auth.user_id).await?;

        let updated = resources
            .repos()
            .groups
            .update_coach_sharing_consent(&group_id, auth.user_id, body.consent)
            .await?;
        if !updated {
            return Err(AppError::not_found("Membership not found"));
        }
        // In Dravr's own app this write is the member's answer to the notice
        // of the coach's AI tools, whichever way they chose; from an API key
        // the change stands and the notice stays due.
        let answered_notice = coach_consents::served_in_app();
        if answered_notice {
            resources
                .repos()
                .groups
                .answer_assistant_notice(&group_id, auth.user_id, Timestamp::now())
                .await?;
        }
        info!(
            user_id = %auth.user_id,
            group_id = %group_id,
            consent = body.consent,
            answered_notice,
            "Member set coach sharing from Group info"
        );

        let member = Self::require_member(&resources, &group_id, auth.user_id).await?;
        let response = Self::coach_sharing_state(&resources, &auth, &group_id, member).await?;
        Ok((StatusCode::OK, Json(response)).into_response())
    }

    // ========================================================================
    // Invite handlers
    // ========================================================================

    /// POST `/api/groups/:group_id/invites` — Create an invite code (admin/owner only)
    async fn handle_create_invite<C: GroupsCtx + MiddlewareCtx>(
        State(resources): State<Arc<C>>,
        auth: AuthenticatedUser,
        Path(group_id): Path<String>,
        Json(body): Json<CreateInviteBody>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        // An invite code admits whoever holds it after the support session ends.
        auth.refuse_impersonation(
            resources.repos().admin_audit.as_ref(),
            "create a group invite",
        )
        .await?;
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

        let created = if body.approves_signup {
            Self::require_roster_manager(&resources, auth.user_id).await?;
            if body.kind != GroupInviteKind::Member {
                return Err(AppError::invalid_input(
                    "A signup code is a member invite; a coach invite cannot approve accounts",
                ));
            }
            resources
                .group_service()
                .create_signup_invite(
                    group_uuid,
                    auth.user_id,
                    tenant_id,
                    body.expires_in_days,
                    body.max_uses,
                )
                .await?
        } else {
            resources
                .group_service()
                .create_invite(
                    group_uuid,
                    auth.user_id,
                    tenant_id,
                    body.expires_in_days,
                    body.max_uses,
                    body.kind,
                )
                .await?
        };

        let response: InviteResponse = created.into();
        Ok((StatusCode::CREATED, Json(response)).into_response())
    }

    /// GET `/api/groups/:group_id/invites` — List invites for a group (admin/owner only)
    async fn handle_list_invites<C: GroupsCtx + MiddlewareCtx>(
        State(resources): State<Arc<C>>,
        auth: AuthenticatedUser,
        Path(group_id): Path<String>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();

        // Verify admin/owner role
        Self::require_admin(&resources, &group_id, auth.user_id).await?;

        let invites = resources.group_service().list_invites(&group_id).await?;

        // One aggregate read for the whole list, under the invites' own
        // tenant (every invite of a group carries its group's).
        let funnels = match invites.first() {
            Some(first) => {
                group_invite_card::invite_funnels(
                    resources.repos(),
                    first.group_id,
                    &first.tenant_id,
                    Timestamp::now(),
                )
                .await?
            }
            None => HashMap::new(),
        };
        let invite_responses: Vec<InviteWithFunnelResponse> = invites
            .into_iter()
            .map(|invite| InviteWithFunnelResponse {
                funnel: funnels.get(&invite.id).copied().unwrap_or_default(),
                invite: InviteResponse::from(invite),
            })
            .collect();

        let response = ListInvitesResponse {
            total: invite_responses.len(),
            invites: invite_responses,
            metadata: Self::build_metadata(),
        };

        Ok((StatusCode::OK, Json(response)).into_response())
    }

    /// DELETE `/api/groups/:group_id/invites/:invite_id` — Deactivate an invite (admin/owner only)
    async fn handle_deactivate_invite<C: GroupsCtx + MiddlewareCtx>(
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

    /// POST `/api/groups/:group_id/leave` — Leave a group
    async fn handle_leave_group<C: GroupsCtx + MiddlewareCtx>(
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

    /// PUT `/api/groups/:group_id/coach` — The caller, the group's owner
    /// holding coach access, becomes the coach of their coachless group
    /// (carnet#1650). 403 for a caller who is not the owner or lacks coach
    /// access, 409 for a group that has a coach, 404 for an unknown group,
    /// another tenant's or an archived one.
    async fn handle_claim_coach<C: GroupsCtx + MiddlewareCtx>(
        State(resources): State<Arc<C>>,
        auth: AuthenticatedUser,
        Path(group_id): Path<String>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let tenant_id = Self::get_tenant_id(&auth)?;
        // Only the room line is wanted: the owner asked for this themselves,
        // so no notice or mail goes out.
        let notices = CoachAccessNotices {
            notifications: None,
            email: None,
            strings: Arc::clone(resources.messaging_strings_registry()),
            frontend_url: None,
        };
        let group = claim_group_coach(
            resources.repos(),
            &notices,
            auth.user_id,
            &group_id,
            tenant_id,
        )
        .await?;
        let group = Self::group_response(&resources, group, auth.user_id).await?;
        Ok((StatusCode::OK, Json(ClaimGroupCoachResponse { group })).into_response())
    }

    /// DELETE `/api/groups/:group_id/coach` — Detach the human coach (admin/owner only)
    async fn handle_remove_coach<C: GroupsCtx + MiddlewareCtx>(
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
            .set_group_coach(&group_id, None, tenant_id, auth.user_id)
            .await?;

        if !cleared {
            return Err(AppError::not_found(format!("Group {group_id}")));
        }

        Ok((StatusCode::NO_CONTENT, ()).into_response())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invite_response_debug_hides_code() {
        let sentinel = "sentinel-invite-2e8a";
        let response = InviteResponse {
            id: "inv-1".to_owned(),
            group_id: "grp-1".to_owned(),
            code: sentinel.to_owned(),
            kind: "member".to_owned(),
            created_by: "user-1".to_owned(),
            expires_at: None,
            max_uses: None,
            use_count: 0,
            is_active: true,
            approves_signup: false,
            created_at: "2026-01-01T00:00:00Z".to_owned(),
        };
        let printed = format!("{response:?}");
        assert!(!printed.contains(sentinel), "{printed}");
        assert!(printed.contains("InviteResponse"), "{printed}");
    }
}
