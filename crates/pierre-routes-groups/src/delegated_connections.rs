// ABOUTME: Delegated-connection and coach-consent routes: a group's coach links platform roster athletes to members, who confirm
// ABOUTME: List, roster, propose, confirm and end a link; a member reads and sets what their coach may do, under /api/groups/{group_id}
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Delegated-connection routes.
//!
//! A group's human coach reads each linked member's workouts on a coaching
//! platform (`TrainingPeaks`, Intervals.icu) through the coach's own account
//! there. The coach lists their roster and proposes which athlete is which
//! member; the member confirms, which is their consent to the read; either
//! side ends a link. What the member lets their coach do beyond the read (the
//! coach writing their calendar, their wellness syncing through the coach's
//! account) lives on their coach relationship, not on a link: the member alone
//! grants it, as they confirm a link or at any time on their membership
//! (`/members/me/coach-consents`), and revokes it. The rules of each step
//! live in [`pierre_services::delegated_connections`] and
//! [`pierre_services::coach_consents`]; this module decides who may take it.
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
use pierre_core::errors::{AppError, ErrorCode};
use pierre_core::models::groups::{CoachConsents, CoachingGroup};
use pierre_core::models::{DelegatedConnection, DelegationStatus, TenantId, User};
use pierre_database::RepositoryRegistry;
use pierre_middleware::AuthenticatedUser;
use pierre_providers::backend_resolver::user_facing_name;
use pierre_providers::core::ActivityQueryParams;
use pierre_providers::registry::ProviderRegistry;
use pierre_runtime_context::{GroupsCtx, MiddlewareCtx};
use pierre_services::coach_consents::{
    consent_offer, link_consents, link_offer, link_reads_wellness, set_coach_consents,
    wellness_read_skipped, ConsentChange, ConsentOffer, WellnessReadSkip,
};
use pierre_services::coach_platform::{check_roster_transport, roster_platform};
use pierre_services::delegated_connections::{
    confirm, end, person_name, propose, roster_for_group, unbound_link_reason, DelegationServices,
    Proposal, RosterEntry,
};
use pierre_services::delegated_read::OwnLogins;
use pierre_services::trainingpeaks_accounts::link_binding;
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
    /// The provider as the athlete knows it (`trainingpeaks`,
    /// `intervals_icu`)
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
    /// Why a confirmed link reads nothing until the coach links the member
    /// again: `athlete_email_missing`, `athlete_email_mismatch` or
    /// `member_email_unverified`. `None` while it reads, and for a proposal.
    pub read_refused: Option<String>,
    /// Whether the member lets their coach write their training calendar,
    /// read from their coach relationship: the link carries no consent
    pub calendar_write: bool,
    /// Whether the link's platform takes a coach's calendar writes, so the
    /// member's `calendar_write` applies to it
    pub calendar_write_supported: bool,
    /// Whether the member lets their wellness sync through the coach's
    /// account, read from their coach relationship
    pub wellness_read: bool,
    /// Whether the coach's account reads wellness on the link's platform, so
    /// the member's `wellness_read` applies to it
    pub wellness_read_supported: bool,
    /// Why the link syncs none of the member's wellness: a source of their
    /// own already gives Dravr it (`own_connection`, `own_wellness_source`).
    /// The member reads it granted or not; the coach only once the member
    /// shared their wellness, since which sources a member connects is
    /// theirs until then. `None` when nothing of theirs keeps it from
    /// syncing, and where the platform offers no wellness.
    pub wellness_read_skipped: Option<String>,
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

/// One athlete on the coach's platform roster, with its link in this group
/// and the member its roster email binds
#[derive(Debug, Serialize, Deserialize)]
pub struct DelegationRosterAthlete {
    /// The athlete's id on the roster
    pub provider_athlete_id: String,
    /// The athlete's name on the roster, when it has one
    pub display_name: Option<String>,
    /// The live link that holds the athlete in this group
    pub connection: Option<DelegatedConnectionResponse>,
    /// The unlinked member of this group whose verified email is the
    /// athlete's roster email, when the platform gives the coach access to
    /// the athlete
    pub suggested_member_user_id: Option<String>,
}

/// Response for the coach's roster
#[derive(Debug, Serialize, Deserialize)]
pub struct DelegationRosterResponse {
    /// The coaching platform the roster is read from, as the athlete knows
    /// it (`trainingpeaks`, `intervals_icu`)
    pub provider: String,
    /// The athletes the roster lists
    pub athletes: Vec<DelegationRosterAthlete>,
    /// Response metadata
    pub metadata: GroupMetadata,
}

/// Request to link a roster athlete to a member
#[derive(Debug, Deserialize)]
pub struct ProposeDelegatedConnectionRequest {
    /// The coaching platform, as the athlete knows it (`trainingpeaks`,
    /// `intervals_icu`)
    pub provider: String,
    /// The athlete's id on the coach's roster
    pub provider_athlete_id: String,
    /// The live member the athlete is
    pub member_user_id: Uuid,
}

/// What the member grants as they confirm a link. Each consent is off unless
/// named, so a confirm with no body grants nothing; one the platform does not
/// offer is ignored
#[derive(Debug, Deserialize, Default)]
pub struct ConfirmDelegatedConnectionRequest {
    /// Let the link's coach write the member's training calendar
    #[serde(default)]
    pub calendar_write: bool,
    /// Let the member's wellness sync through the coach's account
    #[serde(default)]
    pub wellness_read: bool,
}

/// The member grants or revokes what their group's coach may do. A consent
/// left out stays as it stands; a grant the relationship does not offer is
/// ignored
#[derive(Debug, Deserialize, Default)]
pub struct UpdateCoachConsentsRequest {
    /// Grant (`true`) or revoke (`false`) the coach's calendar writes
    #[serde(default)]
    pub calendar_write: Option<bool>,
    /// Grant (`true`) or revoke (`false`) the wellness sync; a revoke deletes
    /// the wellness the member's links synced
    #[serde(default)]
    pub wellness_read: Option<bool>,
}

/// What a member lets their group's coach do beyond reading their training,
/// and what the relationship offers
#[derive(Debug, Serialize)]
pub struct CoachConsentsResponse {
    /// The group's coach writes the member's training calendar on a
    /// coaching platform
    pub calendar_write: bool,
    /// When the member last granted or revoked `calendar_write` (RFC 3339)
    pub calendar_write_at: Option<String>,
    /// Whether the group has a coach and a coaching platform takes a coach's
    /// calendar writes, so the member can be asked for them
    pub calendar_write_supported: bool,
    /// The member's wellness syncs through the coach's account
    pub wellness_read: bool,
    /// When the member last granted or revoked `wellness_read` (RFC 3339)
    pub wellness_read_at: Option<String>,
    /// Whether the member reads a platform through a confirmed link whose
    /// coach credential reads wellness, so they can be asked for it
    pub wellness_read_supported: bool,
    /// Why the member's links sync none of their wellness whatever they
    /// grant: a source of their own already gives Dravr it
    /// (`own_connection`, `own_wellness_source`); `None` when nothing does
    pub wellness_read_skipped: Option<String>,
}

/// Query for the roster read
#[derive(Debug, Deserialize, Default)]
pub struct RosterQuery {
    /// Read the roster live instead of from the ten-minute cache
    #[serde(default)]
    pub refresh: bool,
    /// The coaching platform to read (`trainingpeaks`, `intervals_icu`);
    /// absent, the one the coach connected
    #[serde(default)]
    pub provider: Option<String>,
}

// ============================================================================
// Routes
// ============================================================================

/// Delegated-connection route handler
#[derive(Debug)]
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
                "/api/groups/{group_id}/members/me/coach-consents",
                get(Self::handle_get_coach_consents::<C>).put(Self::handle_set_coach_consents::<C>),
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

    /// What the linking steps that read a coach's roster work with.
    fn services<C: ToolRuntime + GroupsCtx + MiddlewareCtx>(
        resources: &Arc<C>,
    ) -> DelegationServices<'_> {
        DelegationServices {
            repos: MiddlewareCtx::repos(resources.as_ref()),
            registry: ToolRuntime::provider_registry(resources.as_ref()),
            cache: ToolRuntime::cache(resources.as_ref()),
            notifications: GroupsCtx::notification_service(resources.as_ref()),
        }
    }

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
                "Only the group's coach can link coaching-platform athletes",
            )
        })
    }

    fn connection_id(raw: &str) -> Result<Uuid, AppError> {
        Uuid::parse_str(raw).map_err(|_| AppError::not_found("Delegated link"))
    }

    /// The link `connection_id` of `group`, when `caller` is the live member
    /// it names; `None` for anyone else, its coach included.
    async fn own_link(
        repos: &RepositoryRegistry,
        group: &CoachingGroup,
        connection_id: Uuid,
        caller: Uuid,
    ) -> Result<Option<DelegatedConnection>, AppError> {
        let link = repos
            .delegated_connections
            .get_for_participant(connection_id, group.id, caller)
            .await?;
        let is_member = repos
            .groups
            .get_member(&group.id.to_string(), caller)
            .await?
            .is_some();
        Ok(link.filter(|l| is_member && l.member_user_id == caller))
    }

    /// Each link as `viewer` sees it, naming its coach and its member, why a
    /// confirmed one reads nothing when its athlete is not its member by
    /// email, the member's coach consents and which of them its platform
    /// takes, and why its wellness syncs nothing when a source of the
    /// member's own wins: to the coach only once the member shared their
    /// wellness.
    async fn responses(
        repos: &RepositoryRegistry,
        registry: &ProviderRegistry,
        links: &[DelegatedConnection],
        viewer: DelegationViewer,
    ) -> Result<Vec<DelegatedConnectionResponse>, AppError> {
        let mut ids: Vec<Uuid> = links
            .iter()
            .flat_map(|l| [l.coach_user_id, l.member_user_id])
            .collect();
        ids.sort_unstable();
        ids.dedup();
        let users = repos.users.get_global_many(&ids).await?;
        let mut responses = Vec::with_capacity(links.len());
        for link in links {
            let read_refused = if link.status == DelegationStatus::Confirmed {
                unbound_link_reason(link_binding(repos, link).await?)
            } else {
                None
            };
            let consents = link_consents(repos.groups.as_ref(), link).await?;
            let offer = link_offer(registry, link);
            let shown = viewer == DelegationViewer::Member || consents.wellness_read;
            let skipped = if offer.wellness_read && shown {
                wellness_read_skipped(OwnLogins::of(repos), link).await?
            } else {
                None
            };
            responses.push(Self::response(
                link,
                &users,
                consents,
                offer,
                read_refused,
                skipped.map(WellnessReadSkip::as_str),
            )?);
        }
        Ok(responses)
    }

    fn response(
        link: &DelegatedConnection,
        users: &HashMap<Uuid, User>,
        consents: CoachConsents,
        offer: ConsentOffer,
        read_refused: Option<&str>,
        wellness_read_skipped: Option<&str>,
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
            read_refused: read_refused.map(str::to_owned),
            calendar_write: consents.calendar_write,
            calendar_write_supported: offer.calendar_write,
            wellness_read: consents.wellness_read,
            wellness_read_supported: offer.wellness_read,
            wellness_read_skipped: wellness_read_skipped.map(str::to_owned),
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

        let registry = ToolRuntime::provider_registry(resources.as_ref());
        let connections = Self::responses(repos, registry, &links, viewer).await?;
        let response = DelegatedConnectionsResponse {
            total: connections.len(),
            connections,
            viewer,
            metadata: Self::build_metadata(),
        };
        Ok((StatusCode::OK, Json(response)).into_response())
    }

    /// GET `/api/groups/:group_id/delegated-connections/roster` — the coach's
    /// coaching-platform roster, each athlete with its link in this group.
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
        let registry = ToolRuntime::provider_registry(resources.as_ref());
        let platform = roster_platform(
            repos,
            registry,
            auth.user_id,
            tenant,
            query.provider.as_deref(),
        )
        .await?;
        let platform = platform.as_ref();
        check_roster_transport(registry, platform)?;

        let entries = roster_for_group(
            Self::services(&resources),
            platform,
            &group,
            auth.user_id,
            tenant,
            query.refresh,
        )
        .await?;
        let links: Vec<DelegatedConnection> =
            entries.iter().filter_map(|e| e.link.clone()).collect();
        let mut linked: HashMap<String, DelegatedConnectionResponse> =
            Self::responses(repos, registry, &links, DelegationViewer::Coach)
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
                        display_name: athlete.name,
                        suggested_member_user_id: suggested_member_user_id.map(|id| id.to_string()),
                    }
                },
            )
            .collect();
        let response = DelegationRosterResponse {
            provider: platform.user_facing().to_owned(),
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
            Self::services(&resources),
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
        let registry = ToolRuntime::provider_registry(resources.as_ref());
        let response = Self::responses(repos, registry, &[link], DelegationViewer::Coach)
            .await?
            .pop()
            .ok_or_else(|| AppError::internal("Proposed link has no response"))?;
        Ok((StatusCode::CREATED, Json(response)).into_response())
    }

    /// POST `/api/groups/:group_id/delegated-connections/:connection_id/confirm`
    /// — the member a proposal names confirms it, granting with it the
    /// consents the body names; a confirm with no body grants none.
    async fn handle_confirm<C: ToolRuntime + GroupsCtx + MiddlewareCtx>(
        State(resources): State<Arc<C>>,
        auth: AuthenticatedUser,
        Path((group_id, connection_id)): Path<(String, String)>,
        body: Option<Json<ConfirmDelegatedConnectionRequest>>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let (group, tenant) = Self::open_group(&resources, &auth, &group_id).await?;
        let connection_id = Self::connection_id(&connection_id)?;
        let repos = MiddlewareCtx::repos(resources.as_ref());

        let Some(link) = Self::own_link(repos, &group, connection_id, auth.user_id).await? else {
            return Err(AppError::new(
                ErrorCode::PermissionDenied,
                "Only the member a link names can confirm it",
            ));
        };
        if link.status != DelegationStatus::Proposed {
            return Err(AppError::not_found("Pending link"));
        }

        let body = body.map(|Json(body)| body).unwrap_or_default();
        let requested = ConsentChange::grants(body.calendar_write, body.wellness_read);
        let confirmed =
            confirm(Self::services(&resources), &group, &link, tenant, requested).await?;
        Self::spawn_warm_up(
            &resources,
            confirmed.provider.clone(),
            confirmed.member_user_id,
            tenant,
        );
        let registry = ToolRuntime::provider_registry(resources.as_ref());
        let response = Self::responses(repos, registry, &[confirmed], DelegationViewer::Member)
            .await?
            .pop()
            .ok_or_else(|| AppError::internal("Confirmed link has no response"))?;
        Ok((StatusCode::OK, Json(response)).into_response())
    }

    /// The coach consents of `member` in `group`, as the member reads them.
    async fn coach_consents_response(
        repos: &RepositoryRegistry,
        registry: &ProviderRegistry,
        group: &CoachingGroup,
        consents: CoachConsents,
        member: Uuid,
    ) -> Result<CoachConsentsResponse, AppError> {
        let offer = consent_offer(repos, registry, group, member).await?;
        let mut skipped = None;
        if offer.wellness_read {
            let links = repos
                .delegated_connections
                .list_live_for_member_in_group(group.id, member)
                .await?;
            for link in links.iter().filter(|link| {
                link.status == DelegationStatus::Confirmed && link_reads_wellness(registry, link)
            }) {
                skipped = wellness_read_skipped(OwnLogins::of(repos), link).await?;
                if skipped.is_none() {
                    break;
                }
            }
        }
        Ok(CoachConsentsResponse {
            calendar_write: consents.calendar_write,
            calendar_write_at: consents.calendar_write_at.map(|at| at.to_rfc3339()),
            calendar_write_supported: offer.calendar_write,
            wellness_read: consents.wellness_read,
            wellness_read_at: consents.wellness_read_at.map(|at| at.to_rfc3339()),
            wellness_read_supported: offer.wellness_read,
            wellness_read_skipped: skipped.map(|skip| skip.as_str().to_owned()),
        })
    }

    /// GET `/api/groups/:group_id/members/me/coach-consents` — what the
    /// caller, a live member, lets the group's coach do beyond reading their
    /// training, and what the relationship offers.
    async fn handle_get_coach_consents<C: ToolRuntime + GroupsCtx + MiddlewareCtx>(
        State(resources): State<Arc<C>>,
        auth: AuthenticatedUser,
        Path(group_id): Path<String>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let (group, _) = Self::open_group(&resources, &auth, &group_id).await?;
        let repos = MiddlewareCtx::repos(resources.as_ref());
        let member = repos
            .groups
            .get_member(&group.id.to_string(), auth.user_id)
            .await?
            .ok_or_else(|| AppError::not_found("Group member"))?;
        let registry = ToolRuntime::provider_registry(resources.as_ref());
        let response = Self::coach_consents_response(
            repos,
            registry,
            &group,
            member.coach_consents,
            auth.user_id,
        )
        .await?;
        Ok((StatusCode::OK, Json(response)).into_response())
    }

    /// PUT `/api/groups/:group_id/members/me/coach-consents` — the caller, a
    /// live member, grants or revokes what the group's coach may do beyond
    /// reading their training. A grant is taken only from Dravr's own app; a
    /// revoke from anywhere.
    async fn handle_set_coach_consents<C: ToolRuntime + GroupsCtx + MiddlewareCtx>(
        State(resources): State<Arc<C>>,
        auth: AuthenticatedUser,
        Path(group_id): Path<String>,
        Json(body): Json<UpdateCoachConsentsRequest>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let (group, _) = Self::open_group(&resources, &auth, &group_id).await?;
        if group.coach_user_id == Some(auth.user_id) {
            return Err(AppError::new(
                ErrorCode::PermissionDenied,
                "Only a member decides what their coach may do",
            ));
        }
        let repos = MiddlewareCtx::repos(resources.as_ref());
        let registry = ToolRuntime::provider_registry(resources.as_ref());
        let updated = set_coach_consents(
            repos,
            registry,
            &group,
            auth.user_id,
            ConsentChange {
                calendar_write: body.calendar_write,
                wellness_read: body.wellness_read,
            },
        )
        .await?;
        let response = Self::coach_consents_response(
            repos,
            registry,
            &group,
            updated.coach_consents,
            auth.user_id,
        )
        .await?;
        Ok((StatusCode::OK, Json(response)).into_response())
    }

    /// Read the member's recent `backend` workouts through the link they just
    /// confirmed, off the request, so their first question finds them
    /// cached. The read writes through under the member's own keys; its
    /// failure is logged, and the next read tries again.
    fn spawn_warm_up<C: ToolRuntime>(
        resources: &Arc<C>,
        backend: String,
        member: Uuid,
        tenant: TenantId,
    ) {
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
            match fetch_provider_head(&runtime, &backend, member, &tenant, &params).await {
                Ok(activities) => info!(
                    user_id = %member,
                    provider = %backend,
                    count = activities.len(),
                    "Confirmed delegated link warmed up"
                ),
                Err(e) => warn!(
                    user_id = %member,
                    provider = %backend,
                    error = %e,
                    "Warm-up read through a confirmed delegated link failed"
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
