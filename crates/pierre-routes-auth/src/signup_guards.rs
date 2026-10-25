// ABOUTME: The edge guards of the public register form: a per-address window and the Turnstile bot check
// ABOUTME: Plus GET /api/auth/signup-config, which tells the form whether sign-up is open and which widget to render

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! What a stranger's sign-up passes before an account is written (carnet#1063).
//!
//! The operator's bounds (the kill switch and the 24-hour ceiling) live in
//! `pierre_services::signup_policy` and apply to every self-serve door. The
//! two guards here belong to the doors a browser reaches:
//!
//! - a window per client address ([`OAuth2Endpoint::SignUp`]), shared by the
//!   register form and the verification-mail resend;
//! - the Turnstile bot check, on the register form only, when its keys are
//!   configured.

use secrecy::SecretString;
use std::net::IpAddr;

use axum::extract::State;
use axum::http::HeaderMap;
use axum::Json;
use pierre_auth::dto::auth::RegisterRequest;
use pierre_auth::human_check::{HumanCheck, HumanCheckVerdict};
use pierre_auth::oauth2_server::rate_limiting::OAuth2RateLimiter;
use pierre_auth::rate_limiting::OAuth2Endpoint;
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_middleware::PeerAddress;
use pierre_services::signup_policy::signup_refusal;
use serde::{Deserialize, Serialize};
use tracing::{error, warn};

use crate::AuthRoutesContext;

/// What a sign-up past its address's window reads.
const TOO_MANY_SIGN_UPS: &str =
    "Too many sign-ups from this network. Wait a minute, then try again.";

/// The refusal of a sign-up whose bot check failed.
fn human_check_failed() -> AppError {
    AppError::new(
        ErrorCode::PermissionDenied,
        "We could not confirm you are not a bot. Complete the check again, then retry.",
    )
}

/// `POST /api/auth/register`'s body: the account, plus the token the form's
/// Turnstile widget minted. The token stays out of [`RegisterRequest`], which
/// the doors without a browser form (the link page, in-chat sign-up) share.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct PublicRegisterRequest {
    /// The account to create
    #[serde(flatten)]
    pub account: RegisterRequest,
    /// The Turnstile token, required when the bot check is configured
    #[serde(default)]
    pub turnstile_token: Option<SecretString>,
    /// A signup code that approves the account (carnet#1128). Like the
    /// Turnstile token it stays out of [`RegisterRequest`], so the doors
    /// without a form field for it are unchanged.
    #[serde(default)]
    pub signup_code: Option<SecretString>,
}

/// `GET /api/auth/signup-config`: what the register form needs before it is
/// shown.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct SignupConfigResponse {
    /// Whether a sign-up would be admitted right now: the kill switch is open
    /// and the 24-hour ceiling not reached
    pub open: bool,
    /// The Turnstile site key the form renders its widget with; `None` when
    /// the bot check is not configured
    pub turnstile_site_key: Option<String>,
}

/// The client a request from `peer` carrying `headers` came from, through the
/// trusted proxies; `None` on a router served without `ConnectInfo`.
pub(crate) fn client_address(
    limiter: &OAuth2RateLimiter,
    peer: PeerAddress,
    headers: &HeaderMap,
) -> Option<IpAddr> {
    peer.0.map(|ip| limiter.client_address(ip, headers))
}

/// Count one sign-up (or resend) from `client` in its address's window.
///
/// A router served without `ConnectInfo` has no address to key a window on,
/// and admits the request; the deployed server always has one.
///
/// # Errors
///
/// Returns a rate-limit error with its `Retry-After` past the window's
/// allowance, and a resource-unavailable error when the limiter cannot count
/// the request: an uncounted sign-up is refused rather than let through.
pub async fn meter_sign_up(limiter: &OAuth2RateLimiter, client: Option<IpAddr>) -> AppResult<()> {
    let Some(client) = client else {
        return Ok(());
    };
    let status = limiter
        .check_rate_limit(OAuth2Endpoint::SignUp, client)
        .await
        .map_err(|failure| {
            error!(error = %failure, "Sign-up limiter could not count the request");
            AppError::resource_unavailable("Sign-up is temporarily unavailable; retry later")
        })?;
    if status.is_limited {
        warn!(
            limit = status.limit,
            "Sign-up refused: too many from one address"
        );
        return Err(
            AppError::new(ErrorCode::RateLimitExceeded, TOO_MANY_SIGN_UPS)
                .with_retry_after(u64::from(status.retry_after_seconds.unwrap_or(1))),
        );
    }
    Ok(())
}

/// Pass the bot check, when one is configured.
///
/// Cloudflare out of reach admits the sign-up: the per-address window and
/// the operator's ceiling still bound it, and an outage at Cloudflare must not
/// close sign-up by itself.
///
/// # Errors
///
/// Returns a permission-denied error ([`human_check_failed`]) when the
/// token is missing or Cloudflare refused it.
pub(crate) async fn pass_human_check(
    check: Option<&dyn HumanCheck>,
    token: Option<&str>,
    client: Option<IpAddr>,
) -> AppResult<()> {
    let Some(check) = check else {
        return Ok(());
    };
    let Some(token) = token.filter(|t| !t.is_empty()) else {
        warn!("Sign-up refused: no bot-check token");
        return Err(human_check_failed());
    };
    match check.verify(token, client).await {
        HumanCheckVerdict::Passed => Ok(()),
        HumanCheckVerdict::Failed => {
            warn!("Sign-up refused: the bot check failed");
            Err(human_check_failed())
        }
        HumanCheckVerdict::Unavailable => {
            error!("Bot check unavailable; admitting the sign-up on the other guards");
            Ok(())
        }
    }
}

/// Tell the register form whether sign-up is open and which widget to render.
///
/// `open` is the operator's bounds as a sign-up would meet them now
/// ([`signup_refusal`], the decision the sign-up itself takes).
pub(crate) async fn handle_signup_config(
    State(resources): State<AuthRoutesContext>,
) -> Json<SignupConfigResponse> {
    let open = signup_refusal(resources.data.database(), resources.repos.users.as_ref())
        .await
        .is_none();
    Json(SignupConfigResponse {
        open,
        turnstile_site_key: resources
            .human_check
            .as_ref()
            .map(|check| check.site_key().to_owned()),
    })
}
