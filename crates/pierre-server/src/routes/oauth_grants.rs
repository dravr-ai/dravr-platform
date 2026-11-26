// ABOUTME: HTTP boundary for a user's connected MCP apps — list and revoke OAuth client grants, read their receipts
// ABOUTME: Backs the consent feature's revoke surface and grant history (web + mobile "Connected apps" settings)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Routes for managing OAuth client grants (connected MCP apps).
//!
//! - `GET /api/me/oauth-grants` — list the caller's active grants.
//! - `DELETE /api/me/oauth-grants/{id}` — revoke a grant (owner-scoped).
//! - `GET /api/me/oauth-grants/{id}/receipts` — one grant's signed receipts
//!   (issue, revoke, expire), newest first, live or ended (carnet#1544).
//! - `GET /api/me/oauth-grant-receipts` — every receipt of the caller's
//!   grants, newest first and paginated, including those of grants since
//!   revoked or deleted with their client.
//! - `GET /api/me/oauth-standing-policies` — the caller's live standing
//!   policies ("always allow this app this preset"), newest first.
//! - `DELETE /api/me/oauth-standing-policies/{id}` — remove one: renewals
//!   stop, and the grants it renewed stay as they are.
//!
//! A grant is recorded when the user approves an MCP client on the OAuth consent
//! screen; revoking one here means the next authorization for that client shows
//! the consent screen again.

use std::collections::HashMap;
use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{delete, get},
    Json, Router,
};
use dravr_tronc::dates::Timestamp;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::warn;

use crate::mcp::resources::ServerContext;
use pierre_auth::oauth2_server::authorization_details::{ResolvedDetails, Tier};
use pierre_auth::oauth2_server::client_registration::client_display_host;
use pierre_core::errors::AppError;
use pierre_core::models::{
    GrantActor, GrantEvent, OAuthClientGrant, OAuthGrantEvent, OAuthStandingPolicy,
};
use pierre_middleware::extract_auth_from_headers;
use pierre_runtime_context::require_tenant;

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
    /// The preset a detailed grant was approved for (`training_coach`,
    /// `sleep_coach`); `None` for a plain grant.
    preset: Option<&'static str>,
    /// The highest tier a detailed grant reads (`T0` to `T3`); `None` for a
    /// plain grant.
    max_tier: Option<Tier>,
    /// When the grant stops admitting a token (RFC 3339); `None` never
    /// expires. A past value is an expired grant, listed so it can be
    /// revoked.
    expires_at: Option<String>,
    /// The grant's live one-off T2 question (a step-up child, carnet#1544
    /// slice 4b), listed under its app and revocable on its own; `None`
    /// when it has none.
    step_up: Option<StepUpChildView>,
}

/// A connected app's live step-up child.
#[derive(Serialize)]
struct StepUpChildView {
    /// The child's grant id (used to revoke it alone).
    id: String,
    /// When it ends (RFC 3339).
    expires_at: Option<String>,
}

/// The preset and tier `grant`'s stored details name, `(None, None)` for a
/// plain grant.
///
/// A document that does not read is logged and listed as neither: this is
/// the surface the athlete revokes from, so one bad row never hides the
/// rest. Admission refuses such a grant on its own (the ceiling errs).
fn preset_and_tier(grant: &OAuthClientGrant) -> (Option<&'static str>, Option<Tier>) {
    let Some(document) = grant.authorization_details.as_deref() else {
        return (None, None);
    };
    match ResolvedDetails::from_document(document) {
        Ok(details) => (Some(details.preset().name()), Some(details.max_tier())),
        Err(error) => {
            warn!(grant_id = %grant.id, error = %error, "A connected app's grant details do not read");
            (None, None)
        }
    }
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
    // The tenant a grant is keyed under: the one the session names when the
    // account still belongs to it, otherwise the account's first, which is how
    // the consent flow keys a grant whose session names none.
    let tenant_id = require_tenant(&resources, &auth).await?.to_string();
    let grants = resources
        .common
        .repos
        .oauth2_server
        .list_client_grants(&auth.user_id.to_string(), &tenant_id)
        .await?;
    let children = resources
        .common
        .repos
        .oauth2_server
        .list_live_child_grants(&auth.user_id.to_string(), &tenant_id)
        .await?;
    // An expired child keeps `revoked_at` NULL (the expire pass writes only
    // its receipt), so it is left out here: it reads nothing any more.
    let now = Timestamp::now();
    let mut views = Vec::with_capacity(grants.len());
    for grant in grants {
        let step_up = children
            .iter()
            .find(|child| child.terms.parent_grant_id == grant.id && child.grant.is_live_at(now))
            .map(|child| StepUpChildView {
                id: child.grant.id.clone(),
                expires_at: child.grant.expires_at.map(Timestamp::to_rfc3339),
            });
        let client = resources
            .common
            .repos
            .oauth2_server
            .get_client(&grant.client_id)
            .await?;
        let redirect_uris = client.as_ref().map_or(&[][..], |c| &c.redirect_uris[..]);
        let client_host = client_display_host(&grant.client_id, redirect_uris);
        let (preset, max_tier) = preset_and_tier(&grant);
        views.push(GrantView {
            id: grant.id,
            client_name: client.and_then(|client| client.client_name),
            client_host,
            client_id: grant.client_id,
            scope: grant.scope,
            granted_at: grant.granted_at.to_rfc3339(),
            preset,
            max_tier,
            expires_at: grant.expires_at.map(Timestamp::to_rfc3339),
            step_up,
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
    // The tenant a grant is keyed under: the one the session names when the
    // account still belongs to it, otherwise the account's first, which is how
    // the consent flow keys a grant whose session names none.
    let tenant_id = require_tenant(&resources, &auth).await?.to_string();
    let user_id = auth.user_id.to_string();
    let repository = &resources.common.repos.oauth2_server;
    // Named before the revocation: the resource servers hold the client's
    // standing by client, and a revoked grant no longer lists. A step-up
    // child is revoked alone, its parent standing.
    let mut client_id = repository
        .list_client_grants(&user_id, &tenant_id)
        .await?
        .into_iter()
        .find(|grant| grant.id == id)
        .map(|grant| grant.client_id);
    if client_id.is_none() {
        client_id = repository
            .list_live_child_grants(&user_id, &tenant_id)
            .await?
            .into_iter()
            .find(|child| child.grant.id == id)
            .map(|child| child.grant.client_id);
    }
    let revoked = repository
        .revoke_client_grant(&id, &user_id, &tenant_id, GrantActor::User)
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

/// Receipts a page returns when the caller names no `limit`.
const DEFAULT_RECEIPT_PAGE: u32 = 50;

/// The most receipts one page returns.
const MAX_RECEIPT_PAGE: u32 = 100;

/// A page of receipts: `limit` held to `1..=MAX_RECEIPT_PAGE`.
#[derive(Deserialize)]
struct ReceiptPageQuery {
    #[serde(default)]
    limit: Option<u32>,
    #[serde(default)]
    offset: u32,
}

impl ReceiptPageQuery {
    /// The page size asked, held to its bounds.
    fn limit(&self) -> u32 {
        self.limit
            .unwrap_or(DEFAULT_RECEIPT_PAGE)
            .clamp(1, MAX_RECEIPT_PAGE)
    }
}

/// One signed receipt of a grant event.
#[derive(Serialize)]
struct ReceiptView {
    /// The receipt's id (`receipt_id` in its payload).
    id: String,
    /// The grant it is about.
    client_grant_id: String,
    /// `issue`, `revoke` or `expire`.
    event: GrantEvent,
    /// `user`, `client` or `auto`.
    actor: GrantActor,
    /// When it happened (RFC 3339).
    at: String,
    /// The receipt payload, as signed.
    payload: Value,
    /// The name the receipt's client registered, read now, so a revoked
    /// app's history still names it; `None` when it registered none or is
    /// deleted. Not part of what was signed.
    client_name: Option<String>,
    /// Hex SHA-256 of the stored payload bytes: the athlete's `record_log`
    /// entry names it.
    payload_sha256: String,
    /// The compact JWS over the payload; `None` until it is signed.
    jws: Option<String>,
    /// When its `record_log` entry was appended (RFC 3339); `None` until then.
    logged_at: Option<String>,
}

impl ReceiptView {
    /// `event` as the athlete reads it.
    fn of(event: OAuthGrantEvent) -> Result<Self, AppError> {
        let payload = serde_json::from_str(&event.payload).map_err(|e| {
            AppError::internal(format!("grant receipt {} does not read: {e}", event.id))
        })?;
        Ok(Self {
            id: event.id,
            client_grant_id: event.client_grant_id,
            event: event.event,
            actor: event.actor,
            at: event.at.to_rfc3339(),
            payload,
            client_name: None,
            payload_sha256: event.payload_sha256,
            jws: event.jws,
            logged_at: event.logged_at.map(Timestamp::to_rfc3339),
        })
    }
}

/// `events` as the athlete reads them, each named by the name its client
/// registered, read once per client.
async fn receipt_views(
    resources: &Arc<ServerContext>,
    events: Vec<OAuthGrantEvent>,
) -> Result<Vec<ReceiptView>, AppError> {
    let mut views = events
        .into_iter()
        .map(ReceiptView::of)
        .collect::<Result<Vec<_>, _>>()?;
    let mut names: HashMap<String, Option<String>> = HashMap::new();
    for view in &mut views {
        let Some(client_id) = view.payload["grant"]["client_id"]
            .as_str()
            .map(str::to_owned)
        else {
            continue;
        };
        if !names.contains_key(&client_id) {
            let name = resources
                .common
                .repos
                .oauth2_server
                .get_client(&client_id)
                .await?
                .and_then(|client| client.client_name);
            names.insert(client_id.clone(), name);
        }
        view.client_name = names.get(&client_id).cloned().flatten();
    }
    Ok(views)
}

/// Response body of the receipt listings.
#[derive(Serialize)]
struct ReceiptsResponse {
    /// The receipts, newest first.
    receipts: Vec<ReceiptView>,
}

/// `GET /api/me/oauth-grants/{id}/receipts` — one of the caller's grants'
/// receipts, newest first. A grant deleted with its client still lists its
/// receipts; an id that names neither a grant nor a receipt of the caller's
/// is `404`.
async fn handle_grant_receipts(
    State(resources): State<Arc<ServerContext>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(page): Query<ReceiptPageQuery>,
) -> Result<Response, AppError> {
    let auth = extract_auth_from_headers(&headers, &resources).await?;
    let tenant_id = require_tenant(&resources, &auth).await?.to_string();
    let user_id = auth.user_id.to_string();
    let repos = &resources.common.repos;
    let events = repos
        .oauth_grant_events
        .list_for_user(&tenant_id, &user_id, Some(&id), page.limit(), page.offset)
        .await?;
    if events.is_empty()
        && repos
            .oauth2_server
            .get_client_grant(&id, &user_id, &tenant_id)
            .await?
            .is_none()
    {
        return Err(AppError::not_found("OAuth grant"));
    }
    let receipts = receipt_views(&resources, events).await?;
    Ok(Json(ReceiptsResponse { receipts }).into_response())
}

/// `GET /api/me/oauth-grant-receipts` — every receipt of the caller's grants
/// in their tenant, newest first, one page.
async fn handle_all_receipts(
    State(resources): State<Arc<ServerContext>>,
    headers: HeaderMap,
    Query(page): Query<ReceiptPageQuery>,
) -> Result<Response, AppError> {
    let auth = extract_auth_from_headers(&headers, &resources).await?;
    let tenant_id = require_tenant(&resources, &auth).await?.to_string();
    let events = resources
        .common
        .repos
        .oauth_grant_events
        .list_for_user(
            &tenant_id,
            &auth.user_id.to_string(),
            None,
            page.limit(),
            page.offset,
        )
        .await?;
    let receipts = receipt_views(&resources, events).await?;
    Ok(Json(ReceiptsResponse { receipts }).into_response())
}

/// One of the caller's standing policies.
#[derive(Serialize)]
struct StandingPolicyView {
    /// The policy's id (used to remove it), cited by every grant it renewed.
    id: String,
    /// The client it always allows.
    client_id: String,
    /// The name the client registered; `None` when it registered none.
    client_name: Option<String>,
    /// Where the client is, as Connected apps shows it.
    client_host: Option<String>,
    /// The preset it renews (`training_coach`, `sleep_coach`).
    preset: String,
    /// When the athlete ticked the box (RFC 3339).
    created_at: String,
}

/// Response body of `GET /api/me/oauth-standing-policies`.
#[derive(Serialize)]
struct StandingPoliciesResponse {
    /// The caller's live policies, newest first.
    policies: Vec<StandingPolicyView>,
}

/// `policy` as the athlete reads it, its client named as Connected apps
/// names it.
async fn standing_policy_view(
    resources: &Arc<ServerContext>,
    policy: OAuthStandingPolicy,
) -> Result<StandingPolicyView, AppError> {
    let client = resources
        .common
        .repos
        .oauth2_server
        .get_client(&policy.client_id)
        .await?;
    let redirect_uris = client.as_ref().map_or(&[][..], |c| &c.redirect_uris[..]);
    let client_host = client_display_host(&policy.client_id, redirect_uris);
    Ok(StandingPolicyView {
        id: policy.id,
        client_name: client.and_then(|client| client.client_name),
        client_host,
        client_id: policy.client_id,
        preset: policy.preset,
        created_at: policy.created_at.to_rfc3339(),
    })
}

/// `GET /api/me/oauth-standing-policies` — the caller's live standing
/// policies in their tenant.
async fn handle_list_standing_policies(
    State(resources): State<Arc<ServerContext>>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let auth = extract_auth_from_headers(&headers, &resources).await?;
    let tenant_id = require_tenant(&resources, &auth).await?.to_string();
    let policies = resources
        .common
        .repos
        .oauth_standing_policies
        .list_standing_policies(&tenant_id, &auth.user_id.to_string(), None)
        .await?;
    let mut views = Vec::with_capacity(policies.len());
    for policy in policies {
        views.push(standing_policy_view(&resources, policy).await?);
    }
    Ok(Json(StandingPoliciesResponse { policies: views }).into_response())
}

/// `DELETE /api/me/oauth-standing-policies/{id}` — remove one of the
/// caller's live standing policies; another's, or one already removed, is
/// `404`.
async fn handle_revoke_standing_policy(
    State(resources): State<Arc<ServerContext>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let auth = extract_auth_from_headers(&headers, &resources).await?;
    let tenant_id = require_tenant(&resources, &auth).await?.to_string();
    let removed = resources
        .common
        .repos
        .oauth_standing_policies
        .revoke_standing_policy(&tenant_id, &auth.user_id.to_string(), &id)
        .await?;
    if removed {
        Ok(StatusCode::NO_CONTENT.into_response())
    } else {
        Err(AppError::not_found("Standing policy"))
    }
}

/// OAuth-grant management routes ("connected apps").
#[derive(Debug)]
pub struct OAuthGrantsRoutes;

impl OAuthGrantsRoutes {
    /// Build the `/api/me/oauth-grants`, `/api/me/oauth-grant-receipts` and
    /// `/api/me/oauth-standing-policies` routes.
    pub fn routes(resources: Arc<ServerContext>) -> Router {
        Router::new()
            .route("/api/me/oauth-grants", get(handle_list_grants))
            .route("/api/me/oauth-grants/{id}", delete(handle_revoke_grant))
            .route(
                "/api/me/oauth-grants/{id}/receipts",
                get(handle_grant_receipts),
            )
            .route("/api/me/oauth-grant-receipts", get(handle_all_receipts))
            .route(
                "/api/me/oauth-standing-policies",
                get(handle_list_standing_policies),
            )
            .route(
                "/api/me/oauth-standing-policies/{id}",
                delete(handle_revoke_standing_policy),
            )
            .with_state(resources)
    }
}
