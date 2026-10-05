// ABOUTME: Intervals.icu API-key link routes — validate athlete_id + API key live, then persist
// ABOUTME: The key is stored as a ConnectionType::Manual connection; the OAuth link runs through the shared OAuth routes

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Intervals.icu link routes
//!
//! An athlete links Intervals.icu through the Dravr OAuth app (the shared
//! `/api/oauth/*` routes), or by pasting their athlete id + personal API key
//! (HTTP Basic `API_KEY:<api key>`) and `POST`ing them here. The handler
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
use pierre_core::models::{Athlete, ConnectionType, TenantId, UserOAuthToken, API_KEY_TOKEN_TYPE};
use pierre_providers::{CredentialKind, OAuth2Credentials};
use pierre_services::delegated_connections::{forget_coach_roster, supersede_delegated_link};
use pierre_services::oauth_flow::OAuthService;
use pierre_services::provider_revocation::DisconnectReason;
use serde::Deserialize;
use tracing::info;
use uuid::Uuid;

#[cfg(feature = "health-sync")]
use crate::oauth::spawn_health_backfill;
use crate::AuthRoutesContext;

/// Request body for linking an Intervals.icu account.
#[derive(Debug, Deserialize)]
pub struct IntervalsIcuLinkRequest {
    /// Athlete id (e.g. `i123456`) — addresses the athlete-scoped API path.
    /// It is *not* the HTTP Basic username; Intervals.icu wants the literal
    /// string `API_KEY` there.
    pub athlete_id: String,
    /// Personal API key from the athlete's Intervals.icu settings — the password.
    pub api_key: String,
}

/// Why an Intervals.icu link attempt did not go through.
///
/// The JSON route answers with the [`AppError`] each kind converts into; the
/// hosted connect form words each kind from the catalogue, in the athlete's
/// locale, rather than showing that English text.
#[derive(Debug)]
pub enum IntervalsLinkError {
    /// The athlete id field was empty.
    MissingAthleteId,
    /// The API key field was empty.
    MissingApiKey,
    /// This build cannot create the Intervals.icu provider; the reason.
    Unavailable(String),
    /// Intervals.icu did not accept the athlete id + API key; its reason.
    Rejected(String),
    /// Setting the credentials or storing the link failed.
    Storage(AppError),
}

impl From<AppError> for IntervalsLinkError {
    fn from(error: AppError) -> Self {
        Self::Storage(error)
    }
}

impl From<IntervalsLinkError> for AppError {
    fn from(error: IntervalsLinkError) -> Self {
        match error {
            IntervalsLinkError::MissingAthleteId => Self::invalid_input("athlete_id is required"),
            IntervalsLinkError::MissingApiKey => Self::invalid_input("api_key is required"),
            IntervalsLinkError::Unavailable(reason) => {
                Self::invalid_input(format!("Intervals.icu provider unavailable: {reason}"))
            }
            // A rejection is about the athlete id + API key in *this request
            // body*, not about the caller's Dravr session — so it must not be
            // `auth_invalid`. That code maps to HTTP 401, and the shared
            // api-client response interceptor treats any 401 as a dead
            // session: it clears stored auth and signs the user out. A
            // mistyped API key would log the athlete out of Dravr.
            // `invalid_input` maps to 400, which is what a bad field in a
            // request body deserves.
            IntervalsLinkError::Rejected(reason) => Self::invalid_input(format!(
                "Intervals.icu rejected those credentials — check the athlete id and API key ({reason})"
            )),
            IntervalsLinkError::Storage(error) => error,
        }
    }
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
/// LIMITATION(registre#47): `handle_intervals_icu_link` is still how every athlete links: the
/// OAuth flow is built, but no intervals.icu OAuth app credentials are deployed and no client
/// offers it yet, so athletes link on the key model intervals.icu reserves for single-user
/// scripts (5,000 requests/day).
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
    let athlete = link_intervals_icu_account(
        &resources,
        user_id,
        tenant_id,
        &request.athlete_id,
        &request.api_key,
    )
    .await?;

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

/// Validate an athlete id + API key against the live Intervals.icu API, then
/// persist them for `user_id` in `tenant_id` and register the connection.
///
/// The one linking path behind both the session-authed web/mobile route
/// ([`handle_intervals_icu_link`]) and the link-token-authed hosted connect
/// form ([`crate::connect_hosted_intervals`]): the API key is stored encrypted
/// in the access-token column, the athlete id plaintext in
/// `provider_user_id`, and a `Manual` provider connection is registered so the
/// resolver treats it like any other connected backend.
///
/// # Errors
///
/// Returns [`IntervalsLinkError`] when a field is empty, the provider is
/// unavailable (feature not compiled in), Intervals.icu rejects the
/// credentials, or the token or connection cannot be stored.
pub async fn link_intervals_icu_account(
    resources: &AuthRoutesContext,
    user_id: Uuid,
    tenant_id: Uuid,
    athlete_id: &str,
    api_key: &str,
) -> Result<Athlete, IntervalsLinkError> {
    let athlete_id = athlete_id.trim().to_owned();
    let api_key = api_key.trim().to_owned();
    if athlete_id.is_empty() {
        return Err(IntervalsLinkError::MissingAthleteId);
    }
    if api_key.is_empty() {
        return Err(IntervalsLinkError::MissingApiKey);
    }

    // Validate the credentials live before persisting them.
    let provider = resources
        .provider_registry
        .create_provider(INTERVALS_ICU)
        .map_err(|e| IntervalsLinkError::Unavailable(e.to_string()))?;
    provider
        .set_credentials(OAuth2Credentials {
            client_id: athlete_id.clone(),
            client_secret: String::new(),
            access_token: Some(api_key.clone()),
            refresh_token: None,
            expires_at: None,
            scopes: vec![],
            kind: CredentialKind::ApiKey,
            request_budget: None,
        })
        .await?;
    let athlete = provider
        .get_athlete()
        .await
        .map_err(|e| IntervalsLinkError::Rejected(e.to_string()))?;

    let now = Utc::now();
    let token = UserOAuthToken {
        id: Uuid::new_v4().to_string(),
        user_id,
        tenant_id: tenant_id.to_string(),
        provider: INTERVALS_ICU.to_owned(),
        // The API key is the HTTP Basic password — encrypted at rest.
        // (The Basic username is the constant `API_KEY`, not this athlete id.)
        access_token: api_key,
        refresh_token: None,
        token_type: API_KEY_TOKEN_TYPE.to_owned(),
        expires_at: None,
        scope: None,
        // The athlete id addresses the athlete-scoped API path — not secret,
        // stored plaintext.
        provider_user_id: Some(athlete_id),
        oauth_app_client_id: None,
        created_at: now,
        updated_at: now,
    };
    let tenant = TenantId::from_uuid(tenant_id);
    // The athlete's own key takes the place of a link their group coach read
    // them through.
    supersede_delegated_link(&resources.repos, user_id, tenant, INTERVALS_ICU).await?;
    resources.repos.oauth_tokens.upsert_token(&token).await?;
    // A roster read through the key this one replaces may name another
    // account's athletes.
    forget_coach_roster(&resources.cache, user_id, tenant, INTERVALS_ICU).await;

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

    // The wellness feed syncs like any health source; read its last month now
    // rather than at the next scheduled pass.
    #[cfg(feature = "health-sync")]
    spawn_health_backfill(resources, &user_id.to_string(), INTERVALS_ICU, None);

    Ok(athlete)
}

/// `DELETE /api/providers/intervals_icu/disconnect`
///
/// Disconnects through the domain chokepoint every other provider uses, so an
/// OAuth link has its grant withdrawn at Intervals.icu, an API-key link has
/// its stored key deleted, and both emit the `provider.disconnected` event.
///
/// # Errors
///
/// Returns [`AppError`] when the request is unauthenticated or the delete fails.
pub async fn handle_intervals_icu_disconnect(
    State(resources): State<AuthRoutesContext>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let (user_id, tenant_id) = authenticate(&resources, &headers).await?;

    OAuthService::new(resources.data.clone(), resources.config.clone())
        .disconnect_provider(
            user_id,
            INTERVALS_ICU,
            Some(tenant_id),
            DisconnectReason::Athlete,
        )
        .await?;

    info!(user_id = %user_id, "Intervals.icu account disconnected");

    Ok(StatusCode::NO_CONTENT.into_response())
}
