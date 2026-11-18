// ABOUTME: POST /api/user/signup-code/redeem — a Pending account enters a signup code (or a group invite) from the waiting screen
// ABOUTME: Also PUT /api/user/group-invite, the per-address meter every code attempt passes, and the coach-group join a coach's code carries

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Redeeming a signup code from the waiting screen.
//!
//! This is the door for every account that had nowhere to type a code when
//! it was created: a first Google sign-in, the messaging link page, in-chat
//! sign-up, and anyone who registered before receiving one. The account is
//! Pending, so the session is verified without the status gate
//! ([`crate::pending_session`]).

use std::net::IpAddr;

use axum::{
    extract::State,
    http::HeaderMap,
    response::{IntoResponse, Response},
    Json,
};
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use serde_json::json;
use tracing::{error, info, warn};
use uuid::Uuid;

use pierre_auth::oauth2_server::rate_limiting::OAuth2RateLimiter;
use pierre_auth::rate_limiting::OAuth2Endpoint;
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_middleware::PeerAddress;
use pierre_services::auth::RedeemedSignupCode;
use pierre_services::group_invite_hold;

use crate::pending_session::session_account;
use crate::signup_guards::client_address;
use crate::AuthRoutesContext;

/// What a client entering too many codes from one address reads.
const TOO_MANY_CODES: &str = "Too many codes tried from this network. Please wait a minute.";

/// Body of `POST /api/user/signup-code/redeem`.
#[derive(Deserialize)]
pub(crate) struct RedeemSignupCodeRequest {
    /// The code as the person typed it
    pub code: SecretString,
}

/// Count one code attempt from `client` in its address's window: a code is
/// a bearer secret, so entering one is metered like a password guess.
///
/// # Errors
///
/// Returns a rate-limit error past the window's allowance, and a
/// resource-unavailable error when the limiter cannot count the attempt.
pub(crate) async fn meter_signup_code(
    limiter: &OAuth2RateLimiter,
    client: Option<IpAddr>,
) -> AppResult<()> {
    let Some(client) = client else {
        return Ok(());
    };
    let status = limiter
        .check_rate_limit(OAuth2Endpoint::SignupCode, client)
        .await
        .map_err(|failure| {
            error!(error = %failure, "Signup-code limiter could not count the attempt");
            AppError::resource_unavailable("Codes cannot be checked right now; retry later")
        })?;
    if status.is_limited {
        warn!(
            limit = status.limit,
            "Signup code refused: too many from one address"
        );
        return Err(AppError::new(ErrorCode::RateLimitExceeded, TOO_MANY_CODES)
            .with_retry_after(u64::from(status.retry_after_seconds.unwrap_or(1))));
    }
    Ok(())
}

/// Join an account a coach's code approved to the coach's group. Returns
/// whether it joined, `None` for an admin's code. A join that fails (the group
/// is full or archived since) leaves the account as the code made it.
pub(crate) async fn join_coach_group(
    resources: &AuthRoutesContext,
    redeemed: &RedeemedSignupCode,
    user_id: Uuid,
) -> Option<bool> {
    let invite = redeemed.coach_invite.as_ref()?;
    let Some(groups) = resources.group_service.as_ref() else {
        warn!(user_id = %user_id, "A coach's signup code was redeemed in a build without group coaching");
        return Some(false);
    };
    match groups
        .join_group_from_signup_code(&invite.id.to_string(), user_id)
        .await
    {
        Ok(_) => Some(true),
        Err(e) => {
            warn!(user_id = %user_id, group_id = %invite.group_id, error = %e, "Signup code redeemed but the coach's group could not be joined");
            Some(false)
        }
    }
}

/// Hold the group invite a `/groups/join/<code>` link carried for an account
/// registration has just created (carnet#1648). A code no one could hold is
/// logged and dropped: the link never refuses the sign-up it came with.
pub(crate) async fn hold_link_invite(
    resources: &AuthRoutesContext,
    user_id: Uuid,
    code: &SecretString,
) {
    if let Err(e) = group_invite_hold::hold(&resources.repos, user_id, code.expose_secret()).await {
        warn!(user_id = %user_id, error = %e, "The sign-up's group invite link could not be held; the account joins no group");
    }
}

/// Body of `PUT /api/user/group-invite`.
#[derive(Deserialize)]
pub(crate) struct HoldGroupInviteRequest {
    /// The code the `/groups/join/<code>` link carried
    pub code: SecretString,
}

/// `PUT /api/user/group-invite` — hold the group invite a link carried for
/// the signed-in account, Pending or active (carnet#1648).
///
/// The app sends it right after a sign-in that started from a
/// `/groups/join/<code>` link, before anything can reload the page, and
/// forgets its own copy once this answers. The account joins the group once
/// it is active (`POST /api/chat/groups/join`).
///
/// # Errors
///
/// Returns an authentication error without a valid first-party session, a
/// rate-limit error past the per-address allowance, the uniform invalid-code
/// error for a code no one could hold, and the repository's error when the
/// hold cannot be written.
pub(crate) async fn handle_hold_group_invite(
    State(resources): State<AuthRoutesContext>,
    peer: PeerAddress,
    headers: HeaderMap,
    Json(body): Json<HoldGroupInviteRequest>,
) -> AppResult<Response> {
    let user = session_account(&resources, &headers, "hold a group invite").await?;
    let client = client_address(&resources.rate_limiter, peer, &headers);
    meter_signup_code(&resources.rate_limiter, client).await?;
    let invite =
        group_invite_hold::hold(&resources.repos, user.id, body.code.expose_secret()).await?;
    Ok(Json(json!({ "held": true, "kind": invite.kind })).into_response())
}

/// `POST /api/user/signup-code/redeem` — redeem a signup code for the
/// signed-in Pending account.
///
/// Answers with the account's status after the redemption: `active` when
/// the code activated it, `pending` while it waits on the address, and
/// whether a coach's code joined its group. A group invite that approves
/// nothing, entered here, is held instead (carnet#1648): the account joins
/// the group once it is active; `group_invite_held` says so.
///
/// # Errors
///
/// Returns an authentication error without a valid first-party session, a
/// rate-limit error past the per-address allowance, and every error
/// [`pierre_services::auth::AuthService::redeem_signup_code`] returns.
pub(crate) async fn handle_redeem_signup_code(
    State(resources): State<AuthRoutesContext>,
    peer: PeerAddress,
    headers: HeaderMap,
    Json(body): Json<RedeemSignupCodeRequest>,
) -> AppResult<Response> {
    let user = session_account(&resources, &headers, "redeem a signup code").await?;
    let client = client_address(&resources.rate_limiter, peer, &headers);
    meter_signup_code(&resources.rate_limiter, client).await?;

    if group_invite_hold::names_plain_group_invite(&resources.repos, body.code.expose_secret())
        .await?
    {
        group_invite_hold::hold(&resources.repos, user.id, body.code.expose_secret()).await?;
        info!(user_id = %user.id, "Group invite held from the waiting screen");
        return Ok(Json(json!({
            "user_status": user.user_status,
            "group_joined": null,
            "group_invite_held": true,
        }))
        .into_response());
    }

    let redeemed = resources
        .account_service()
        .redeem_signup_code(user.id, &body.code)
        .await?;
    let group_joined = join_coach_group(&resources, &redeemed, user.id).await;
    info!(user_id = %user.id, status = ?redeemed.status, "Signup code redeemed from the waiting screen");

    Ok(Json(json!({
        "user_status": redeemed.status,
        "group_joined": group_joined,
    }))
    .into_response())
}
