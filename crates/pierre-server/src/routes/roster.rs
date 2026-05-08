// ABOUTME: Coach-athlete roster routes — gated by users.manages_roster=true or is_admin=true
// ABOUTME: GET/POST /api/roster lists/creates assignments; DELETE /api/roster/{athlete_id} revokes
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Roster routes
//!
//! - `GET /api/roster` → list active assignments where the caller is the
//!   coach (one row per assigned athlete in the same tenant).
//! - `POST /api/roster` body `{ "athlete_user_id": "<uuid>" }` → assign
//!   the named athlete to the caller's roster. Refuses if the athlete
//!   is in a different tenant or the assignment is already active
//!   (idempotent).
//! - `DELETE /api/roster/{athlete_user_id}` → soft-revoke the active
//!   assignment between the caller and that athlete. Audit history is
//!   preserved (`revoked_at`, `revoked_by` fill in).
//!
//! ## Permission model
//!
//! All three handlers gate on `users.manages_roster=true` OR
//! `users.is_admin=true`. A user with `coaching_persona=Coach` but
//! `manages_roster=false` is rejected — those are independent fields
//! (Coach voice ≠ Coach permission). See
//! `Coaching Persona Architecture.md` §8 for the rationale.
//!
//! ## Tenant isolation
//!
//! Both the caller's `active_tenant_id` and the athlete's `tenant_id`
//! must match. Cross-tenant assignments are refused at the route layer,
//! never written. The `coach_athlete_assignments` table itself carries
//! a `tenant_id` column on every row so all reads stay tenant-scoped at
//! the SQL layer too.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use pierre_core::errors::ErrorCode;
use pierre_core::models::{CoachAthleteAssignment, TenantId, User};
use pierre_database::repositories::UserRepository;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::info;
use uuid::Uuid;

use crate::errors::{AppError, AppResult};
use crate::mcp::resources::ServerContext;
use crate::middleware::extractors::extract_auth_from_headers;

/// Mount roster routes under `/api/roster`.
pub fn router(state: Arc<ServerContext>) -> Router {
    Router::new()
        .route("/api/roster", get(list_athletes).post(assign_athlete))
        .route("/api/roster/{athlete_user_id}", delete(revoke_assignment))
        .with_state(state)
}

/// JSON body for `POST /api/roster`.
#[derive(Debug, Deserialize)]
pub struct AssignAthleteRequest {
    /// `UUID` of the user to add to the caller's roster. Must belong
    /// to the caller's active tenant.
    pub athlete_user_id: Uuid,
}

/// JSON shape for assignment listing — flattens the model with
/// `snake_case` ISO 8601 timestamps for SDK consumption.
#[derive(Debug, Serialize)]
pub struct AssignmentResponse {
    /// Stable assignment row identifier.
    pub id: Uuid,
    /// User who is coaching.
    pub coach_user_id: Uuid,
    /// User being coached.
    pub athlete_user_id: Uuid,
    /// Tenant scope (matches both users' active tenant).
    pub tenant_id: Uuid,
    /// Who created the assignment, if known.
    pub assigned_by: Option<Uuid>,
    /// When the assignment was created.
    pub assigned_at: DateTime<Utc>,
    /// When the assignment was revoked, or `None` while active.
    pub revoked_at: Option<DateTime<Utc>>,
    /// Who revoked the assignment, or `None` while active.
    pub revoked_by: Option<Uuid>,
}

impl From<CoachAthleteAssignment> for AssignmentResponse {
    fn from(a: CoachAthleteAssignment) -> Self {
        Self {
            id: a.id,
            coach_user_id: a.coach_user_id,
            athlete_user_id: a.athlete_user_id,
            tenant_id: a.tenant_id.as_uuid(),
            assigned_by: a.assigned_by,
            assigned_at: a.assigned_at,
            revoked_at: a.revoked_at,
            revoked_by: a.revoked_by,
        }
    }
}

/// Resolve the caller, validate they may manage rosters, and return
/// `(caller, tenant_id)`. Three failure modes:
///
/// - missing `Authorization` / cookie → 401 from `extract_auth_from_headers`
/// - user lookup fails or row is missing → 401 (the JWT is stale)
/// - `manages_roster=false` AND `is_admin=false` → 403
async fn require_roster_manager(
    headers: &HeaderMap,
    resources: &Arc<ServerContext>,
) -> AppResult<(User, TenantId)> {
    let auth = extract_auth_from_headers(headers, resources).await?;
    let users: &dyn UserRepository = resources.repos.users.as_ref();
    let user = users
        .get_global(auth.user_id)
        .await?
        .ok_or_else(|| AppError::auth_invalid("Authenticated user no longer exists"))?;
    if !user.manages_roster && !user.is_admin {
        return Err(AppError::new(
            ErrorCode::PermissionDenied,
            "User does not have roster-management permission (manages_roster=false)",
        ));
    }
    let tenant_uuid = auth
        .active_tenant_id
        .ok_or_else(|| AppError::auth_invalid("No active tenant in session"))?;
    Ok((user, TenantId::from_uuid(tenant_uuid)))
}

/// `GET /api/roster` — list the caller's active assignments (athletes
/// they coach). Empty list when the caller has not been assigned any
/// athletes yet.
async fn list_athletes(
    State(resources): State<Arc<ServerContext>>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let (caller, tenant_id) = require_roster_manager(&headers, &resources).await?;
    let assignments = resources
        .repos
        .roster
        .list_athletes_for_coach(caller.id, tenant_id)
        .await?;
    let body: Vec<AssignmentResponse> = assignments.into_iter().map(Into::into).collect();
    Ok((StatusCode::OK, Json(json!({ "assignments": body }))).into_response())
}

/// `POST /api/roster` — assign an athlete in the same tenant. Refuses
/// if the athlete does not exist, lives in another tenant, or already
/// has an active assignment from this coach (idempotent).
async fn assign_athlete(
    State(resources): State<Arc<ServerContext>>,
    headers: HeaderMap,
    Json(body): Json<AssignAthleteRequest>,
) -> Result<Response, AppError> {
    let (caller, tenant_id) = require_roster_manager(&headers, &resources).await?;
    if body.athlete_user_id == caller.id {
        return Err(AppError::invalid_input(
            "Cannot assign yourself as your own athlete",
        ));
    }

    // Tenant isolation: athlete must belong to the caller's active tenant.
    let users: &dyn UserRepository = resources.repos.users.as_ref();
    let athlete = users
        .get_global(body.athlete_user_id)
        .await?
        .ok_or_else(|| AppError::not_found(format!("User {}", body.athlete_user_id)))?;
    let athlete_role = resources
        .repos
        .tenants
        .get_user_role(athlete.id, tenant_id)
        .await?;
    if athlete_role.is_none() {
        return Err(AppError::new(
            ErrorCode::PermissionDenied,
            "Athlete does not belong to the caller's active tenant",
        ));
    }

    let assignment =
        CoachAthleteAssignment::new(caller.id, body.athlete_user_id, tenant_id, Some(caller.id));
    let inserted = resources.repos.roster.assign_athlete(&assignment).await?;
    inserted.map_or_else(
        || {
            Err(AppError::already_exists(
                "Active roster assignment for this coach and athlete",
            ))
        },
        |row| {
            info!(
                coach_user_id = %row.coach_user_id,
                athlete_user_id = %row.athlete_user_id,
                tenant_id = %row.tenant_id,
                "Roster assignment created"
            );
            Ok((StatusCode::CREATED, Json(AssignmentResponse::from(row))).into_response())
        },
    )
}

/// `DELETE /api/roster/{athlete_user_id}` — soft-revoke the active
/// assignment. 404 when there is no active row to revoke.
async fn revoke_assignment(
    State(resources): State<Arc<ServerContext>>,
    headers: HeaderMap,
    Path(athlete_user_id): Path<Uuid>,
) -> Result<Response, AppError> {
    let (caller, tenant_id) = require_roster_manager(&headers, &resources).await?;
    let revoked = resources
        .repos
        .roster
        .revoke_assignment(caller.id, athlete_user_id, tenant_id, Some(caller.id))
        .await?;
    if !revoked {
        return Err(AppError::not_found(
            "No active assignment for this coach and athlete",
        ));
    }
    info!(
        coach_user_id = %caller.id,
        athlete_user_id = %athlete_user_id,
        tenant_id = %tenant_id,
        "Roster assignment revoked"
    );
    Ok((
        StatusCode::OK,
        Json(json!({ "message": "Assignment revoked" })),
    )
        .into_response())
}
