// ABOUTME: The account behind a first-party session whatever its approval, for the routes a Pending account must reach
// ABOUTME: Writes refuse a suspended account and an impersonation; the held-invite and invite-card reads only identify the reader

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A Pending account cannot pass the status gate the API routes apply, yet
//! the waiting screen must reach a few routes: clearing what a deleted
//! account left on its address, and redeeming a signup code. Those verify the
//! session here instead.

use axum::http::HeaderMap;
use uuid::Uuid;

use pierre_auth::security::cookies::{auth_cookie_name, get_cookie_value};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{User, UserStatus};

use crate::AuthRoutesContext;

/// The session token a request carries: the auth cookie, else a bearer
/// header.
fn session_token(headers: &HeaderMap) -> Option<String> {
    get_cookie_value(headers, &auth_cookie_name()).or_else(|| {
        headers
            .get("authorization")?
            .to_str()
            .ok()?
            .strip_prefix("Bearer ")
            .map(str::to_owned)
    })
}

/// The account `claims` name, refusing a suspended one.
async fn account_of(resources: &AuthRoutesContext, sub: &str) -> AppResult<User> {
    let user_id =
        Uuid::parse_str(sub).map_err(|_| AppError::auth_invalid("The session names no account"))?;
    let user = resources
        .repos
        .users
        .get_global(user_id)
        .await?
        .ok_or_else(|| AppError::auth_invalid("The session's account no longer exists"))?;
    if user.user_status == UserStatus::Suspended {
        return Err(AppError::account_suspended(
            "Your account has been suspended",
        ));
    }
    Ok(user)
}

/// The account behind a first-party session, for a read that changes nothing:
/// whatever its approval status but suspended. A support session reads as the
/// account it opened, as it reads everything else, and nothing is written to
/// the audit trail: no write is attempted.
///
/// # Errors
///
/// An authentication error without a valid first-party session, and a
/// suspended account's refusal.
pub(crate) async fn session_reader(
    resources: &AuthRoutesContext,
    headers: &HeaderMap,
) -> AppResult<User> {
    let token = session_token(headers).ok_or_else(AppError::auth_required)?;
    let claims = resources
        .auth_manager
        .validate_token(&token, &resources.jwks_manager)?;
    if !claims.is_session() {
        return Err(AppError::auth_invalid(
            "A delegated access token is not a session",
        ));
    }
    account_of(resources, &claims.sub).await
}

/// The account behind a first-party session, whatever its approval status
/// but suspended. An impersonation token is refused with the `403` every
/// support-session write answers, and the attempt to `action` written to the
/// admin audit trail.
pub(crate) async fn session_account(
    resources: &AuthRoutesContext,
    headers: &HeaderMap,
    action: &str,
) -> AppResult<User> {
    let token = session_token(headers).ok_or_else(AppError::auth_required)?;
    let claims = resources
        .auth_manager
        .validate_session_token_for_write(
            &token,
            &resources.jwks_manager,
            resources.repos.admin_audit.as_ref(),
            action,
        )
        .await?;
    account_of(resources, &claims.sub).await
}
