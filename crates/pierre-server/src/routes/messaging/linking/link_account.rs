// ABOUTME: The account behind a messaging link page submission — sign in, or sign up through the one signup path
// ABOUTME: Returns the account id and status, or the words the link page shows when it refuses
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::net::IpAddr;

use pierre_auth::dto::auth::RegisterRequest;
use pierre_auth::oauth2_server::rate_limiting::OAuth2RateLimiter;
use pierre_auth::password::verify_password;
use pierre_core::constants::error_messages;
use pierre_core::errors::AppError;
use pierre_core::models::UserStatus;
use pierre_database::backends::UserRepository;
use pierre_services::auth::AuthService;
use tracing::{error, info, warn};
use uuid::Uuid;

use super::ChannelLinkAuthForm;
use crate::mcp::resources::ServerContext;

/// What the link page says to an unknown address or a wrong password, alike.
const INVALID_CREDENTIALS: &str = "Invalid email or password.";

/// What the link page says when the sign-in could not be checked at all.
const SIGN_IN_FAULT: &str = "An error occurred. Please try again.";

/// Why [`verify_credentials`] refused a sign-in.
enum CredentialRefusal {
    /// An unknown address or a wrong password: the caller's guess
    Invalid,
    /// The lookup or the verifier failed: not the caller's guess
    Fault,
}

impl CredentialRefusal {
    /// The words the link page shows for this refusal.
    fn message(&self) -> String {
        match self {
            Self::Invalid => INVALID_CREDENTIALS,
            Self::Fault => SIGN_IN_FAULT,
        }
        .to_owned()
    }
}

/// Resolve user identity from form data (login or register)
///
/// `client` is the address the sign-in came from, `None` only on a router
/// served without `ConnectInfo`.
pub(super) async fn resolve_user_from_form(
    resources: &ServerContext,
    form: &ChannelLinkAuthForm,
    client: Option<IpAddr>,
) -> Result<(Uuid, UserStatus), String> {
    match form.action.as_str() {
        "register" => register_user(resources, form).await,
        _ => authenticate_user(resources, &form.email, &form.password, client).await,
    }
}

/// Authenticate a user by email and password
///
/// Returns the user ID on success, or an error message string on failure.
/// The sign-in windows are read before the password is checked and a refused
/// password is counted in them (carnet#804), so this page is no faster a
/// guessing oracle than the app's own sign-in.
async fn authenticate_user(
    resources: &ServerContext,
    email: &str,
    password: &str,
    client: Option<IpAddr>,
) -> Result<(Uuid, UserStatus), String> {
    let limiter = &resources.auth.oauth2_rate_limiter;
    if let Some(refusal) = sign_in_refusal(limiter, client, email).await {
        return Err(refusal);
    }
    match verify_credentials(resources, email, password).await {
        Ok(account) => Ok(account),
        Err(refusal) => {
            if matches!(refusal, CredentialRefusal::Invalid) {
                limiter.count_failed_sign_in(client, email).await;
            }
            Err(refusal.message())
        }
    }
}

/// The words the page shows when the sign-in windows of `client` and `email`
/// refuse the attempt before its password is checked, or `None` to try it.
async fn sign_in_refusal(
    limiter: &OAuth2RateLimiter,
    client: Option<IpAddr>,
    email: &str,
) -> Option<String> {
    match limiter.sign_in_wait(client, email).await {
        Ok(None) => None,
        Ok(Some(retry_after)) => {
            warn!(retry_after, "Link page sign-in refused: too many attempts");
            Some("Too many sign-in attempts. Wait a minute, then try again.".to_owned())
        }
        Err(e) => {
            error!(error = %e, "Link page sign-in limiter could not read its windows");
            Some(SIGN_IN_FAULT.to_owned())
        }
    }
}

/// The account `email` and `password` sign in, or why they were refused.
async fn verify_credentials(
    resources: &ServerContext,
    email: &str,
    password: &str,
) -> Result<(Uuid, UserStatus), CredentialRefusal> {
    let user_repo: &dyn UserRepository = resources.common.repos.users.as_ref();

    let user = user_repo
        .get_by_email(email)
        .await
        .map_err(|e| {
            error!(error = %e, "Database error during link auth");
            CredentialRefusal::Fault
        })?
        .ok_or(CredentialRefusal::Invalid)?;

    let is_valid = verify_password(password.to_owned(), user.password_hash.clone())
        .await
        .map_err(|_| CredentialRefusal::Fault)?;

    if !is_valid {
        return Err(CredentialRefusal::Invalid);
    }

    Ok((user.id, user.user_status))
}

/// Register a new user through the link page and return their id and status.
///
/// The account is created by [`AuthService::register`], the same rules every
/// other signup follows — email and password validation, the approval posture
/// (pre-approved address, auto-approved domain, or pending admin approval) and
/// the personal tenant — so a code the bot handed out is not a way around them.
/// Returns a user-facing error on failure.
async fn register_user(
    resources: &ServerContext,
    form: &ChannelLinkAuthForm,
) -> Result<(Uuid, UserStatus), String> {
    let auth_service = AuthService::new(
        resources.auth.auth_manager.clone(),
        resources.auth.jwks_manager.clone(),
        resources.common.config.clone(),
        resources.data(),
    );
    let display_name = form
        .display_name
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(String::from);

    let registered = auth_service
        .register(RegisterRequest {
            email: form.email.clone(),
            password: form.password.clone(),
            display_name,
        })
        .await
        .map_err(|e| registration_refusal(&e))?;

    let user_id = Uuid::parse_str(&registered.user_id).map_err(|e| {
        error!(error = %e, "Registration returned an unreadable user id");
        "An error occurred. Please try again.".to_owned()
    })?;
    info!(user_id = %user_id, "User registered via channel link auth");
    Ok((user_id, registered.user_status))
}

/// The link page's words for a refused registration: the reason when it is
/// the caller's to fix, a generic retry otherwise.
fn registration_refusal(error: &AppError) -> String {
    let message = error.message.as_str();
    if message.ends_with(error_messages::USER_ALREADY_EXISTS) {
        "An account with this email already exists. Please log in instead.".to_owned()
    } else if message.ends_with(error_messages::INVALID_EMAIL_FORMAT) {
        "Please enter a valid email address.".to_owned()
    } else if message.ends_with(error_messages::PASSWORD_TOO_WEAK) {
        format!("{}.", error_messages::PASSWORD_TOO_WEAK)
    } else {
        error!(error = %error, "Failed to register user during link auth");
        "An error occurred. Please try again.".to_owned()
    }
}
