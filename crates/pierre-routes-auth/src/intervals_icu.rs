// ABOUTME: Intervals.icu API-key link routes — validate athlete_id + API key live, then persist
// ABOUTME: Non-OAuth linking: HTTP Basic (athlete_id:api_key) stored as a ConnectionType::Manual connection

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Intervals.icu link routes
//!
//! Intervals.icu authenticates with an athlete-generated API key (HTTP Basic
//! `athlete_id:api_key`), not OAuth, so it has no redirect/callback flow. The
//! athlete pastes their athlete id + API key and `POST`s them here. The handler
//! validates the pair against the live API before persisting: the API key is
//! stored encrypted in the access-token column, the athlete id plaintext in
//! `user_oauth_tokens.provider_user_id`, and a `Manual` provider connection is
//! registered so the resolver treats it like any other connected backend.

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::Utc;
use pierre_core::constants::oauth::INTERVALS_ICU;
use pierre_core::errors::AppError;
use pierre_core::models::{ConnectionType, TenantId, UserOAuthToken};
use pierre_providers::OAuth2Credentials;
use serde::Deserialize;
use tracing::info;
use uuid::Uuid;

use crate::AuthRoutesContext;

/// Request body for linking an Intervals.icu account.
#[derive(Debug, Deserialize)]
pub struct IntervalsIcuLinkRequest {
    /// Athlete id (e.g. `i123456`) — the HTTP Basic username.
    pub athlete_id: String,
    /// Personal API key from the athlete's Intervals.icu settings — the password.
    pub api_key: String,
}

/// Resolve the authenticated `user_id` + active `tenant_id` from request headers.
async fn authenticate(
    resources: &AuthRoutesContext,
    headers: &HeaderMap,
) -> Result<(Uuid, Uuid), AppError> {
    let auth_result = resources
        .auth_middleware
        .authenticate_request_with_headers(headers)
        .await?;
    let tenant_id = auth_result
        .active_tenant_id
        .ok_or_else(|| AppError::invalid_input("No active tenant"))?;
    Ok((auth_result.user_id, tenant_id))
}

/// `POST /api/providers/intervals_icu/link-credentials`
///
/// Validates the athlete id + API key against the live Intervals.icu API (a
/// `GET /api/v1/athlete/{id}` round-trip) before persisting, so a bad key is
/// rejected at link time rather than on the first data fetch.
///
/// # Errors
///
/// Returns [`AppError`] when the request is unauthenticated, the body is
/// missing fields, the provider is unavailable (feature not compiled in), or
/// Intervals.icu rejects the credentials.
pub async fn handle_intervals_icu_link(
    State(resources): State<AuthRoutesContext>,
    headers: HeaderMap,
    Json(request): Json<IntervalsIcuLinkRequest>,
) -> Result<Response, AppError> {
    let (user_id, tenant_id) = authenticate(&resources, &headers).await?;

    let athlete_id = request.athlete_id.trim().to_owned();
    let api_key = request.api_key.trim().to_owned();
    if athlete_id.is_empty() {
        return Err(AppError::invalid_input("athlete_id is required"));
    }
    if api_key.is_empty() {
        return Err(AppError::invalid_input("api_key is required"));
    }

    // Validate the credentials live before persisting them.
    let provider = resources
        .provider_registry
        .create_provider(INTERVALS_ICU)
        .map_err(|e| AppError::invalid_input(format!("Intervals.icu provider unavailable: {e}")))?;
    provider
        .set_credentials(OAuth2Credentials {
            client_id: athlete_id.clone(),
            client_secret: String::new(),
            access_token: Some(api_key.clone()),
            refresh_token: None,
            expires_at: None,
            scopes: vec![],
        })
        .await?;
    let athlete = provider.get_athlete().await.map_err(|e| {
        AppError::auth_invalid(format!(
            "Intervals.icu rejected those credentials — check the athlete id and API key ({e})"
        ))
    })?;

    let now = Utc::now();
    let token = UserOAuthToken {
        id: Uuid::new_v4().to_string(),
        user_id,
        tenant_id: tenant_id.to_string(),
        provider: INTERVALS_ICU.to_owned(),
        // The API key is the HTTP Basic password — encrypted at rest.
        access_token: api_key,
        refresh_token: None,
        token_type: "api_key".to_owned(),
        expires_at: None,
        scope: None,
        // The athlete id is the HTTP Basic username — not secret, stored plaintext.
        provider_user_id: Some(athlete_id),
        created_at: now,
        updated_at: now,
    };
    resources.repos.oauth_tokens.upsert_token(&token).await?;

    let tenant = TenantId::from(tenant_id);
    resources
        .repos
        .provider_connections
        .register_connection(
            user_id,
            tenant,
            INTERVALS_ICU,
            &ConnectionType::Manual,
            None,
        )
        .await
        .map_err(|e| AppError::internal(format!("Failed to register connection: {e}")))?;

    info!(user_id = %user_id, "Intervals.icu account linked");

    Ok(Json(serde_json::json!({
        "status": "connected",
        "provider": INTERVALS_ICU,
        "athlete": {
            "id": athlete.id,
            "name": athlete.firstname,
        }
    }))
    .into_response())
}

/// `DELETE /api/providers/intervals_icu/disconnect`
///
/// Removes the stored API key + athlete id and the provider connection.
///
/// # Errors
///
/// Returns [`AppError`] when the request is unauthenticated or the delete fails.
pub async fn handle_intervals_icu_disconnect(
    State(resources): State<AuthRoutesContext>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let (user_id, tenant_id) = authenticate(&resources, &headers).await?;
    let tenant = TenantId::from(tenant_id);

    resources
        .repos
        .oauth_tokens
        .delete_token(user_id, tenant, INTERVALS_ICU)
        .await?;
    resources
        .repos
        .provider_connections
        .remove_connection(user_id, tenant, INTERVALS_ICU)
        .await
        .map_err(|e| AppError::internal(format!("Failed to remove connection: {e}")))?;

    info!(user_id = %user_id, "Intervals.icu account disconnected");

    Ok(StatusCode::NO_CONTENT.into_response())
}
