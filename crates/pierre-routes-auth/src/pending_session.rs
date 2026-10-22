// ABOUTME: The account behind a first-party session whatever its approval, for the routes a Pending account must reach
// ABOUTME: Shared by the tombstone clear and the signup-code redeem; refuses a suspended account and an impersonation

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

/// The account behind a first-party session, whatever its approval status
/// but suspended.
///
/// `action` names what the caller is about to do, for the refusal an
/// impersonation gets.
pub async fn session_account(
    resources: &AuthRoutesContext,
    headers: &HeaderMap,
    action: &str,
) -> AppResult<User> {
    let token = session_token(headers).ok_or_else(AppError::auth_required)?;
    let claims = resources
        .auth_manager
        .validate_session_token(&token, &resources.jwks_manager)?;
    if claims.impersonation_session_id.is_some() {
        return Err(AppError::auth_invalid(format!(
            "An impersonation session cannot {action}"
        )));
    }
    let user_id = Uuid::parse_str(&claims.sub)
        .map_err(|_| AppError::auth_invalid("The session names no account"))?;
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
