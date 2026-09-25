// ABOUTME: TrainingPeaks delegated-connection routes: a group's coach links roster athletes to members, who confirm
// ABOUTME: List, roster, propose, confirm and end under /api/groups/{group_id}/delegated-connections
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Delegated-connection routes.
//!
//! A group's human coach reads each linked member's `TrainingPeaks` workouts
//! through the coach's own `TrainingPeaks` account. The coach lists their
//! roster and proposes which athlete is which member; the member confirms,
//! which is their consent to the read; either side ends a link. The rules of
//! each step live in [`pierre_services::delegated_connections`]; this module
//! decides who may take it.
//!
//! Every handler reads the group without regard to tenant — groups span
//! tenants — and refuses an archived one. Who may act is a user key: the
//! group's `coach_user_id`, a live membership, or the participants a link
//! names.

use std::collections::HashMap;
use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Json, Router,
};
use chrono::Utc;
use pierre_auth::auth::AuthResult;
use pierre_core::constants::oauth_providers::{SCIOTTE_TRAININGPEAKS, TRAININGPEAKS};
use pierre_core::errors::{AppError, ErrorCode};
use pierre_core::models::groups::CoachingGroup;
use pierre_core::models::{DelegatedConnection, DelegationStatus, TenantId, User};
use pierre_database::RepositoryRegistry;
use pierre_middleware::AuthenticatedUser;
use pierre_providers::backend_resolver::user_facing_name;
use pierre_providers::core::ActivityQueryParams;
use pierre_runtime_context::{GroupsCtx, MiddlewareCtx};
use pierre_services::delegated_connections::{
    confirm, end, person_name, propose, roster_for_group, Proposal, RosterEntry,
};
use pierre_tool_runtime::activity_fetch::fetch_provider_head;
use pierre_tool_runtime::runtime::ToolRuntime;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};
use uuid::Uuid;

use crate::groups::GroupMetadata;

/// How many of the member's recent workouts the read after a confirm warms.
const WARM_UP_ACTIVITIES: usize = 30;

// ============================================================================
// Response and request types
// ============================================================================

/// One live delegated connection, as the group info shows it to its coach
/// and to the member it names.
#[derive(Debug, Serialize, Deserialize)]
pub struct DelegatedConnectionResponse {
    /// Link id
    pub id: String,
    /// The group the link belongs to
    pub group_id: String,
    /// The provider as the athlete knows it (`trainingpeaks`)
    pub provider: String,
    /// The group's coach, whose account serves the reads
    pub coach_user_id: String,
    /// The coach's name: their display name, else their email
    pub coach_display_name: String,
    /// The member whose workouts are read
    pub member_user_id: String,
    /// The member as the group's member list names them
    pub member_display_name: String,
    /// The athlete's id on the coach's roster
    pub provider_athlete_id: String,
    /// The athlete's name on the coach's roster, when it has one
    pub provider_athlete_name: Option<String>,
    /// `proposed` or `confirmed`
    pub status: DelegationStatus,
    /// When the coach proposed it (RFC 3339)
    pub proposed_at: String,
    /// When the member confirmed it (RFC 3339)
    pub confirmed_at: Option<String>,
}

/// Which side of the group's links the caller sees.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DelegationViewer {
    /// The group's coach: every live link in the group.
    Coach,
    /// A member: only the links naming them.
    Member,
}

/// Response for listing a group's live links
#[derive(Debug, Serialize, Deserialize)]
pub struct DelegatedConnectionsResponse {
    /// The live links the caller may see
    pub connections: Vec<DelegatedConnectionResponse>,
    /// Total count
    pub total: usize,
    /// Which side the caller is on
    pub viewer: DelegationViewer,
    /// Response metadata
    pub metadata: GroupMetadata,
}

/// One athlete on the coach's `TrainingPeaks` roster, with its link in this
/// group and the member its name suggests
#[derive(Debug, Serialize, Deserialize)]
pub struct DelegationRosterAthlete {
    /// The athlete's id on the roster
    pub provider_athlete_id: String,
    /// The athlete's name on the roster, when it has one
    pub display_name: Option<String>,
    /// The live link that holds the athlete in this group
    pub connection: Option<DelegatedConnectionResponse>,
    /// The unlinked member whose name reads as the athlete's, when exactly
    /// one does
    pub suggested_member_user_id: Option<String>,
}

/// Response for the coach's roster
#[derive(Debug, Serialize, Deserialize)]
pub struct DelegationRosterResponse {
    /// Always `trainingpeaks`
    pub provider: String,
    /// The athletes the roster lists
    pub athletes: Vec<DelegationRosterAthlete>,
    /// Response metadata
    pub metadata: GroupMetadata,
}

/// Request to link a roster athlete to a member
#[derive(Debug, Deserialize)]
pub struct ProposeDelegatedConnectionRequest {
    /// The provider, `trainingpeaks`
    pub provider: String,
    /// The athlete's id on the coach's roster
    pub provider_athlete_id: String,
    /// The live member the athlete is
    pub member_user_id: Uuid,
}

/// Query for the roster read
#[derive(Debug, Deserialize, Default)]
pub struct RosterQuery {
    /// Read the roster live instead of from the ten-minute cache
    #[serde(default)]
    pub refresh: bool,
}

// ============================================================================
// Routes
// ============================================================================

/// Delegated-connection route handler
pub struct DelegatedConnectionRoutes;

impl DelegatedConnectionRoutes {
    /// Create the delegated-connection routes, mounted beside
    /// [`crate::groups::GroupRoutes::routes`] under `/api/groups`.
    pub fn routes<C: ToolRuntime + GroupsCtx + MiddlewareCtx>(resources: Arc<C>) -> Router {
        Router::new()
            .route(
                "/api/groups/{group_id}/delegated-connections",
                get(Self::handle_list::<C>).post(Self::handle_propose::<C>),
            )
            .route(
                "/api/groups/{group_id}/delegated-connections/roster",
                get(Self::handle_roster::<C>),
            )
            .route(
                "/api/groups/{group_id}/delegated-connections/{connection_id}/confirm",
                post(Self::handle_confirm::<C>),
            )
            .route(
                "/api/groups/{group_id}/delegated-connections/{connection_id}",
                delete(Self::handle_end::<C>),
            )
            .with_state(resources)
    }

    // ========================================================================
    // Helpers
    // ========================================================================

    fn build_metadata() -> GroupMetadata {
        GroupMetadata {
            timestamp: Utc::now().to_rfc3339(),
            api_version: "1.0".to_owned(),
        }
    }

    /// The group, active, and the caller's tenant.
    async fn open_group<C: GroupsCtx>(
        resources: &Arc<C>,
        auth: &AuthResult,
        group_id: &str,
    ) -> Result<(CoachingGroup, TenantId), AppError> {
        let tenant = auth
            .active_tenant_id
            .map(TenantId::from_uuid)
            .ok_or_else(|| AppError::auth_invalid("No active tenant in session"))?;
        let group = resources
            .group_service()
            .get_group(group_id, tenant)
            .await?
            .ok_or_else(|| AppError::not_found("Group"))?;
        if !group.is_active {
            return Err(AppError::not_found("This group is no longer available"));
        }
        Ok((group, tenant))
    }

    /// Refuse anyone but the group's coach.
    fn require_coach(group: &CoachingGroup, caller: Uuid) -> Result<(), AppError> {
        (group.coach_user_id == Some(caller)).ok_or_else(|| {
            AppError::new(
                ErrorCode::PermissionDenied,
                "Only the group's coach can link TrainingPeaks athletes",
            )
        })
    }

    fn connection_id(raw: &str) -> Result<Uuid, AppError> {
        Uuid::parse_str(raw).map_err(|_| AppError::not_found("TrainingPeaks link"))
    }

    /// Each link as the clients show it, naming its coach and its member.
    async fn responses(
        repos: &RepositoryRegistry,
        links: &[DelegatedConnection],
    ) -> Result<Vec<DelegatedConnectionResponse>, AppError> {
        let mut ids: Vec<Uuid> = links
            .iter()
            .flat_map(|l| [l.coach_user_id, l.member_user_id])
            .collect();
        ids.sort_unstable();
        ids.dedup();
        let users = repos.users.get_global_many(&ids).await?;
        links
            .iter()
            .map(|link| Self::response(link, &users))
            .collect()
    }

    fn response(
        link: &DelegatedConnection,
        users: &HashMap<Uuid, User>,
    ) -> Result<DelegatedConnectionResponse, AppError> {
        let (Some(coach), Some(member)) = (
            users.get(&link.coach_user_id),
            users.get(&link.member_user_id),
        ) else {
            return Err(AppError::internal(format!(
                "Delegated connection {} names a user that does not exist",
                link.id
            )));
        };
        Ok(DelegatedConnectionResponse {
            id: link.id.to_string(),
            group_id: link.group_id.to_string(),
            provider: user_facing_name(&link.provider).to_owned(),
            coach_user_id: link.coach_user_id.to_string(),
            coach_display_name: person_name(coach),
            member_user_id: link.member_user_id.to_string(),
            member_display_name: member.email.clone(),
            provider_athlete_id: link.provider_athlete_id.clone(),
            provider_athlete_name: link.provider_athlete_name.clone(),
            status: link.status,
            proposed_at: link.proposed_at.to_rfc3339(),
            confirmed_at: link.confirmed_at.map(|at| at.to_rfc3339()),
        })
    }

    // ========================================================================
    // Handlers
    // ========================================================================

    /// GET `/api/groups/:group_id/delegated-connections` — the group's live
    /// links: all of them for its coach, a member's own for a member.
    async fn handle_list<C: ToolRuntime + GroupsCtx + MiddlewareCtx>(
        State(resources): State<Arc<C>>,
        auth: AuthenticatedUser,
        Path(group_id): Path<String>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let (group, _) = Self::open_group(&resources, &auth, &group_id).await?;
        let repos = MiddlewareCtx::repos(resources.as_ref());

        let (viewer, links) = if group.coach_user_id == Some(auth.user_id) {
            (
                DelegationViewer::Coach,
                repos
                    .delegated_connections
                    .list_live_for_coach_in_group(group.id, auth.user_id)
                    .await?,
            )
        } else if repos
            .groups
            .get_member(&group_id, auth.user_id)
            .await?
            .is_some()
        {
            (
                DelegationViewer::Member,
                repos
                    .delegated_connections
                    .list_live_for_member_in_group(group.id, auth.user_id)
                    .await?,
            )
        } else {
            return Err(AppError::not_found("You are not a member of this group"));
        };

        let connections = Self::responses(repos, &links).await?;
        let response = DelegatedConnectionsResponse {
            total: connections.len(),
            connections,
            viewer,
            metadata: Self::build_metadata(),
        };
        Ok((StatusCode::OK, Json(response)).into_response())
    }

    /// GET `/api/groups/:group_id/delegated-connections/roster` — the coach's
    /// `TrainingPeaks` roster, each athlete with its link in this group.
    async fn handle_roster<C: ToolRuntime + GroupsCtx + MiddlewareCtx>(
        State(resources): State<Arc<C>>,
        auth: AuthenticatedUser,
        Path(group_id): Path<String>,
        Query(query): Query<RosterQuery>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let (group, tenant) = Self::open_group(&resources, &auth, &group_id).await?;
        Self::require_coach(&group, auth.user_id)?;
        let repos = MiddlewareCtx::repos(resources.as_ref());

        let entries = roster_for_group(
            repos,
            ToolRuntime::cache(resources.as_ref()),
            GroupsCtx::notification_service(resources.as_ref()),
            &group,
            auth.user_id,
            tenant,
            query.refresh,
        )
        .await?;
        let links: Vec<DelegatedConnection> =
            entries.iter().filter_map(|e| e.link.clone()).collect();
        let mut linked: HashMap<String, DelegatedConnectionResponse> =
            Self::responses(repos, &links)
                .await?
                .into_iter()
                .map(|r| (r.provider_athlete_id.clone(), r))
                .collect();

        let athletes = entries
            .into_iter()
            .map(
                |RosterEntry {
                     athlete,
                     link,
                     suggested_member_user_id,
                 }| {
                    DelegationRosterAthlete {
                        connection: link.and_then(|_| linked.remove(&athlete.id)),
                        provider_athlete_id: athlete.id,
                        display_name: athlete.display_name,
                        suggested_member_user_id: suggested_member_user_id.map(|id| id.to_string()),
                    }
                },
            )
            .collect();
        let response = DelegationRosterResponse {
            provider: TRAININGPEAKS.to_owned(),
            athletes,
            metadata: Self::build_metadata(),
        };
        Ok((StatusCode::OK, Json(response)).into_response())
    }

    /// POST `/api/groups/:group_id/delegated-connections` — the coach links a
    /// roster athlete to a live member; the member is asked to confirm.
    async fn handle_propose<C: ToolRuntime + GroupsCtx + MiddlewareCtx>(
        State(resources): State<Arc<C>>,
        auth: AuthenticatedUser,
        Path(group_id): Path<String>,
        Json(body): Json<ProposeDelegatedConnectionRequest>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let (group, tenant) = Self::open_group(&resources, &auth, &group_id).await?;
        Self::require_coach(&group, auth.user_id)?;
        let repos = MiddlewareCtx::repos(resources.as_ref());

        let link = propose(
            repos,
            ToolRuntime::cache(resources.as_ref()),
            GroupsCtx::notification_service(resources.as_ref()),
            &group,
            auth.user_id,
            tenant,
            Proposal {
                provider: &body.provider,
                provider_athlete_id: &body.provider_athlete_id,
                member_user_id: body.member_user_id,
            },
        )
        .await?;
        let response = Self::responses(repos, &[link])
            .await?
            .pop()
            .ok_or_else(|| AppError::internal("Proposed link has no response"))?;
        Ok((StatusCode::CREATED, Json(response)).into_response())
    }

    /// POST `/api/groups/:group_id/delegated-connections/:connection_id/confirm`
    /// — the member a proposal names confirms it.
    async fn handle_confirm<C: ToolRuntime + GroupsCtx + MiddlewareCtx>(
        State(resources): State<Arc<C>>,
        auth: AuthenticatedUser,
        Path((group_id, connection_id)): Path<(String, String)>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let (group, tenant) = Self::open_group(&resources, &auth, &group_id).await?;
        let connection_id = Self::connection_id(&connection_id)?;
        let repos = MiddlewareCtx::repos(resources.as_ref());

        let link = repos
            .delegated_connections
            .get_for_participant(connection_id, group.id, auth.user_id)
            .await?;
        let is_member = repos
            .groups
            .get_member(&group_id, auth.user_id)
            .await?
            .is_some();
        let Some(link) = link.filter(|l| is_member && l.member_user_id == auth.user_id) else {
            return Err(AppError::new(
                ErrorCode::PermissionDenied,
                "Only the member a link names can confirm it",
            ));
        };
        if link.status != DelegationStatus::Proposed {
            return Err(AppError::not_found("Pending TrainingPeaks link"));
        }

        let confirmed = confirm(
            repos,
            GroupsCtx::notification_service(resources.as_ref()),
            &group,
            &link,
            tenant,
        )
        .await?;
        Self::spawn_warm_up(&resources, confirmed.member_user_id, tenant);
        let response = Self::responses(repos, &[confirmed])
            .await?
            .pop()
            .ok_or_else(|| AppError::internal("Confirmed link has no response"))?;
        Ok((StatusCode::OK, Json(response)).into_response())
    }

    /// Read the member's recent `TrainingPeaks` workouts through the link they
    /// just confirmed, off the request, so their first question finds them
    /// cached. The read writes through under the member's own keys; its
    /// failure is logged, and the next read tries again.
    fn spawn_warm_up<C: ToolRuntime>(resources: &Arc<C>, member: Uuid, tenant: TenantId) {
        let cloned: Arc<C> = Arc::clone(resources);
        let runtime: Arc<dyn ToolRuntime> = cloned;
        tokio::spawn(async move {
            let params = ActivityQueryParams {
                limit: Some(WARM_UP_ACTIVITIES),
                offset: None,
                before: None,
                after: None,
            };
            let tenant = tenant.to_string();
            match fetch_provider_head(&runtime, SCIOTTE_TRAININGPEAKS, member, &tenant, &params)
                .await
            {
                Ok(activities) => info!(
                    user_id = %member,
                    count = activities.len(),
                    "Confirmed TrainingPeaks link warmed up"
                ),
                Err(e) => warn!(
                    user_id = %member,
                    error = %e,
                    "Warm-up read through a confirmed TrainingPeaks link failed"
                ),
            }
        });
    }

    /// DELETE `/api/groups/:group_id/delegated-connections/:connection_id` —
    /// the coach or the linked member ends a live link.
    async fn handle_end<C: ToolRuntime + GroupsCtx + MiddlewareCtx>(
        State(resources): State<Arc<C>>,
        auth: AuthenticatedUser,
        Path((group_id, connection_id)): Path<(String, String)>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let (group, _) = Self::open_group(&resources, &auth, &group_id).await?;
        let connection_id = Self::connection_id(&connection_id)?;
        let repos = MiddlewareCtx::repos(resources.as_ref());

        let Some(link) = repos
            .delegated_connections
            .get_for_participant(connection_id, group.id, auth.user_id)
            .await?
        else {
            return Err(AppError::new(
                ErrorCode::PermissionDenied,
                "Only the group's coach or the linked member can end this link",
            ));
        };

        end(
            repos,
            GroupsCtx::notification_service(resources.as_ref()),
            &group,
            &link,
            auth.user_id,
        )
        .await?;
        Ok((StatusCode::NO_CONTENT, ()).into_response())
    }
}
