// ABOUTME: HTTP boundary for a user's connected MCP apps — list and revoke OAuth client grants
// ABOUTME: Backs the consent feature's revoke surface (web + mobile "Connected apps" settings)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Routes for managing OAuth client grants (connected MCP apps).
//!
//! - `GET /api/me/oauth-grants` — list the caller's active grants.
//! - `DELETE /api/me/oauth-grants/{id}` — revoke a grant (owner-scoped).
//!
//! A grant is recorded when the user approves an MCP client on the OAuth consent
//! screen; revoking one here means the next authorization for that client shows
//! the consent screen again.

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{delete, get},
    Json, Router,
};
use serde::Serialize;

use crate::mcp::resources::ServerContext;
use pierre_core::errors::AppError;
use pierre_middleware::extract_auth_from_headers;

/// Resolve the tenant a grant is keyed under: the caller's active tenant when the
/// session carries one, otherwise their first tenant (mirroring how the consent
/// flow keys the grant when the OAuth session has no explicit active tenant).
async fn resolve_tenant_id(
    resources: &Arc<ServerContext>,
    user_id: uuid::Uuid,
    active_tenant_id: Option<uuid::Uuid>,
) -> Result<String, AppError> {
    if let Some(tenant_id) = active_tenant_id {
        return Ok(tenant_id.to_string());
    }
    let tenants = resources
        .common
        .repos
        .tenants
        .list_for_user(user_id)
        .await?;
    tenants
        .first()
        .map(|tenant| tenant.id.to_string())
        .ok_or_else(|| AppError::auth_invalid("User has no tenant"))
}

/// One connected MCP OAuth client in the caller's "connected apps" list.
#[derive(Serialize)]
struct GrantView {
    /// Grant id (used to revoke).
    id: String,
    /// OAuth client the user approved.
    client_id: String,
    /// Scope the grant covers.
    scope: String,
    /// When the grant was made (RFC 3339).
    granted_at: String,
}

/// Response body for `GET /api/me/oauth-grants`.
#[derive(Serialize)]
struct GrantsResponse {
    /// The caller's active grants, most recent first.
    grants: Vec<GrantView>,
}

/// `GET /api/me/oauth-grants` — list the caller's active OAuth client grants.
async fn handle_list_grants(
    State(resources): State<Arc<ServerContext>>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let auth = extract_auth_from_headers(&headers, &resources).await?;
    let tenant_id = resolve_tenant_id(&resources, auth.user_id, auth.active_tenant_id).await?;
    let grants = resources
        .common
        .repos
        .oauth2_server
        .list_client_grants(&auth.user_id.to_string(), &tenant_id)
        .await?;
    let grants = grants
        .into_iter()
        .map(|grant| GrantView {
            id: grant.id,
            client_id: grant.client_id,
            scope: grant.scope,
            granted_at: grant.granted_at.to_rfc3339(),
        })
        .collect();
    Ok(Json(GrantsResponse { grants }).into_response())
}

/// `DELETE /api/me/oauth-grants/{id}` — revoke one of the caller's grants.
async fn handle_revoke_grant(
    State(resources): State<Arc<ServerContext>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let auth = extract_auth_from_headers(&headers, &resources).await?;
    let tenant_id = resolve_tenant_id(&resources, auth.user_id, auth.active_tenant_id).await?;
    let revoked = resources
        .common
        .repos
        .oauth2_server
        .revoke_client_grant(&id, &auth.user_id.to_string(), &tenant_id)
        .await?;
    if revoked {
        Ok(StatusCode::NO_CONTENT.into_response())
    } else {
        Err(AppError::not_found("OAuth grant"))
    }
}

/// OAuth-grant management routes ("connected apps").
pub struct OAuthGrantsRoutes;

impl OAuthGrantsRoutes {
    /// Build the `/api/me/oauth-grants` routes.
    pub fn routes(resources: Arc<ServerContext>) -> Router {
        Router::new()
            .route("/api/me/oauth-grants", get(handle_list_grants))
            .route("/api/me/oauth-grants/{id}", delete(handle_revoke_grant))
            .with_state(resources)
    }
}
