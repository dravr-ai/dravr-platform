// ABOUTME: Hosted Sciotte login page for channel-initiated provider connection
// ABOUTME: Serves an HTML page that runs the full Sciotte login state machine
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Hosted Sciotte login for channel-initiated flows.
//!
//! This module implements the three public-facing pages the user sees when a
//! chat channel (Slack, Discord, Telegram, ...) hands them a login URL:
//!
//! - `GET /providers/sciotte/login?token=...` — renders the login page. The token
//!   query parameter is a provider link-token minted by
//!   `POST /api/channels/provider/sciotte/link-token`. The page embeds the token
//!   so its client-side JS can call the existing `/api/providers/sciotte/*` API
//!   with `Authorization: Bearer <token>`.
//! - `GET /providers/sciotte/success` — confirmation page, auto-closes and can
//!   deep-link back to the originating channel.
//! - `GET /providers/sciotte/error?message=...` — renders a user-facing error page.
//!
//! The mint endpoint (`POST /api/channels/provider/sciotte/link-token`) is
//! service-to-service — it requires admin credentials and is called by the
//! channel bot (e.g. dravr-canot) to hand out hosted-login URLs.

use std::env;
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::{Html, IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::errors::AppError;
use crate::mcp::resources::ServerResources;
use crate::middleware::admin_guard::require_admin;
use crate::middleware::extract_auth_from_headers;
use crate::middleware::provider_link_token::{
    mint_link_token, verify_link_token, MintProviderLinkTokenArgs, ProviderLinkTokenClaims,
    PROVIDER_LINK_TOKEN_TTL_MINUTES,
};
use crate::routes::auth::sciotte_hosted_templates;
use crate::utils::uuid::parse_uuid_with_message;

/// Fallback base URL when `BASE_URL` env var is not set
const FALLBACK_BASE_URL: &str = "http://localhost:8081";

/// Default target platform when the caller does not specify one
const DEFAULT_TARGET: &str = "strava";

/// Request body for minting a Sciotte hosted-login link-token
#[derive(Debug, Deserialize)]
pub struct MintLinkTokenRequest {
    /// Pierre user the link-token will authorize
    pub user_id: String,
    /// Active tenant for the session
    pub tenant_id: String,
    /// Target platform ("strava" or "garmin")
    #[serde(default = "default_target")]
    pub target: String,
    /// Originating channel slug (e.g. "slack", "discord")
    pub channel: String,
    /// Optional channel thread/DM id for the bot to post confirmation back to
    #[serde(default)]
    pub channel_thread: Option<String>,
}

fn default_target() -> String {
    DEFAULT_TARGET.to_owned()
}

/// Response from minting a Sciotte hosted-login link-token
#[derive(Debug, Serialize)]
pub struct MintLinkTokenResponse {
    /// The full hosted-login URL to hand to the end user
    pub url: String,
    /// The raw signed token (for callers that want to build their own URL)
    pub token: String,
    /// TTL in minutes — matches the link-token's exp claim
    pub expires_in_minutes: i64,
}

/// Query parameters for the hosted login page
#[derive(Debug, Deserialize)]
pub struct HostedLoginQuery {
    /// The signed link-token that authorizes this page to call
    /// `/api/providers/sciotte/*` on behalf of a user
    pub token: Option<String>,
}

/// Query parameters for the hosted error page
#[derive(Debug, Deserialize)]
pub struct HostedErrorQuery {
    /// User-facing error message
    pub message: Option<String>,
}

/// Query parameters for the hosted success page
#[derive(Debug, Deserialize)]
pub struct HostedSuccessQuery {
    /// Optional channel slug so the success page can show channel-specific
    /// follow-up text (e.g. "Return to Slack")
    pub channel: Option<String>,
    /// Target platform shown on the success page ("strava" / "garmin")
    pub target: Option<String>,
}

/// POST `/api/channels/provider/sciotte/link-token`
///
/// Mint a short-lived signed link-token that authorizes the bearer to complete
/// a Sciotte login on behalf of a Pierre user. Returns the full hosted-login URL.
/// Requires admin authentication (service-to-service).
pub(super) async fn handle_mint_sciotte_link_token(
    State(resources): State<Arc<ServerResources>>,
    headers: HeaderMap,
    Json(request): Json<MintLinkTokenRequest>,
) -> Result<Response, AppError> {
    let auth = extract_auth_from_headers(&headers, &resources).await?;

    // SECURITY: Service-to-service endpoint — require admin role.
    require_admin(auth.user_id, &resources.repos.users).await?;

    let user_id = parse_uuid_with_message(&request.user_id, "Invalid user_id UUID")?;
    let tenant_id = parse_uuid_with_message(&request.tenant_id, "Invalid tenant_id UUID")?;

    // Rate-limit per target user_id to prevent phishing-style link spam.
    resources.mint_rate_limiter.record_attempt(user_id).await?;

    if request.channel.is_empty() {
        return Err(AppError::invalid_input("channel is required"));
    }
    let target = match request.target.as_str() {
        "strava" | "garmin" => request.target.as_str(),
        other => {
            return Err(AppError::invalid_input(format!(
                "Unsupported target '{other}' — expected 'strava' or 'garmin'"
            )));
        }
    };

    let token = mint_link_token(
        &MintProviderLinkTokenArgs {
            user_id,
            tenant_id,
            provider: "sciotte",
            target,
            channel: &request.channel,
            channel_thread: request.channel_thread.as_deref(),
        },
        &resources.admin_jwt_secret,
    )?;

    let base_url = env::var("BASE_URL").unwrap_or_else(|_| FALLBACK_BASE_URL.to_owned());
    let url = format!(
        "{base_url}/providers/sciotte/login?token={}",
        urlencoding::encode(&token)
    );

    info!(
        user_id = %user_id,
        tenant_id = %tenant_id,
        channel = %request.channel,
        target = %target,
        "Minted Sciotte hosted-login link-token"
    );

    Ok(Json(MintLinkTokenResponse {
        url,
        token,
        expires_in_minutes: PROVIDER_LINK_TOKEN_TTL_MINUTES,
    })
    .into_response())
}

/// Validate the link-token and burn its nonce, returning the verified claims.
async fn validate_and_burn_link_token(
    resources: &ServerResources,
    token: &str,
) -> Result<ProviderLinkTokenClaims, Response> {
    let claims = verify_link_token(token, &resources.admin_jwt_secret, "sciotte").map_err(|e| {
        warn!(error = %e, "Rejected hosted-login page: invalid link-token");
        render_error_response(
            "This login link is invalid or has expired. Please request a fresh link from your chat channel.",
        )
    })?;

    // SECURITY: Burn the jti on the first page load so the URL can't be replayed.
    // The token remains valid for the POST endpoints within its 20-min TTL so the
    // multi-step Sciotte flow (credentials -> 2FA -> OTP) can complete.
    resources
        .nonce_store
        .burn(&claims.jti)
        .await
        .map_err(|e| {
            warn!(jti = %claims.jti, error = %e, "Hosted-login page: link already consumed");
            render_error_response(
                "This login link has already been opened. For your security, each link can only be opened once. Please request a fresh link from your chat channel.",
            )
        })?;

    Ok(claims)
}

/// GET `/providers/sciotte/login?token=...`
///
/// Renders the hosted Sciotte login page. Validates the link-token up-front so
/// a user with an invalid/expired link sees the error page instead of a
/// broken-looking form.
pub(super) async fn handle_sciotte_hosted_login_page(
    State(resources): State<Arc<ServerResources>>,
    Query(query): Query<HostedLoginQuery>,
) -> Response {
    let Some(token) = query.token.as_deref() else {
        return render_error_response(
            "Missing login token. Please request a fresh link from your chat channel.",
        );
    };

    let claims = match validate_and_burn_link_token(&resources, token).await {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    // Target is clamped server-side at mint time; re-validate on render in case of tampering.
    let target = if matches!(claims.tgt.as_str(), "strava" | "garmin") {
        claims.tgt.as_str()
    } else {
        DEFAULT_TARGET
    };

    info!(
        user_id = %claims.sub,
        channel = %claims.channel,
        target = %target,
        "Rendered Sciotte hosted-login page"
    );

    Html(sciotte_hosted_templates::render_login_page(
        token,
        target,
        &claims.channel,
    ))
    .into_response()
}

/// GET `/providers/sciotte/success`
pub(super) async fn handle_sciotte_hosted_success_page(
    Query(query): Query<HostedSuccessQuery>,
) -> Response {
    let channel = query.channel.as_deref().unwrap_or("");
    let target = query.target.as_deref().unwrap_or(DEFAULT_TARGET);
    Html(sciotte_hosted_templates::render_success_page(
        channel, target,
    ))
    .into_response()
}

/// GET `/providers/sciotte/error?message=...`
pub(super) async fn handle_sciotte_hosted_error_page(
    Query(query): Query<HostedErrorQuery>,
) -> Response {
    let message = query.message.as_deref().unwrap_or(
        "Something went wrong connecting your account. Please request a fresh link from your chat channel.",
    );
    render_error_response(message)
}

fn render_error_response(message: &str) -> Response {
    Html(sciotte_hosted_templates::render_error_page(message)).into_response()
}
