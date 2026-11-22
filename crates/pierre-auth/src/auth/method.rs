// ABOUTME: How a request authenticated — JWT, API key, channel link or MCP token — and the impersonation a JWT carries
// ABOUTME: Decides which credentials may be exchanged for a session, and what a support session may not leave behind
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::permissions::impersonation::{BoundCredential, CallerCredential, CredentialKind};
use pierre_database::repositories::admin_audit::{
    AdminAuditRepository, AuditOutcome, AuditSource, NewAuditEvent,
};
use serde_json::json;
use tracing::{error, warn};
use uuid::Uuid;

use super::{AuthResult, Claims};
use crate::oauth2_server::first_party::names_third_party;

/// The refusal an impersonation token gets where a session is taken to sign
/// in: the consent screen's sign-in bridge
/// ([`super::AuthManager::validate_session_token`]).
pub(crate) const IMPERSONATION_NOT_A_SESSION: &str = "An impersonation token is not a session";

/// The refusal a support session gets for a write
/// [`AuthResult::refuse_impersonation`] guards. A `403`: the admin stays
/// signed in, and the sentence says who can do it instead.
pub const IMPERSONATION_WRITE_REFUSED: &str = "An admin acting as this account cannot leave a credential, grant or consent behind; only the account's owner can do this";

/// The audit-trail action of a write [`AuthResult::refuse_impersonation`]
/// refused.
pub const IMPERSONATION_REFUSED_ACTION: &str = "impersonation.write_refused";

/// The impersonation a `JWT` was minted for: an admin acting as the
/// token's user, through a live impersonation session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImpersonationGrant {
    /// The impersonation session the token belongs to; ending it revokes the
    /// token
    pub session_id: String,
    /// The admin acting as the user
    pub impersonator_id: Uuid,
    /// When the token expires (`exp`, seconds since the epoch): a token
    /// re-minted from it never outlives it
    pub expires_at: i64,
}

/// Authentication method used
#[derive(Debug, Clone)]
pub enum AuthMethod {
    /// `JWT` token authentication
    JwtToken {
        /// User tier for rate limiting
        tier: String,
        /// `Some` when the token was minted for an impersonation, whose
        /// session the authentication found live
        impersonation: Option<ImpersonationGrant>,
    },
    /// `API` key authentication
    ApiKey {
        /// `API` key `ID`
        key_id: String,
        /// `API` key tier
        tier: String,
    },
    /// Messaging channel link authentication
    ///
    /// Used when a webhook (`Telegram`, `WhatsApp`, `Discord`, `Slack`,
    /// `Messenger`) resolves an inbound message to a Pierre user via the
    /// `channel_link` row written at OTP/deep-link verification time. The
    /// presence of a valid link plus an active `UserStatus` constitutes the
    /// credential — same authority as a `JwtToken`, surfaced through the
    /// same [`AuthResult`](super::AuthResult) so the chat pipeline, command dispatcher, and
    /// tool execution layer cannot distinguish messaging from `HTTP` callers.
    ChannelLink {
        /// Channel identifier (`telegram`, `whatsapp`, `discord`, …).
        channel: String,
        /// Channel-side sender id (`Telegram` chat id, `WhatsApp` phone, …).
        channel_user_id: String,
        /// User tier for rate limiting (mirrors the `JWT` variant's field
        /// so [`crate::rate_limiting::calculate_jwt_rate_limit`] applies the
        /// user-tier policy uniformly).
        tier: String,
    },
    /// Personal MCP token authentication (`pmcp_`, carnet#788)
    ///
    /// The token an athlete mints in Settings and pastes into an MCP client
    /// (Claude Desktop, Cursor). It is that client's credential, not the
    /// athlete's own: accepted by `/mcp` only, carrying every delegable scope
    /// and never `admin`, and refused by every REST route, as a delegated
    /// OAuth grant is.
    McpToken {
        /// The token's row id in the owner's token list
        token_id: String,
        /// User tier for rate limiting (the owner's, as on a `JWT`)
        tier: String,
    },
}

impl AuthMethod {
    /// Refuse to mint an ordinary session from an API key, a personal MCP
    /// token or an impersonation token.
    ///
    /// A key or an MCP token is a caller outside Dravr's apps; a session token
    /// is the athlete in one of them. Exchanged for a session, a key would read
    /// as the athlete's own app on every chat and REST route, past the
    /// transport gate that keeps some providers' data off external callers
    /// (carnet#724). An impersonation token is an admin acting as the
    /// athlete for an hour, revoked when the impersonation ends and attributed
    /// to the admin on every request: exchanged for an ordinary session it
    /// would outlive that hour and the revocation, and read as the athlete
    /// acting alone (carnet#909).
    ///
    /// # Errors
    ///
    /// [`ErrorCode::PermissionDenied`](pierre_core::errors::ErrorCode::PermissionDenied)
    /// when the request was authenticated with an API key, an MCP token or an
    /// impersonation token.
    pub fn refuse_session_upgrade(&self) -> AppResult<()> {
        match self {
            Self::ApiKey { .. } => Err(AppError::new(
                ErrorCode::PermissionDenied,
                "An API key cannot be exchanged for a session; sign in to obtain one",
            )),
            Self::McpToken { .. } => Err(AppError::new(
                ErrorCode::PermissionDenied,
                "An MCP token cannot be exchanged for a session; sign in to obtain one",
            )),
            Self::JwtToken {
                impersonation: Some(_),
                ..
            } => Err(AppError::new(
                ErrorCode::PermissionDenied,
                "An impersonation token cannot be exchanged for a session",
            )),
            Self::JwtToken {
                impersonation: None,
                ..
            }
            | Self::ChannelLink { .. } => Ok(()),
        }
    }

    /// The impersonation the credential was minted for, if any.
    #[must_use]
    pub const fn impersonation(&self) -> Option<&ImpersonationGrant> {
        match self {
            Self::JwtToken { impersonation, .. } => impersonation.as_ref(),
            Self::ApiKey { .. } | Self::ChannelLink { .. } | Self::McpToken { .. } => None,
        }
    }

    /// Get a human-readable display name for the authentication method
    #[must_use]
    pub const fn display_name(&self) -> &str {
        match self {
            Self::JwtToken { .. } => "JWT Token",
            Self::ApiKey { .. } => "API Key",
            Self::ChannelLink { .. } => "Channel Link",
            Self::McpToken { .. } => "MCP Token",
        }
    }

    /// Get detailed information about the authentication method
    #[must_use]
    pub fn details(&self) -> String {
        match self {
            Self::JwtToken { tier, .. } => {
                format!("JWT Token (tier: {tier})")
            }
            Self::ApiKey { key_id, tier } => {
                format!("API Key (tier: {tier}, id: {key_id})")
            }
            Self::ChannelLink {
                channel,
                channel_user_id,
                tier,
            } => {
                format!(
                    "Channel Link (channel: {channel}, sender: {channel_user_id}, tier: {tier})"
                )
            }
            Self::McpToken { token_id, tier } => {
                format!("MCP Token (tier: {tier}, id: {token_id})")
            }
        }
    }
}

impl AuthResult {
    /// The credential a support session started through it is bound to
    /// (carnet#1511): the API key, or the OAuth grant a delegated token was
    /// minted under. `None` for any other credential, which never starts
    /// one.
    fn bound_credential(&self) -> Option<BoundCredential> {
        match &self.auth_method {
            AuthMethod::ApiKey { key_id, .. } => Some(BoundCredential::ApiKey(key_id.clone())),
            AuthMethod::JwtToken { .. } => self.grant_id.clone().map(BoundCredential::Grant),
            AuthMethod::McpToken { .. } | AuthMethod::ChannelLink { .. } => None,
        }
    }

    /// The credential the request came through, as the admin audit trail
    /// names it and a support session binds to it (carnet#1511): its kind,
    /// the OAuth client its token was issued to (from the token's own
    /// `client_id` claim), and what a support session started through it is
    /// bound to.
    #[must_use]
    pub fn caller_credential(&self) -> CallerCredential {
        let kind = match &self.auth_method {
            AuthMethod::ApiKey { .. } => CredentialKind::ApiKey,
            AuthMethod::McpToken { .. } => CredentialKind::McpToken,
            AuthMethod::ChannelLink { .. } => CredentialKind::ChannelLink,
            AuthMethod::JwtToken { .. } if names_third_party(self.client_id.as_deref()) => {
                CredentialKind::OAuthGrant
            }
            AuthMethod::JwtToken { .. } => CredentialKind::FirstPartySession,
        };
        CallerCredential {
            kind,
            client_id: self.client_id.clone(),
            binding: self.bound_credential(),
        }
    }

    /// Whether the credential is an outside client's acting for the user: an
    /// API key, an MCP token, or a token naming an OAuth application that is
    /// not one of Dravr's own apps ([`names_third_party`]).
    #[must_use]
    pub fn acts_for_outside_client(&self) -> bool {
        match &self.auth_method {
            AuthMethod::ApiKey { .. } | AuthMethod::McpToken { .. } => true,
            AuthMethod::JwtToken { .. } => names_third_party(self.client_id.as_deref()),
            AuthMethod::ChannelLink { .. } => false,
        }
    }

    /// Refuse an action that would leave behind, after a support session
    /// ends, something that authenticates as the athlete or widens who reads
    /// their data: a personal MCP token, an API key, an A2A client, an OAuth
    /// app, a password, a channel link, a push device, a provider connection,
    /// a stored LLM key or bot credential, a group invite or promotion, a
    /// conversation participant, a consent to share or to process, a billing
    /// session, a coach-access request, the account's deletion.
    ///
    /// An impersonation token is an admin acting as the athlete for an hour at
    /// most, revoked when the session ends. What it mints is not: a token or a
    /// link created under it would keep authenticating as the athlete, with no
    /// impersonator recorded, after the hour and the session's end
    /// (carnet#909, carnet#1634). The consent screen refuses the same token
    /// for the same reason (carnet#1560).
    ///
    /// The refusal is written to the admin audit trail, attributed to the
    /// admin who made the attempt, naming the athlete, the session and
    /// `action`. A trail write that fails is logged and never turns the
    /// refusal into a pass.
    ///
    /// # Errors
    ///
    /// `PermissionDenied` ([`IMPERSONATION_WRITE_REFUSED`]) when the request
    /// was authenticated with an impersonation token: a `403`, so the admin
    /// stays signed in.
    pub async fn refuse_impersonation(
        &self,
        audit: &dyn AdminAuditRepository,
        action: &str,
    ) -> AppResult<()> {
        refuse_support_session(
            self.auth_method.impersonation(),
            self.user_id,
            audit,
            action,
        )
        .await
    }

    /// [`AuthMethod::refuse_session_upgrade`], with an impersonation's
    /// refusal written to the admin audit trail as the attempt to `action`,
    /// attributed to the admin, as every refused support-session write is.
    ///
    /// # Errors
    ///
    /// As [`AuthMethod::refuse_session_upgrade`].
    pub async fn refuse_session_exchange(
        &self,
        audit: &dyn AdminAuditRepository,
        action: &str,
    ) -> AppResult<()> {
        if let Some(grant) = self.auth_method.impersonation() {
            record_impersonation_refusal(
                audit,
                &self.user_id.to_string(),
                grant.impersonator_id,
                &grant.session_id,
                action,
            )
            .await;
        }
        self.auth_method.refuse_session_upgrade()
    }
}

/// Refuse `action` on `user_id`'s account when `support` names the support
/// session it was attempted in, as [`AuthResult::refuse_impersonation`] does:
/// written to the admin audit trail, attributed to the admin.
///
/// For a caller that holds the support session apart from an [`AuthResult`]:
/// a chat turn's tool, or a stage that mints a link for the athlete.
///
/// # Errors
///
/// `PermissionDenied` ([`IMPERSONATION_WRITE_REFUSED`]) when `support` is
/// `Some`.
pub async fn refuse_support_session(
    support: Option<&ImpersonationGrant>,
    user_id: Uuid,
    audit: &dyn AdminAuditRepository,
    action: &str,
) -> AppResult<()> {
    let Some(grant) = support else {
        return Ok(());
    };
    record_impersonation_refusal(
        audit,
        &user_id.to_string(),
        grant.impersonator_id,
        &grant.session_id,
        action,
    )
    .await;
    Err(impersonation_write_refused())
}

/// Refuse `claims` as a session when they are an impersonation token's, and
/// write the attempt to `action` to the admin audit trail
/// ([`super::AuthManager::validate_session_token`],
/// [`super::AuthManager::validate_session_token_for_write`]). The refusal is
/// the one `refusal` builds.
pub(crate) async fn refuse_impersonated_session(
    claims: &Claims,
    audit: &dyn AdminAuditRepository,
    action: &str,
    refusal: impl FnOnce() -> AppError,
) -> AppResult<()> {
    let Some(session_id) = &claims.impersonation_session_id else {
        return Ok(());
    };
    if let Some(Ok(impersonator_id)) = claims.impersonator_id.as_deref().map(Uuid::parse_str) {
        record_impersonation_refusal(audit, &claims.sub, impersonator_id, session_id, action).await;
    } else {
        warn!(
            user_id = %claims.sub,
            session_id = %session_id,
            action,
            "Impersonation token naming no admin refused as a session"
        );
    }
    Err(refusal())
}

/// The refusal an impersonation token gets where a session is taken to sign
/// in: a `401`, as any credential that is not a session.
pub(crate) fn impersonation_not_a_session() -> AppError {
    AppError::auth_invalid(IMPERSONATION_NOT_A_SESSION)
}

/// The refusal an impersonation token gets where a session is taken to
/// write: the `403` every refused support-session write answers
/// ([`AuthResult::refuse_impersonation`]), so the admin stays signed in.
pub(crate) fn impersonation_write_refused() -> AppError {
    AppError::new(ErrorCode::PermissionDenied, IMPERSONATION_WRITE_REFUSED)
}

/// Log and write to the admin audit trail that the support session
/// `session_id`, opened by `impersonator_id` on `user_id`, was refused
/// `action`. A trail write that fails is logged; the caller refuses either
/// way.
pub(crate) async fn record_impersonation_refusal(
    audit: &dyn AdminAuditRepository,
    user_id: &str,
    impersonator_id: Uuid,
    session_id: &str,
    action: &str,
) {
    warn!(
        user_id,
        impersonator_id = %impersonator_id,
        session_id,
        action,
        "Impersonation token refused: a support session leaves no credential or grant behind"
    );
    let event = NewAuditEvent {
        action: IMPERSONATION_REFUSED_ACTION.to_owned(),
        summary: format!("Refused to {action} during a support session"),
        outcome: AuditOutcome::Refused,
        source: AuditSource::Console,
        actor_user_id: Some(impersonator_id),
        // The athlete's tenant, not the admin's: only a view of the whole
        // trail reads it, as every row reaching past the tenant its admin
        // acts in.
        tenant_id: None,
        ip_address: None,
        user_agent: None,
        metadata: json!({
            "target": { "user_id": user_id },
            "impersonation_session_id": session_id,
            "attempted": action,
        }),
    };
    if let Err(e) = audit.record(&event).await {
        error!(
            error = %e,
            action = IMPERSONATION_REFUSED_ACTION,
            "Failed to record a refused support-session write in the audit trail"
        );
    }
}
