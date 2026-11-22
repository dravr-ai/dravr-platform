// ABOUTME: Impersonation system for admins to act as other users
// ABOUTME: Provides audit logging and session management for impersonation
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::errors::{AppError, AppResult, ErrorCode};
use crate::models::User;

/// Impersonation session record for audit trail
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImpersonationSession {
    /// Unique session identifier
    pub id: String,
    /// Admin performing the impersonation
    pub impersonator_id: Uuid,
    /// User being impersonated
    pub target_user_id: Uuid,
    /// Reason for impersonation (for audit)
    pub reason: Option<String>,
    /// When impersonation started
    pub started_at: DateTime<Utc>,
    /// When impersonation ended (None if still active)
    pub ended_at: Option<DateTime<Utc>>,
    /// Whether session is currently active
    pub is_active: bool,
    /// Record creation timestamp
    pub created_at: DateTime<Utc>,
    /// The OAuth grant a support session through an admin's assistant was
    /// started with, and the only one it changes: the refresh-token family
    /// its access token was minted in ([`BoundCredential::Grant`]). `None`
    /// for a session started with an API key or on the web (carnet#1511).
    #[serde(default)]
    pub grant_id: Option<String>,
    /// The admin's API key a support session through their assistant was
    /// started with, and the only one it changes
    /// ([`BoundCredential::ApiKey`]). `None` for a session started over
    /// OAuth or on the web.
    #[serde(default)]
    pub api_key_id: Option<String>,
}

/// The one credential a support session through an admin's assistant acts
/// through.
///
/// JF, Q21: "the session is bound to the grant or API key that started
/// it". Any other credential of the same admin, another key or another
/// grant of the same application, acts as the admin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoundCredential {
    /// An OAuth grant: the refresh-token family the access token was minted
    /// in. One authorization starts one family, so a second connection of
    /// the same application is another grant.
    Grant(String),
    /// One of the admin's API keys, by id.
    ApiKey(String),
}

/// What kind of credential a call came through, as the admin audit trail
/// names it (carnet#1511).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialKind {
    /// A third-party OAuth application's access token, delegated at consent.
    OAuthGrant,
    /// One of the user's API keys.
    ApiKey,
    /// A personal MCP token (`pmcp_`) the user minted for an MCP client.
    McpToken,
    /// A session of one of Dravr's own apps, or a token naming no client.
    FirstPartySession,
    /// An A2A machine client's client-credentials token, acting as the user
    /// who registered it.
    A2aClientCredentials,
    /// A linked messaging channel.
    ChannelLink,
}

impl CredentialKind {
    /// The name the audit trail records.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OAuthGrant => "oauth_grant",
            Self::ApiKey => "api_key",
            Self::McpToken => "mcp_token",
            Self::FirstPartySession => "first_party_session",
            Self::A2aClientCredentials => "a2a_client_credentials",
            Self::ChannelLink => "channel_link",
        }
    }
}

/// The credential an external call came through: what the admin audit trail
/// names of it, and what a support session binds to.
///
/// Ids and kinds only, never a token or a secret: the client is the OAuth
/// `client_id` the authorization server issued the token to (or the A2A
/// client a client-credentials token names), never what the caller says
/// about itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallerCredential {
    /// The kind of credential.
    pub kind: CredentialKind,
    /// The client the credential was issued to; `None` for an API key, a
    /// personal MCP token, a channel link, and a session no client signed
    /// in to.
    pub client_id: Option<String>,
    /// What a support session started through this credential is bound to;
    /// `None` for a credential that never starts one.
    pub binding: Option<BoundCredential>,
}

impl CallerCredential {
    /// The kind of credential a support session bound to `session` came
    /// through, read from the session itself; `None` for a web session.
    #[must_use]
    pub fn kind_of_session(session: &ImpersonationSession) -> Option<CredentialKind> {
        session.bound_credential().map(|bound| match bound {
            BoundCredential::Grant(_) => CredentialKind::OAuthGrant,
            BoundCredential::ApiKey(_) => CredentialKind::ApiKey,
        })
    }
}

/// `audit_events.action` of a support session an assistant started.
pub const STARTED_ACTION: &str = "impersonation.started";

/// `audit_events.action` of a support session an assistant ended, or that
/// ended with the credential it was bound to.
pub const ENDED_ACTION: &str = "impersonation.ended";

/// `audit_events.action` of a support session ended by its hour.
pub const EXPIRED_ACTION: &str = "impersonation.expired";

/// How long a support session acts as its user from its start, never
/// renewed: on the web console (the impersonation token's life) and through
/// an admin's assistant alike.
pub const SESSION_MINUTES: i64 = 60;

/// Whether `impersonator` may act as `target`, on every door a session
/// starts from (the admin console and an admin's assistant).
///
/// # Errors
///
/// Returns an invalid-input error when `target` is the impersonator, and a
/// permission-denied error when `target` is an admin: acting as one would
/// lend its every power to a session the audit trail attributes to someone
/// else.
pub fn check_target(impersonator: Uuid, target: &User) -> AppResult<()> {
    if target.id == impersonator {
        return Err(AppError::invalid_input("Cannot impersonate yourself"));
    }
    if target.role.is_admin() {
        return Err(AppError::new(
            ErrorCode::PermissionDenied,
            "Cannot impersonate another admin",
        ));
    }
    Ok(())
}

impl ImpersonationSession {
    /// Create a new impersonation session
    #[must_use]
    pub fn new(impersonator_id: Uuid, target_user_id: Uuid, reason: Option<String>) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4().to_string(),
            impersonator_id,
            target_user_id,
            reason,
            started_at: now,
            ended_at: None,
            is_active: true,
            created_at: now,
            grant_id: None,
            api_key_id: None,
        }
    }

    /// A session an admin's assistant starts with `credential`, the only
    /// credential it changes ([`Self::bound_credential`]).
    #[must_use]
    pub fn bound_to(
        impersonator_id: Uuid,
        target_user_id: Uuid,
        reason: Option<String>,
        credential: &BoundCredential,
    ) -> Self {
        let session = Self::new(impersonator_id, target_user_id, reason);
        match credential {
            BoundCredential::Grant(id) => Self {
                grant_id: Some(id.clone()),
                ..session
            },
            BoundCredential::ApiKey(id) => Self {
                api_key_id: Some(id.clone()),
                ..session
            },
        }
    }

    /// The credential the session is bound to; `None` for a web session.
    #[must_use]
    pub fn bound_credential(&self) -> Option<BoundCredential> {
        self.grant_id
            .clone()
            .map(BoundCredential::Grant)
            .or_else(|| self.api_key_id.clone().map(BoundCredential::ApiKey))
    }

    /// End the impersonation session
    pub fn end(&mut self) {
        self.ended_at = Some(Utc::now());
        self.is_active = false;
    }

    /// Get duration of impersonation in seconds
    #[must_use]
    pub fn duration_seconds(&self) -> i64 {
        let end = self.ended_at.unwrap_or_else(Utc::now);
        (end - self.started_at).num_seconds()
    }
}
