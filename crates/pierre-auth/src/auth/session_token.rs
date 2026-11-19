// ABOUTME: Validates a token as the athlete's own first-party session, for a sign-in or for a write
// ABOUTME: Refuses a delegated token and an impersonation token: a 401 to sign in, an audited 403 to write
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::AppResult;
use pierre_database::repositories::admin_audit::AdminAuditRepository;

use super::{method, AuthManager, Claims};
use crate::admin::jwks::JwksManager;

impl AuthManager {
    /// Validate a token as the athlete's own session: a first-party credential,
    /// minted with the whole self grant, held by the athlete themselves.
    ///
    /// A delegated OAuth access token is refused however valid its signature
    /// and however wide its grant ([`Claims::is_session`]). It is a third
    /// party's credential, limited to what the athlete consented to, and a
    /// surface that takes a session — the OAuth consent screen above all,
    /// where a session approves new grants — would otherwise act on it with
    /// the athlete's whole authority.
    ///
    /// An impersonation token is refused too. It is an admin acting as the
    /// athlete for an hour at most, and only the auth middleware checks that
    /// its session is still live. Taken as a session here, it would approve
    /// grants and mint first-party sessions in the athlete's name that outlive
    /// the hour and the impersonation's end (carnet#909, carnet#1560). The
    /// refusal is written to the admin audit trail as the attempt to
    /// `action`, attributed to the admin, as every refused support-session
    /// write is ([`super::AuthResult::refuse_impersonation`]).
    ///
    /// # Errors
    ///
    /// Everything [`Self::validate_token`] refuses, a token that names a
    /// third-party client or whose grant is narrower than
    /// [`pierre_core::permissions::scopes::OAuthScope::self_grant`], and an impersonation token.
    pub async fn validate_session_token(
        &self,
        token: &str,
        jwks_manager: &JwksManager,
        audit: &dyn AdminAuditRepository,
        action: &str,
    ) -> AppResult<Claims> {
        let claims = Self::require_session(self.validate_token(token, jwks_manager)?)?;
        method::refuse_impersonated_session(
            &claims,
            audit,
            action,
            method::impersonation_not_a_session,
        )
        .await?;
        Ok(claims)
    }

    /// Validate a token as the athlete's own session for a write the route
    /// makes on it, as [`Self::validate_session_token`] does — for a route
    /// that verifies the session itself instead of through the auth
    /// middleware, such as the pending-account routes.
    ///
    /// An impersonation token is refused with the `403` every refused
    /// support-session write answers ([`super::AuthResult::refuse_impersonation`]),
    /// not a `401`: the web and mobile clients sign out on any `401`, and the
    /// admin stays signed in. The refusal is written to the admin audit trail
    /// as the attempt to `action`, attributed to the admin.
    ///
    /// # Errors
    ///
    /// Everything [`Self::validate_session_token`] refuses; an impersonation
    /// token is `PermissionDenied` ([`super::IMPERSONATION_WRITE_REFUSED`]).
    pub async fn validate_session_token_for_write(
        &self,
        token: &str,
        jwks_manager: &JwksManager,
        audit: &dyn AdminAuditRepository,
        action: &str,
    ) -> AppResult<Claims> {
        let claims = Self::require_session(self.validate_token(token, jwks_manager)?)?;
        method::refuse_impersonated_session(
            &claims,
            audit,
            action,
            method::impersonation_write_refused,
        )
        .await?;
        Ok(claims)
    }
}
