// ABOUTME: How a request authenticated — JWT, API key, channel link or MCP token — and the impersonation a JWT carries
// ABOUTME: Decides which credentials may be exchanged for an ordinary session (refuse_session_upgrade)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::{AppError, AppResult, ErrorCode};
use uuid::Uuid;

/// The impersonation a `JWT` was minted for: a super-admin acting as the
/// token's user, through a live impersonation session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImpersonationGrant {
    /// The impersonation session the token belongs to; ending it revokes the
    /// token
    pub session_id: String,
    /// The super-admin acting as the user
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
    /// same [`AuthResult`] so the chat pipeline, command dispatcher, and
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
    /// (carnet#724). An impersonation token is an operator acting as the
    /// athlete for an hour, revoked when the impersonation ends and attributed
    /// to the operator on every request: exchanged for an ordinary session it
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
