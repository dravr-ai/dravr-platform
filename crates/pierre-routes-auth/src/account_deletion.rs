// ABOUTME: Self-serve account deletion — the athlete previews then deletes their own account, re-proving who they are
// ABOUTME: Runs the operator removal path (provider grants revoked upstream, every owned row gone in one transaction)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `GET /api/user/account-deletion` and `POST /api/user/account-deletion`.
//!
//! The athlete deletes their own account from the privacy pane on web and
//! mobile (App Store guideline 5.1.1(v) requires the in-app path). This is
//! the same removal an operator runs ([`user_removal::remove_user`]), so a
//! self-serve delete disconnects every provider through the disconnect
//! chokepoint — the grant is revoked at the provider and every row the
//! provider contributed is purged — then deletes the account in one
//! transaction that clears every row the user owns and refuses to commit
//! while one survives. Session refresh tokens, device tokens, messaging
//! sessions and channel links, and coaching-group memberships go with it.
//!
//! The request must come from a signed-in session (never an API key or a
//! messaging channel link), name the account's email as a deliberate
//! confirmation, and carry the account password unless the account signs in
//! only through Google or Apple. Operator accounts are removed by another
//! operator, never by themselves. Anything another person relies on — a group
//! the user owns or coaches, an invite they created, a tenant others belong
//! to, an agent others use, a subscription the billing provider may still
//! charge — refuses the delete with 409 naming each, touching nothing, so the
//! athlete can resolve them and retry.

use std::collections::BTreeSet;

use axum::{
    extract::State,
    http::{header::RETRY_AFTER, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::{field::Empty, info, warn, Span};
use uuid::Uuid;

use pierre_auth::auth::AuthMethod;
use pierre_auth::password::verify_password;
use pierre_auth::security::cookies::clear_auth_cookie;
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::{normalize_email, User, UserReference, FEDERATED_ONLY_PASSWORD_HASH};
use pierre_core::permissions::UserRole;
use pierre_services::oauth_flow::OAuthService;
use pierre_services::provider_revocation::DisconnectReason;
use pierre_services::user_removal::{self, UserRemoval};

use crate::password_attempts::meter_password_attempt;
use crate::AuthRoutesContext;

/// Why a deletion request was refused, as the `error` field of the refusal
/// body: the clients pick their localized message by it.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum DeletionRefusal {
    /// The typed confirmation is not the account's email.
    EmailMismatch,
    /// The account has a password and the request carried none.
    PasswordRequired,
    /// The password does not match the account's.
    PasswordIncorrect,
    /// Rows other people rely on reference the user; nothing was touched.
    Blocked,
    /// The account spent its window of password attempts.
    TooManyAttempts,
}

/// Body of `POST /api/user/account-deletion`.
#[derive(Debug, Deserialize)]
pub struct DeleteAccountRequest {
    /// The account's email, typed by the athlete as the confirmation.
    pub confirm_email: String,
    /// The account password; required unless the account signs in only
    /// through a federated identity.
    #[serde(default)]
    pub password: Option<String>,
}

/// Body of `GET /api/user/account-deletion`: what a delete would ask for and
/// what it would do, read before the athlete confirms.
#[derive(Debug, Serialize)]
pub struct AccountDeletionPreview {
    /// The email the confirmation must match.
    pub email: String,
    /// Whether the delete asks for the account password.
    pub requires_password: bool,
    /// Providers the delete disconnects, by user-facing name, deduplicated.
    pub providers: Vec<String>,
    /// What refuses the delete until the athlete resolves it; empty when the
    /// account can be deleted now.
    pub blockers: Vec<UserReference>,
}

/// The signed-in user behind a deletion request, refusing every credential
/// but a first-party session and every operator account.
async fn session_user(resources: &AuthRoutesContext, headers: &HeaderMap) -> AppResult<User> {
    let auth = resources
        .auth_middleware
        .authenticate_request_with_headers(headers)
        .await?;
    if !matches!(auth.auth_method, AuthMethod::JwtToken { .. }) {
        return Err(AppError::new(
            ErrorCode::PermissionDenied,
            "An account can only be deleted from a signed-in session",
        ));
    }
    Span::current().record("user_id", auth.user_id.to_string());
    let user = resources
        .repos
        .users
        .get_global(auth.user_id)
        .await?
        .ok_or_else(|| AppError::not_found(format!("User {}", auth.user_id)))?;
    if user.is_admin || user.role != UserRole::User {
        return Err(AppError::new(
            ErrorCode::PermissionDenied,
            "An operator account is removed by another operator",
        ));
    }
    Ok(user)
}

/// Whether the account holds a password it can be re-proven with.
fn has_password(user: &User) -> bool {
    user.password_hash != FEDERATED_ONLY_PASSWORD_HASH
}

/// A refusal the clients localize by its `error` code.
fn refusal(status: StatusCode, reason: DeletionRefusal, message: &str) -> Response {
    (status, Json(json!({ "error": reason, "message": message }))).into_response()
}

/// `GET /api/user/account-deletion` — what deleting the signed-in account
/// asks for, which providers it disconnects, and what blocks it.
///
/// # Errors
///
/// Returns an authentication error without a session, a permission error for
/// an API key, a channel link or an operator account, and a database error
/// from the reads.
#[tracing::instrument(
    skip(resources, headers),
    fields(route = "account_deletion_preview", user_id = Empty)
)]
pub async fn handle_account_deletion_preview(
    State(resources): State<AuthRoutesContext>,
    headers: HeaderMap,
) -> AppResult<Response> {
    let user = session_user(&resources, &headers).await?;
    let providers: BTreeSet<String> = user_removal::held_providers(&resources.repos, user.id)
        .await?
        .into_iter()
        .map(|held| held.provider)
        .collect();
    let blockers = resources.repos.users.deletion_blockers(user.id).await?;
    let requires_password = has_password(&user);
    Ok((
        StatusCode::OK,
        Json(AccountDeletionPreview {
            email: user.email,
            requires_password,
            providers: providers.into_iter().collect(),
            blockers,
        }),
    )
        .into_response())
}

/// `POST /api/user/account-deletion` — delete the signed-in account.
///
/// Refuses with 400 when `confirm_email` is not the account's email, with
/// 403 when the account's password is missing or wrong, with 429 once the
/// account has spent its window of password attempts (shared with
/// `change-password`), and with 409 naming
/// each blocker while rows other people rely on reference the user; nothing
/// is touched then. Otherwise every provider is disconnected (revoked at the
/// provider) and the account is deleted with every row it owns; the auth
/// cookie is cleared and every device session ends with the account row.
///
/// LIMITATION(registre#798): `handle_delete_account` leaves the Firebase identity
/// (`users.firebase_uid`) at Google; the server holds no Firebase Admin credential, so
/// only the client deletes it, and only while its Firebase session is recent.
///
/// # Errors
///
/// Returns an authentication error without a session, a permission error for
/// an API key, a channel link or an operator account, a database error from
/// the reads that precede any change, and the failing step's error when a
/// disconnect or the account delete fails part-way; the message then names
/// the providers already disconnected, since their grants stay withdrawn.
#[tracing::instrument(
    skip(resources, headers, request),
    fields(route = "account_deletion", user_id = Empty)
)]
pub async fn handle_delete_account(
    State(resources): State<AuthRoutesContext>,
    headers: HeaderMap,
    Json(request): Json<DeleteAccountRequest>,
) -> AppResult<Response> {
    let user = session_user(&resources, &headers).await?;
    let user_id: Uuid = user.id;

    if normalize_email(&request.confirm_email) != normalize_email(&user.email) {
        return Ok(refusal(
            StatusCode::BAD_REQUEST,
            DeletionRefusal::EmailMismatch,
            "The email you typed does not match this account",
        ));
    }
    if has_password(&user) {
        let Some(password) = request.password.filter(|p| !p.is_empty()) else {
            return Ok(refusal(
                StatusCode::FORBIDDEN,
                DeletionRefusal::PasswordRequired,
                "Enter your password to delete this account",
            ));
        };
        if let Some(retry_after) = meter_password_attempt(&resources.rate_limiter, user_id).await? {
            let mut response = refusal(
                StatusCode::TOO_MANY_REQUESTS,
                DeletionRefusal::TooManyAttempts,
                "Too many password attempts; retry later",
            );
            response
                .headers_mut()
                .insert(RETRY_AFTER, HeaderValue::from(retry_after));
            return Ok(response);
        }
        if !verify_password(password, user.password_hash.clone()).await? {
            warn!(user_id = %user_id, "Account deletion refused: wrong password");
            return Ok(refusal(
                StatusCode::FORBIDDEN,
                DeletionRefusal::PasswordIncorrect,
                "The password is incorrect",
            ));
        }
    }

    let oauth_service = OAuthService::new(resources.data.clone(), resources.config.clone());
    let outcome = user_removal::remove_user(
        &resources.repos,
        Some(&oauth_service),
        user_id,
        DisconnectReason::Athlete,
    )
    .await?;

    let report = match outcome {
        UserRemoval::Blocked(blockers) => {
            info!(
                user_id = %user_id,
                blockers = blockers.len(),
                "Account deletion refused: rows other people rely on reference the user"
            );
            return Ok((
                StatusCode::CONFLICT,
                Json(json!({
                    "error": DeletionRefusal::Blocked,
                    "message": user_removal::blockers_message(&blockers),
                    "blockers": blockers,
                })),
            )
                .into_response());
        }
        UserRemoval::Interrupted(interruption) => {
            let cause = interruption.error.message.clone();
            return Err(AppError::new(
                interruption.error.code,
                interruption.describe(&cause),
            ));
        }
        UserRemoval::Removed(report) => report,
    };

    info!(
        user_id = %user_id,
        disconnected = report.disconnected.len(),
        not_revocable = report.not_revocable.len(),
        memberships_removed = report.memberships_removed,
        "Account deleted by its owner"
    );

    let mut response_headers = HeaderMap::new();
    clear_auth_cookie(&mut response_headers);
    let disconnected: Vec<&str> = report
        .disconnected
        .iter()
        .map(|d| d.provider.as_str())
        .collect();
    Ok((
        StatusCode::OK,
        response_headers,
        Json(json!({
            "message": "Your account and its data were deleted",
            "disconnected_providers": disconnected,
        })),
    )
        .into_response())
}
