// ABOUTME: The explicit clear of a deleted account's tombstone, by the account that now holds the identity
// ABOUTME: A signed-in session, pending accounts included, clears its own email's or its proven provider account's

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `POST /api/user/import-tombstones/clear`.
//!
//! When an account is deleted, its provider accounts and its email are
//! tombstoned: provider data for them is refused, and a new sign-up with the
//! email waits Pending instead of being admitted
//! ([`pierre_services::import_tombstones`]). The person who now holds the
//! identity reopens it here, explicitly:
//!
//! - with `tombstone_id`, the tombstone a refusal named: an email tombstone
//!   when it names the account's own, proven address; a provider-account
//!   tombstone when this account was refused after completing that
//!   provider's authorization within the claim window;
//! - without, the live tombstone naming the account's own proven address.
//!
//! The call is idempotent: a tombstone already cleared is answered as it
//! stands. Clearing an email tombstone re-runs the admission decision, so an
//! account an auto-approval rule admits leaves Pending at once.
//!
//! The account may be Pending (that is the state an email tombstone holds it
//! in), so the session is verified here without the status gate the API
//! routes apply; a suspended account, an impersonation and every credential
//! but a first-party session are refused.

use axum::{
    extract::State,
    http::HeaderMap,
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::json;
use tracing::info;
use uuid::Uuid;

use pierre_auth::security::cookies::{auth_cookie_name, get_cookie_value};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{User, UserStatus};
use pierre_database::repositories::TombstoneKind;
use pierre_services::import_tombstones::clear_for_account;

use crate::AuthRoutesContext;

/// Body of `POST /api/user/import-tombstones/clear`.
#[derive(Debug, Default, Deserialize)]
pub struct ClearTombstoneRequest {
    /// The tombstone a refusal named; absent, the one naming the account's
    /// own email.
    #[serde(default)]
    pub tombstone_id: Option<String>,
}

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
async fn session_account(resources: &AuthRoutesContext, headers: &HeaderMap) -> AppResult<User> {
    let token = session_token(headers).ok_or_else(AppError::auth_required)?;
    let claims = resources
        .auth_manager
        .validate_session_token(&token, &resources.jwks_manager)?;
    if claims.impersonation_session_id.is_some() {
        return Err(AppError::auth_invalid(
            "An impersonation session cannot clear the account's tombstones",
        ));
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

/// `POST /api/user/import-tombstones/clear` — clear a tombstone whose
/// identity this account holds (see the module documentation).
///
/// Answers with every tombstone cleared (empty when the account's email
/// names none) and the account's status after the admission re-run.
///
/// # Errors
///
/// Returns an authentication error without a valid first-party session, a
/// suspended-account error, an invalid-input error when an email tombstone is
/// asked for before the account proved its address, a not-found error for a
/// tombstone that does not exist or whose identity this account does not
/// hold, and a database error when a read or the clear fails.
pub async fn handle_clear_import_tombstone(
    State(resources): State<AuthRoutesContext>,
    headers: HeaderMap,
    body: Option<Json<ClearTombstoneRequest>>,
) -> AppResult<Response> {
    let user = session_account(&resources, &headers).await?;
    let request = body.map(|Json(request)| request).unwrap_or_default();

    // An email is held by whoever proved the inbox, not by whoever typed it
    // at sign-up; a provider account's proof is its refused authorization.
    let wants_email = match request.tombstone_id.as_deref() {
        None => true,
        Some(id) => resources
            .repos
            .import_tombstones
            .get(id)
            .await?
            .is_some_and(|t| t.kind == TombstoneKind::Email),
    };
    if wants_email
        && !resources
            .repos
            .email_verification
            .is_verified(user.id)
            .await?
    {
        return Err(AppError::invalid_input(
            "Confirm your email address before clearing what a deleted account left on it",
        ));
    }

    let cleared = clear_for_account(
        resources.repos.import_tombstones.as_ref(),
        &user,
        request.tombstone_id.as_deref(),
    )
    .await?;

    let status = if cleared.iter().any(|t| t.kind == TombstoneKind::Email) {
        resources
            .account_service()
            .apply_approval_after_verification(user.id, false)
            .await?
    } else {
        user.user_status
    };
    info!(
        user_id = %user.id,
        cleared = cleared.len(),
        status = ?status,
        "Import tombstones cleared by the account"
    );
    Ok(Json(json!({
        "cleared": cleared
            .iter()
            .map(|t| json!({
                "tombstone_id": t.id,
                "kind": t.kind,
                "provider": t.provider,
                "deleted_at": t.created_at.to_rfc3339(),
                "cleared_at": t.cleared_at.map(|at| at.to_rfc3339()),
            }))
            .collect::<Vec<_>>(),
        "user_status": status,
    }))
    .into_response())
}
