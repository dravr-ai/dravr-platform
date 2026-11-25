// ABOUTME: OAuth 2.0 server persistence models for clients, auth codes, tokens, and state
// ABOUTME: Used by DatabaseProvider trait for OAuth 2.0 server storage operations
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::fmt;

use dravr_tronc::dates::Timestamp;
use secrecy::SecretString;
use uuid::Uuid;

/// Stored OAuth 2.0 Client
#[derive(Debug, Clone)]
pub struct OAuth2Client {
    /// Internal database ID
    pub id: String,
    /// OAuth 2.0 client identifier
    pub client_id: String,
    /// Hashed client secret for authentication
    pub client_secret_hash: String,
    /// Registered redirect URIs for authorization code flow
    pub redirect_uris: Vec<String>,
    /// Allowed OAuth 2.0 grant types (`authorization_code`, `client_credentials`, etc.)
    pub grant_types: Vec<String>,
    /// Allowed OAuth 2.0 response types (code, token, etc.)
    pub response_types: Vec<String>,
    /// Human-readable client name
    pub client_name: Option<String>,
    /// Client's home page URL
    pub client_uri: Option<String>,
    /// Space-separated list of allowed scopes
    pub scope: Option<String>,
    /// When this client was created
    pub created_at: Timestamp,
    /// Optional expiration time for the client registration
    pub expires_at: Option<Timestamp>,
}

/// What one retention sweep of the RFC 7591 client registrations deleted.
///
/// A client's authorization codes, refresh tokens and CSRF states go with it
/// through their `ON DELETE CASCADE` foreign keys; its consent grants carry no
/// foreign key and are deleted explicitly, and counted here.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OAuth2ClientSweep {
    /// Registrations deleted because their `expires_at` passed the grace cutoff
    pub expired: u64,
    /// Registrations deleted because no user authorized them before the cutoff
    pub abandoned: u64,
    /// Consent grants deleted because the client they name no longer exists
    pub orphaned_grants: u64,
    /// `expire` receipts written for grants past their expiry that had not
    /// ended (the grant rows stay, listed as expired)
    pub expired_grants: u64,
    /// RFC 9126 pushed authorization requests deleted past their expiry
    pub expired_pushed_requests: u64,
}

/// An RFC 9126 pushed authorization request, one `oauth2_pushed_requests` row.
///
/// The `request_uri` handed to the client is never stored, only its SHA-256.
/// `Debug` is hand-written: the parameters carry the request's `state`, so
/// they print as their size.
#[derive(Clone)]
pub struct OAuth2PushedRequest {
    /// Row identifier
    pub id: String,
    /// SHA-256 hex of the `request_uri` the client was given
    pub request_uri_hash: String,
    /// The client that pushed the request, the only one that may use it
    pub client_id: String,
    /// The checked authorization request's parameters, as a JSON object of strings
    pub parameters: String,
    /// When the `request_uri` stops resolving
    pub expires_at: Timestamp,
    /// When a code was minted from it; `None` while unused
    pub used_at: Option<Timestamp>,
    /// When the client pushed it
    pub created_at: Timestamp,
}

impl fmt::Debug for OAuth2PushedRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OAuth2PushedRequest")
            .field("id", &self.id)
            .field("request_uri_hash", &self.request_uri_hash)
            .field("client_id", &self.client_id)
            .field("parameters_bytes", &self.parameters.len())
            .field("expires_at", &self.expires_at)
            .field("used_at", &self.used_at)
            .field("created_at", &self.created_at)
            .finish()
    }
}

/// A pending RFC 8628 Device Authorization for `pierre-cli auth login`.
///
/// Created when the CLI calls `/admin/device/authorization`, flipped to
/// `approved`/`denied` by an admin at the verification page, and consumed
/// (deleted) when the CLI's token poll mints the admin token. Timestamps are
/// Unix epoch seconds to match the `device_authorization` table; the raw
/// `device_code` is never stored, only its SHA-256 hash.
///
/// `Debug` is hand-written: the `user_code` approves the flow at the
/// verification page, so it prints as `[REDACTED]`.
#[derive(Clone)]
pub struct DeviceAuthorization {
    /// SHA-256 hex of the raw `device_code` (the raw code is never persisted).
    pub device_code_hash: String,
    /// Short human-typeable code the admin enters at the approval page.
    pub user_code: String,
    /// Lifecycle status: `pending`, `approved`, or `denied`.
    pub status: String,
    /// The id of the admin who approved, as text, set once
    /// `status = approved`; empty for an approval a service token made.
    /// Never an email: the row is deleted with its approver's account.
    pub approved_by: Option<String>,
    /// Creation time, Unix epoch seconds.
    pub created_at: i64,
    /// Expiry, Unix epoch seconds; the flow must complete before this.
    pub expires_at: i64,
}

impl fmt::Debug for DeviceAuthorization {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DeviceAuthorization")
            .field("device_code_hash", &self.device_code_hash)
            .field("user_code", &"[REDACTED]")
            .field("status", &self.status)
            .field("approved_by", &self.approved_by)
            .field("created_at", &self.created_at)
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

/// OAuth 2.0 Authorization Code
///
/// `Debug` is hand-written: the code is exchangeable for a token and the
/// state is the CSRF secret, so both print as `[REDACTED]`.
#[derive(Clone)]
pub struct OAuth2AuthCode {
    /// The authorization code value
    pub code: String,
    /// Client ID that requested this code
    pub client_id: String,
    /// User who authorized the code
    pub user_id: Uuid,
    /// Tenant ID for multi-tenancy
    pub tenant_id: String,
    /// Redirect URI that must match during token exchange
    pub redirect_uri: String,
    /// Space-separated list of granted scopes
    pub scope: Option<String>,
    /// When this authorization code expires
    pub expires_at: Timestamp,
    /// Whether this code has been exchanged for a token
    pub used: bool,
    /// Client-generated state for CSRF protection (RFC 6749 Section 10.12)
    pub state: Option<String>,
    /// PKCE code challenge (RFC 7636)
    pub code_challenge: Option<String>,
    /// PKCE code challenge method (plain or S256)
    pub code_challenge_method: Option<String>,
    /// RFC 8707 resource the authorization was bound to — the audience of the
    /// access token this code is exchanged for. `None` when the request named none.
    pub resource: Option<String>,
    /// The consent grant (`oauth_client_grants.id`) the code was issued
    /// under; `None` for a code minted before grants were named, or for one
    /// no consent grant governs (Dravr's own apps).
    pub client_grant_id: Option<String>,
    /// RFC 7638 thumbprint of the `DPoP` key the authorization was bound to
    /// (RFC 9449 §10: a proof at PAR, or `dpop_jkt` on authorize). The token
    /// endpoint then requires a proof with that key. `None` when unbound.
    pub dpop_jkt: Option<String>,
}

impl fmt::Debug for OAuth2AuthCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OAuth2AuthCode")
            .field("code", &"[REDACTED]")
            .field("client_id", &self.client_id)
            .field("user_id", &self.user_id)
            .field("tenant_id", &self.tenant_id)
            .field("redirect_uri", &self.redirect_uri)
            .field("scope", &self.scope)
            .field("expires_at", &self.expires_at)
            .field("used", &self.used)
            .field("state", &self.state.as_ref().map(|_| "[REDACTED]"))
            .field("code_challenge", &self.code_challenge)
            .field("code_challenge_method", &self.code_challenge_method)
            .field("resource", &self.resource)
            .field("client_grant_id", &self.client_grant_id)
            .field("dpop_jkt", &self.dpop_jkt)
            .finish()
    }
}

/// OAuth 2.0 Refresh Token
#[derive(Debug, Clone)]
pub struct OAuth2RefreshToken {
    /// The refresh token value
    pub token: SecretString,
    /// Client application identifier
    pub client_id: String,
    /// User identifier who owns this token
    pub user_id: Uuid,
    /// Tenant identifier for multi-tenant support
    pub tenant_id: String,
    /// Optional space-separated list of granted scopes
    pub scope: Option<String>,
    /// Timestamp when this refresh token expires
    pub expires_at: Timestamp,
    /// Timestamp when this refresh token was created
    pub created_at: Timestamp,
    /// Whether this refresh token has been revoked
    pub revoked: bool,
    /// The rotation chain this token belongs to. Every token a refresh
    /// exchange issues joins its predecessor's family, so presenting one that
    /// was already rotated out revokes the whole chain.
    pub family_id: String,
    /// RFC 8707 resource the grant was bound to — every access token minted
    /// from this refresh token carries it as its audience. `None` when unbound.
    pub resource: Option<String>,
    /// When the rotation chain began. Every successor carries its
    /// predecessor's value, so the exchange can refuse a family older than
    /// the absolute family lifetime. `None` is a legacy family issued before
    /// the column existed: no absolute limit.
    pub family_created_at: Option<Timestamp>,
    /// The consent grant (`oauth_client_grants.id`) the family was issued
    /// under, which every successor and access token minted from it carries.
    /// `None` for a family begun before grants were named.
    pub client_grant_id: Option<String>,
    /// RFC 7638 thumbprint of the `DPoP` key the family is bound to: every
    /// refresh needs a proof with that key (RFC 9449 §5), and every access
    /// token minted from it carries `cnf.jkt`. `None` when unbound (Bearer).
    pub dpop_jkt: Option<String>,
}

/// OAuth 2.0 State for CSRF Protection
///
/// `Debug` is hand-written: the state value is the CSRF secret, so it prints
/// as `[REDACTED]`.
#[derive(Clone)]
pub struct OAuth2State {
    /// Unique state value for CSRF protection
    pub state: String,
    /// Client application identifier
    pub client_id: String,
    /// Optional user identifier if authenticated
    pub user_id: Option<Uuid>,
    /// Optional tenant identifier for multi-tenant support
    pub tenant_id: Option<String>,
    /// URI to redirect to after authorization
    pub redirect_uri: String,
    /// Optional space-separated list of requested scopes
    pub scope: Option<String>,
    /// Optional PKCE code challenge for enhanced security
    pub code_challenge: Option<String>,
    /// Method used for PKCE code challenge (S256 or plain)
    pub code_challenge_method: Option<String>,
    /// Timestamp when this state was created
    pub created_at: Timestamp,
    /// Timestamp when this state expires
    pub expires_at: Timestamp,
    /// Whether this state has been consumed
    pub used: bool,
}

impl fmt::Debug for OAuth2State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OAuth2State")
            .field("state", &"[REDACTED]")
            .field("client_id", &self.client_id)
            .field("user_id", &self.user_id)
            .field("tenant_id", &self.tenant_id)
            .field("redirect_uri", &self.redirect_uri)
            .field("scope", &self.scope)
            .field("code_challenge", &self.code_challenge)
            .field("code_challenge_method", &self.code_challenge_method)
            .field("created_at", &self.created_at)
            .field("expires_at", &self.expires_at)
            .field("used", &self.used)
            .finish()
    }
}
