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
use pierre_auth::oauth2_server::client_registration::client_display_host;
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
    resources
        .common
        .repos
        .tenants
        .first_for_user(user_id)
        .await?
        .map(|tenant_id| tenant_id.to_string())
        .ok_or_else(|| AppError::auth_invalid("User has no tenant"))
}

/// One connected MCP OAuth client in the caller's "connected apps" list.
#[derive(Serialize)]
struct GrantView {
    /// Grant id (used to revoke).
    id: String,
    /// OAuth client the user approved.
    client_id: String,
    /// The name the client registered. It chose that name itself, so it is
    /// shown with `client_host`; `None` when it registered none.
    client_name: Option<String>,
    /// Where the client is: the host of its metadata document, which it cannot
    /// fake, else the host it receives its authorization code on.
    client_host: Option<String>,
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
    let mut views = Vec::with_capacity(grants.len());
    for grant in grants {
        let client = resources
            .common
            .repos
            .oauth2_server
            .get_client(&grant.client_id)
            .await?;
        let redirect_uris = client.as_ref().map_or(&[][..], |c| &c.redirect_uris[..]);
        let client_host = client_display_host(&grant.client_id, redirect_uris);
        views.push(GrantView {
            id: grant.id,
            client_name: client.and_then(|client| client.client_name),
            client_host,
            client_id: grant.client_id,
            scope: grant.scope,
            granted_at: grant.granted_at.to_rfc3339(),
        });
    }
    Ok(Json(GrantsResponse { grants: views }).into_response())
}

/// `DELETE /api/me/oauth-grants/{id}` — revoke one of the caller's grants.
async fn handle_revoke_grant(
    State(resources): State<Arc<ServerContext>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let auth = extract_auth_from_headers(&headers, &resources).await?;
    let tenant_id = resolve_tenant_id(&resources, auth.user_id, auth.active_tenant_id).await?;
    let user_id = auth.user_id.to_string();
    let repository = &resources.common.repos.oauth2_server;
    // Named before the revocation: the resource servers hold the client's
    // standing by client, and a revoked grant no longer lists.
    let client_id = repository
        .list_client_grants(&user_id, &tenant_id)
        .await?
        .into_iter()
        .find(|grant| grant.id == id)
        .map(|grant| grant.client_id);
    let revoked = repository
        .revoke_client_grant(&id, &user_id, &tenant_id)
        .await?;
    if let Some(client_id) = client_id.filter(|_| revoked) {
        resources
            .account_service()
            .grant_standings()
            .forget(auth.user_id, &tenant_id, &client_id)
            .await;
        Ok(StatusCode::NO_CONTENT.into_response())
    } else {
        Err(AppError::not_found("OAuth grant"))
    }
}

/// OAuth-grant management routes ("connected apps").
#[derive(Debug)]
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
