// ABOUTME: AuthService::sign_in_with_code: the first-party session a redeemed authorization code opens
// ABOUTME: Also the CLI's seeded E2E session (carnet#1201); both raise user.login naming their origin
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Opening a first-party session for an account already identified, by the
//! code the hosted login page issued or by `pierre-cli user mint-session`,
//! and the `user.login` notify event that names how it was opened.

use chrono::Utc;
use secrecy::SecretString;
use tracing::info;

use pierre_auth::dto::auth::LoginResponse;
use pierre_core::constants::limits;
use pierre_core::errors::{AppError, AppResult};

use super::AuthService;
use crate::analytics::cache_user_email;

/// How a first-party session was opened for an account already identified:
/// what [`AuthService::sign_in_with_code`]'s log line says and the `source`
/// its `user.login` notify event carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionOrigin {
    /// The app redeemed the code the hosted login page issued
    /// (`POST /oauth/token`, `grant_type=authorization_code`).
    AuthorizationCode,
    /// `pierre-cli user mint-session` opened it for a mobile E2E flow that
    /// starts signed in (carnet#1201); nobody typed a password.
    CliMintedSession,
}

impl SessionOrigin {
    /// The `source` the `user.login` notify event names this origin by.
    const fn as_str(self) -> &'static str {
        match self {
            Self::AuthorizationCode => "authorization_code",
            Self::CliMintedSession => "pierre_cli_mint_session",
        }
    }
}

/// Raise the `user.login` notify event for a session just opened.
///
/// The user and tenant ride on the event itself, not only on a request span,
/// so a session opened outside an HTTP handler (the CLI's) still names them.
/// The identity cache is warmed first so the `NotifyLayer` enricher can attach
/// the account's email to the event.
fn raise_user_login(response: &LoginResponse, origin: SessionOrigin) {
    cache_user_email(&response.user.user_id, &response.user.email);
    info!(
        target: "notify",
        event = "user.login",
        user_id = %response.user.user_id,
        tenant_id = response.user.tenant_id.as_deref(),
        source = origin.as_str(),
        "user authenticated"
    );
}

impl AuthService {
    /// Open the first-party session an authorization code signs in to.
    ///
    /// Dravr's web and mobile apps sign in on the hosted login page, which
    /// checks the password (or Google), and redeem the code it issues
    /// (carnet#787). `user_id` and `tenant_id` are what that code recorded,
    /// and `client_id` the app that redeemed it, which the session's JWT
    /// names in its `client_id` claim. The session is the one a sign-in has
    /// always opened: the self grant, 24 hours, the tenant the code names
    /// while the athlete still belongs to it. A suspended account is refused;
    /// a pending one signs in, so the app can show the approval it awaits.
    ///
    /// `origin` says how the account was identified: by a redeemed code, or
    /// by the CLI's seeded session for the mobile E2E lanes, which opens the
    /// very session a code would. The sign-in is logged and raises
    /// `user.login` naming it.
    ///
    /// # Errors
    /// Returns an error if the account is gone or suspended, or the session
    /// cannot be minted
    pub async fn sign_in_with_code(
        &self,
        user_id: uuid::Uuid,
        tenant_id: &str,
        client_id: &str,
        origin: SessionOrigin,
    ) -> AppResult<LoginResponse> {
        let repos = self.data.repos();
        let user = repos.users.get_global(user_id).await?.ok_or_else(|| {
            AppError::auth_invalid("The account this sign-in names no longer exists")
        })?;
        Self::reject_if_suspended(&user)?;

        let tenant_id = if self.is_tenant_member(user.id, tenant_id).await? {
            Some(tenant_id.to_owned())
        } else {
            self.ensure_user_has_tenant(&user).await?
        };
        let jwt_token = self
            .auth_manager
            .generate_client_session_token(
                &user,
                &self.jwks_manager,
                tenant_id.clone(),
                Some(client_id),
            )
            .map_err(|e| AppError::internal(format!("Failed to generate token: {e}")))?;
        let expires_at = Utc::now() + chrono::Duration::hours(limits::DEFAULT_SESSION_HOURS);

        repos
            .users
            .update_last_active(user.id)
            .await
            .map_err(|e| AppError::database(format!("Failed to update last active: {e}")))?;

        match origin {
            SessionOrigin::AuthorizationCode => {
                info!(user_id = %user.id, client_id, "User signed in with an authorization code");
            }
            SessionOrigin::CliMintedSession => {
                info!(user_id = %user.id, client_id, "User signed in with a session pierre-cli minted");
            }
        }

        let user_info = self.user_info(&user, tenant_id).await;
        let response = LoginResponse {
            jwt_token: Some(SecretString::from(jwt_token)),
            csrf_token: SecretString::default(), // Will be set by HTTP handler
            expires_at: expires_at.to_rfc3339(),
            user: user_info,
        };
        raise_user_login(&response, origin);
        Ok(response)
    }
}
