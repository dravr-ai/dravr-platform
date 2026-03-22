// ABOUTME: Sciotte provider routes for credential-based login and session management
// ABOUTME: Collects credentials via Pierre's UI, runs in-process Chrome login via dravr-sciotte
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::Utc;
use pierre_core::models::{ConnectionType, TenantId, UserOAuthToken};
use serde::Deserialize;
use tokio::sync::Mutex;
use tracing::info;
use uuid::Uuid;

use crate::errors::AppError;
use crate::mcp::resources::ServerResources;

#[cfg(feature = "provider-sciotte")]
use dravr_sciotte::cache::CachedScraper;
#[cfg(feature = "provider-sciotte")]
use dravr_sciotte::error::LoginResult;
#[cfg(feature = "provider-sciotte")]
use dravr_sciotte::models::AuthSession;
#[cfg(feature = "provider-sciotte")]
use dravr_sciotte::scraper::ChromeScraper;
#[cfg(feature = "provider-sciotte")]
use dravr_sciotte::ActivityScraper;

// Pending OTP scraper — holds the Chrome browser between multi-step login calls.
// Keyed by `user_id` to prevent cross-user interference.
/// Pending login session: scraper + provider name (e.g., `sciotte` or `sciotte_garmin`)
#[cfg(feature = "provider-sciotte")]
type PendingScraper = (CachedScraper<ChromeScraper>, String);

#[cfg(feature = "provider-sciotte")]
static PENDING_OTP_SCRAPERS: LazyLock<Mutex<HashMap<Uuid, PendingScraper>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Debug, Deserialize)]
pub struct SciotteLoginRequest {
    pub email: String,
    pub password: String,
    #[serde(default = "default_method")]
    pub method: String,
    /// Target platform: "strava" (default) or "garmin"
    #[serde(default = "default_target")]
    pub target: String,
}

fn default_method() -> String {
    "email".to_owned()
}

fn default_target() -> String {
    "strava".to_owned()
}

/// Create a sciotte scraper configured for the target platform
#[cfg(feature = "provider-sciotte")]
fn create_scraper_for_target(target: &str) -> CachedScraper<ChromeScraper> {
    use dravr_sciotte::config::{CacheConfig, ScraperConfig};
    use dravr_sciotte::provider::ProviderConfig as SciotteProviderConfig;

    let provider_config = match target {
        "garmin" => SciotteProviderConfig::garmin_default(),
        _ => SciotteProviderConfig::strava_default(),
    };
    let scraper = ChromeScraper::new(ScraperConfig::default(), provider_config);
    CachedScraper::new(scraper, &CacheConfig::default())
}

/// Get the Pierre provider name for the target
fn provider_name_for_target(target: &str) -> &'static str {
    match target {
        "garmin" => "sciotte_garmin",
        _ => "sciotte",
    }
}

#[derive(Debug, Deserialize)]
pub struct SciotteSelectTwoFactorRequest {
    pub option_id: String,
}

#[derive(Debug, Deserialize)]
pub struct SciotteOtpRequest {
    pub code: String,
}

#[derive(Debug, Deserialize)]
pub struct SciotteConnectRequest {
    pub session_id: String,
}

/// Store a successful sciotte session in Pierre's encrypted DB and register the connection
#[cfg(feature = "provider-sciotte")]
async fn store_sciotte_session(
    resources: &ServerResources,
    user_id: uuid::Uuid,
    tenant_id: uuid::Uuid,
    session: &AuthSession,
    provider_name: &str,
) -> Result<Response, AppError> {
    let session_json = serde_json::to_string(session)
        .map_err(|e| AppError::internal(format!("Failed to serialize session: {e}")))?;

    let now = Utc::now();
    let token = UserOAuthToken {
        id: Uuid::new_v4().to_string(),
        user_id,
        tenant_id: tenant_id.to_string(),
        provider: provider_name.to_owned(),
        access_token: session_json,
        refresh_token: None,
        token_type: "session".to_owned(),
        expires_at: session.expires_at,
        scope: None,
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
            provider_name,
            &ConnectionType::Manual,
            None,
        )
        .await
        .map_err(|e| AppError::internal(format!("Failed to register connection: {e}")))?;

    Ok(Json(serde_json::json!({"status": "connected", "provider": provider_name})).into_response())
}

/// Convert a `LoginResult` into an HTTP response, storing the scraper for follow-up calls if needed
#[cfg(feature = "provider-sciotte")]
async fn login_result_to_response(
    result: LoginResult,
    resources: &ServerResources,
    user_id: uuid::Uuid,
    tenant_id: uuid::Uuid,
    scraper: CachedScraper<ChromeScraper>,
    log_prefix: &str,
    provider_name: &str,
) -> Result<Response, AppError> {
    match result {
        LoginResult::Success(session) => {
            info!(user_id = %user_id, "{log_prefix} successful");
            store_sciotte_session(resources, user_id, tenant_id, &session, provider_name).await
        }
        LoginResult::OtpRequired => {
            PENDING_OTP_SCRAPERS
                .lock()
                .await
                .insert(user_id, (scraper, provider_name.to_owned()));
            Ok(Json(serde_json::json!({"status": "otp_required"})).into_response())
        }
        LoginResult::TwoFactorChoice(options) => {
            PENDING_OTP_SCRAPERS
                .lock()
                .await
                .insert(user_id, (scraper, provider_name.to_owned()));
            let options_json: Vec<serde_json::Value> = options
                .iter()
                .map(|o| serde_json::json!({"id": o.id, "label": o.label}))
                .collect();
            Ok(
                Json(serde_json::json!({"status": "two_factor_choice", "options": options_json}))
                    .into_response(),
            )
        }
        LoginResult::Failed(reason) => {
            Ok(Json(serde_json::json!({"status": "failed", "error": reason})).into_response())
        }
    }
}

/// Extract authenticated `user_id` and `tenant_id` from request headers
async fn authenticate(
    resources: &ServerResources,
    headers: &HeaderMap,
) -> Result<(uuid::Uuid, uuid::Uuid), AppError> {
    let auth_result = resources
        .auth_middleware
        .authenticate_request_with_headers(headers)
        .await?;
    let tenant_id = auth_result
        .active_tenant_id
        .ok_or_else(|| AppError::invalid_input("No active tenant"))?;
    Ok((auth_result.user_id, tenant_id))
}

/// Take the pending scraper + provider name for a user, or return an error
#[cfg(feature = "provider-sciotte")]
async fn take_pending_scraper(user_id: uuid::Uuid) -> Result<PendingScraper, AppError> {
    PENDING_OTP_SCRAPERS
        .lock()
        .await
        .remove(&user_id)
        .ok_or_else(|| AppError::invalid_input("No pending login session — please log in again"))
}

/// Credential-based login via in-process dravr-sciotte
#[cfg(feature = "provider-sciotte")]
pub(super) async fn handle_sciotte_login(
    State(resources): State<Arc<ServerResources>>,
    headers: HeaderMap,
    Json(request): Json<SciotteLoginRequest>,
) -> Result<Response, AppError> {
    let (user_id, tenant_id) = authenticate(&resources, &headers).await?;

    if request.email.is_empty() || request.password.is_empty() {
        return Err(AppError::invalid_input("Email and password are required"));
    }

    let target = &request.target;
    let provider = provider_name_for_target(target);
    info!(user_id = %user_id, target = %target, "Starting sciotte credential login");

    let cached = create_scraper_for_target(target);

    let result = cached
        .credential_login(&request.email, &request.password, &request.method)
        .await
        .map_err(|e| AppError::internal(format!("Sciotte login failed: {e}")))?;

    login_result_to_response(
        result,
        &resources,
        user_id,
        tenant_id,
        cached,
        "Sciotte credential login",
        provider,
    )
    .await
}

/// Select a 2FA method for a pending login
#[cfg(feature = "provider-sciotte")]
pub(super) async fn handle_sciotte_select_2fa(
    State(resources): State<Arc<ServerResources>>,
    headers: HeaderMap,
    Json(request): Json<SciotteSelectTwoFactorRequest>,
) -> Result<Response, AppError> {
    let (user_id, tenant_id) = authenticate(&resources, &headers).await?;
    let (scraper, provider_name) = take_pending_scraper(user_id).await?;

    info!(user_id = %user_id, option = %request.option_id, "Selecting 2FA method");

    let result = scraper
        .select_two_factor(&request.option_id)
        .await
        .map_err(|e| AppError::internal(format!("2FA selection failed: {e}")))?;

    login_result_to_response(
        result,
        &resources,
        user_id,
        tenant_id,
        scraper,
        "Sciotte 2FA login",
        &provider_name,
    )
    .await
}

/// Submit OTP code for a pending login
#[cfg(feature = "provider-sciotte")]
pub(super) async fn handle_sciotte_submit_otp(
    State(resources): State<Arc<ServerResources>>,
    headers: HeaderMap,
    Json(request): Json<SciotteOtpRequest>,
) -> Result<Response, AppError> {
    let (user_id, tenant_id) = authenticate(&resources, &headers).await?;

    if request.code.is_empty() {
        return Err(AppError::invalid_input("Verification code is required"));
    }

    let (scraper, provider_name) = take_pending_scraper(user_id).await?;

    info!(user_id = %user_id, "Submitting OTP code");

    let result = scraper
        .submit_otp(&request.code)
        .await
        .map_err(|e| AppError::internal(format!("OTP submission failed: {e}")))?;

    login_result_to_response(
        result,
        &resources,
        user_id,
        tenant_id,
        scraper,
        "Sciotte OTP login",
        &provider_name,
    )
    .await
}

/// Connect with a pre-existing serialized session (used by external sciotte CLI)
pub(super) async fn handle_sciotte_connect(
    State(resources): State<Arc<ServerResources>>,
    headers: HeaderMap,
    Json(request): Json<SciotteConnectRequest>,
) -> Result<Response, AppError> {
    let (user_id, tenant_id) = authenticate(&resources, &headers).await?;

    if request.session_id.is_empty() {
        return Err(AppError::invalid_input("session_id is required"));
    }

    let now = Utc::now();
    let token = UserOAuthToken {
        id: Uuid::new_v4().to_string(),
        user_id,
        tenant_id: tenant_id.to_string(),
        provider: "sciotte".to_owned(),
        access_token: request.session_id,
        refresh_token: None,
        token_type: "session".to_owned(),
        expires_at: None,
        scope: None,
        created_at: now,
        updated_at: now,
    };

    resources.repos.oauth_tokens.upsert_token(&token).await?;

    let tenant = TenantId::from(tenant_id);
    resources
        .repos
        .provider_connections
        .register_connection(user_id, tenant, "sciotte", &ConnectionType::Manual, None)
        .await
        .map_err(|e| AppError::internal(format!("Failed to register connection: {e}")))?;

    info!(user_id = %user_id, "Sciotte session connected");

    Ok(Json(serde_json::json!({"status": "connected", "provider": "sciotte"})).into_response())
}

/// Disconnect the sciotte session
pub(super) async fn handle_sciotte_disconnect(
    State(resources): State<Arc<ServerResources>>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let (user_id, tenant_id) = authenticate(&resources, &headers).await?;
    let tenant = TenantId::from(tenant_id);

    resources
        .repos
        .oauth_tokens
        .delete_token(user_id, tenant, "sciotte")
        .await?;

    info!(user_id = %user_id, "Sciotte session disconnected");

    Ok(StatusCode::NO_CONTENT.into_response())
}
