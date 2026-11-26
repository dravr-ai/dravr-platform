// ABOUTME: JWT-based user authentication and authorization system
// ABOUTME: Handles user login, token generation, validation, and session management
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Authentication and Session Management
//!
// NOTE: All `.clone()` calls in this file are Safe - they are necessary for:
// - Arc resource sharing for auth managers across threads
// - String ownership for JWT claims and session data
// - Database result ownership transfers
//!
//! This module provides JWT-based authentication and session management
//! for the multi-tenant Pierre MCP Server.

use secrecy::{ExposeSecret, SecretString};
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::iter;

use dravr_tronc::date_pattern;
use dravr_tronc::dates::{Elapsed, Pattern, Timestamp};
use jsonwebtoken::{
    decode, encode,
    errors::{Error as JwtError, ErrorKind},
    Algorithm, Header, Validation,
};
use serde::{Deserialize, Serialize};
use tracing::{debug, error, warn};
use uuid::Uuid;

use crate::admin::jwks::JwksManager;
use crate::oauth2_server::first_party::names_third_party;
use crate::oauth2_server::grant_ceiling::GrantCeiling;
use pierre_core::constants::{
    service_names::{MCP, OAUTH2_AUTHORIZE, PIERRE_MCP_SERVER},
    time_constants::SECONDS_PER_HOUR,
};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{AuthRequest, AuthResponse, User, UserSession};
use pierre_core::permissions::impersonation::SESSION_MINUTES;
use pierre_core::permissions::scopes::OAuthScope;
use pierre_core::uuid_utils::parse_uuid;

/// `2026-11-17 10:00:05 UTC`: when a token expired.
const EXPIRED_AT: Pattern<Timestamp> = date_pattern!(Timestamp, "%Y-%m-%d %H:%M:%S UTC");

mod delegated_token;
mod method;
mod session_token;
pub use delegated_token::{oauth_access_token_lifetime_secs, TokenLineage};
pub use method::{
    refuse_support_session, AuthMethod, ImpersonationGrant, IMPERSONATION_REFUSED_ACTION,
    IMPERSONATION_WRITE_REFUSED,
};

/// Convert a duration to a human-readable format
fn humanize_duration(duration: Elapsed) -> String {
    let total_secs = duration.num_seconds().abs();
    let hours = total_secs / i64::from(SECONDS_PER_HOUR);
    let minutes = (total_secs % i64::from(SECONDS_PER_HOUR)) / 60;

    if hours > 0 {
        format!("{hours} hours")
    } else if minutes > 0 {
        format!("{minutes} minutes")
    } else {
        format!("{total_secs} seconds")
    }
}

/// Whether a `jsonwebtoken` failure lies with the token the caller presented.
///
/// An expired, forged, malformed or mis-addressed token is the caller's, and
/// anyone can send one. The remaining kinds (a key this server cannot read, an
/// algorithm list it configured empty, the crypto provider failing) are the
/// server's. An unlisted kind counts as the server's, so a variant a later
/// `jsonwebtoken` adds is reported loudly until someone classifies it.
const fn is_presented_token_fault(kind: &ErrorKind) -> bool {
    matches!(
        kind,
        ErrorKind::InvalidToken
            | ErrorKind::InvalidSignature
            | ErrorKind::MissingRequiredClaim(_)
            | ErrorKind::InvalidClaimFormat(_)
            | ErrorKind::ExpiredSignature
            | ErrorKind::InvalidIssuer
            | ErrorKind::InvalidAudience
            | ErrorKind::InvalidSubject
            | ErrorKind::ImmatureSignature
            | ErrorKind::InvalidAlgorithm
            | ErrorKind::Base64(_)
            | ErrorKind::Json(_)
            | ErrorKind::Utf8(_)
    )
}

/// `JWT` validation error with detailed information
#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum JwtValidationError {
    /// Token has expired
    TokenExpired {
        /// When the token expired
        expired_at: Timestamp,
        /// Current time for reference
        current_time: Timestamp,
    },
    /// Token signature is invalid
    TokenInvalid {
        /// Reason for invalidity
        reason: String,
    },
    /// Token is malformed (not proper `JWT` format)
    TokenMalformed {
        /// Details about malformation
        details: String,
    },
}

impl Display for JwtValidationError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::TokenExpired {
                expired_at,
                current_time,
            } => {
                let duration_expired = current_time.signed_duration_since(*expired_at);
                if duration_expired.num_minutes() < 60 {
                    write!(
                        f,
                        "JWT token expired {} minutes ago at {}",
                        duration_expired.num_minutes(),
                        expired_at.format_with(&EXPIRED_AT)
                    )
                } else if duration_expired.num_days() < 1 {
                    write!(
                        f,
                        "JWT token expired {} hours ago at {}",
                        duration_expired.num_hours(),
                        expired_at.format_with(&EXPIRED_AT)
                    )
                } else {
                    write!(
                        f,
                        "JWT token expired {} days ago at {}",
                        duration_expired.num_days(),
                        expired_at.format_with(&EXPIRED_AT)
                    )
                }
            }
            Self::TokenInvalid { reason } => {
                write!(f, "JWT token signature is invalid: {reason}")
            }
            Self::TokenMalformed { details } => {
                write!(f, "JWT token is malformed: {details}")
            }
        }
    }
}

impl Error for JwtValidationError {}

/// `JWT` claims for user authentication
///
/// The `active_tenant_id` field represents the currently active tenant context
/// for this session. Users can belong to multiple tenants (via `tenant_users` table);
/// sign-in names the tenant a session acts in, and no route changes it afterwards.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Claims {
    /// User `ID`
    pub sub: String,
    /// User email
    pub email: String,
    /// Issued at timestamp (seconds since Unix epoch)
    pub iat: i64,
    /// Expiration timestamp
    pub exp: i64,
    /// Issuer (who issued the token)
    pub iss: String,
    /// JWT ID (unique identifier for this token)
    pub jti: String,
    /// Audience (who the token is intended for): the platform audience `mcp`,
    /// or the RFC 8707 resource an OAuth access token was bound to — which only
    /// that resource server accepts
    pub aud: String,
    /// The tenant this session asks to act in (a user can belong to several).
    ///
    /// It is honoured only after `pierre_runtime_context::tenant::resolve_tenant` finds a
    /// membership row for it; a session naming none acts in the account's first tenant. Sign-in
    /// sets it and a session refresh or restore carries it forward.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_tenant_id: Option<String>,
    /// Original user ID when impersonating (the admin)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub impersonator_id: Option<String>,
    /// Impersonation session ID for audit trail
    #[serde(skip_serializing_if = "Option::is_none")]
    pub impersonation_session_id: Option<String>,
    /// The grant this token carries, space-delimited (RFC 9068 §2.2.3).
    ///
    /// Every token gets one, first-party sessions included. An unscoped token
    /// would be a bypass of the gate rather than an exemption from it: the
    /// resource server cannot tell "this caller was granted everything" from
    /// "this caller predates scopes" without one, and would have to let the
    /// second through to keep the first working.
    ///
    /// `#[serde(default)]` so a token minted before this claim still decodes;
    /// it decodes to the empty grant, which the gate refuses for any tool that
    /// reads or writes. That is the intended direction for an expiring
    /// credential — tokens turn over within hours.
    #[serde(default)]
    pub scope: String,
    /// The OAuth client the token was issued to (RFC 9068 §2.2): the
    /// `client_id` assigned at registration and checked at issuance, which
    /// the audit trail names — never the `clientInfo` or `User-Agent` a caller
    /// sends about itself. `None` on a session no client signed in to, and on
    /// a token minted before this claim existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    /// The OAuth grant a delegated access token was minted under: the
    /// refresh-token family its authorization started, which every token
    /// rotated from it keeps. One authorization is one family, so two
    /// connections of the same application hold two. A support session an
    /// admin's assistant starts is bound to it (carnet#1511). `None` on any
    /// other token, and on one minted before this claim existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grant_id: Option<String>,
    /// The consent grant a delegated access token was minted under: the
    /// `oauth_client_grants` row the athlete approved, which the code, every
    /// refresh token of its family and every access token minted from them
    /// name. `/mcp` and A2A admit the token only while that grant is live.
    /// Not [`Self::grant_id`], the refresh family. `None` on any other token,
    /// and on one minted before this claim existed, which is admitted as
    /// before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_grant_id: Option<String>,
    /// The key the token is bound to (RFC 9449 §6.1, RFC 7800): present on a
    /// token minted under a `DPoP` proof. The resource server then admits it
    /// only under the `DPoP` scheme with a proof signed by that key, never as
    /// Bearer. `None` on an unbound (Bearer) token and on any token minted
    /// before this claim existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cnf: Option<Confirmation>,
    /// When the athlete last signed in on the authorization server's own page
    /// (RFC 9470 `auth_time`, seconds since the Unix epoch): set only on the
    /// sign-in token the hosted login (password or Google, verified by Dravr)
    /// mints, which a step-up approval must find at most five minutes old.
    /// `None` on every other token, so no web-app session, refreshed session
    /// or delegated token ever satisfies a step-up.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_time: Option<i64>,
}

/// A token's confirmation claim (RFC 7800 `cnf`), carrying the RFC 7638
/// SHA-256 thumbprint of the `DPoP` key it is bound to (RFC 9449 §6.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Confirmation {
    /// The key's RFC 7638 thumbprint, base64url without padding
    pub jkt: String,
}

/// The token kinds (owl PDPP §5.1) this server mints.
///
/// The protected resource metadata lists them: `owner`, a session of Dravr's
/// own apps, and `client`, a third party's delegated token. `agent_write` (4c)
/// and `relay_ingest` (phase 8) join when the server can serve them.
pub const PDPP_TOKEN_KINDS: [&str; 2] = ["owner", "client"];

impl Claims {
    /// The token's PDPP kind, derived and never stored.
    ///
    /// `owner` for a session ([`Self::is_session`]), an impersonation
    /// included; `client` for a token a third party holds
    /// ([`Self::is_delegated`]), a client-credentials token included; `None`
    /// for anything else (a token naming no client and holding less than the
    /// self grant, minted before the `client_id` or `scope` claim).
    #[must_use]
    pub fn pdpp_token_kind(&self) -> Option<&'static str> {
        if self.is_session() {
            Some(PDPP_TOKEN_KINDS[0])
        } else if self.is_delegated() {
            Some(PDPP_TOKEN_KINDS[1])
        } else {
            None
        }
    }

    /// The client a client-credentials token was issued to; `None` for a
    /// token that acts for a user. RFC 9068 §2.2 makes such a token's `sub`
    /// the client itself, and no `client_id` this server assigns is a user's
    /// UUID, so reading both claims keeps a client from being taken for one.
    #[must_use]
    pub fn client_credentials_client(&self) -> Option<&str> {
        self.client_id
            .as_deref()
            .filter(|client_id| *client_id == self.sub)
    }

    /// Whether the token is a third party's: it names a client that is not
    /// one of Dravr's own apps ([`names_third_party`]), whatever its grant.
    #[must_use]
    pub fn is_delegated(&self) -> bool {
        names_third_party(self.client_id.as_deref())
    }

    /// Whether the token is a session of Dravr's own apps: not a third
    /// party's, and holding the whole [`OAuthScope::self_grant`].
    ///
    /// Both must hold. A delegated token holding every scope is still its
    /// client's, and a narrower grant naming no client was minted before the
    /// `client_id` claim existed (a delegation) or before the `scope` claim
    /// did, so it is not taken for a session.
    ///
    /// An impersonation token is a session by this test: it names no client
    /// and holds every scope. A reader that must not take one refuses it by
    /// its own claim, as [`AuthManager::validate_session_token`] does.
    #[must_use]
    pub fn is_session(&self) -> bool {
        !self.is_delegated() && OAuthScope::is_self_grant(&OAuthScope::parse_granted(&self.scope))
    }
}

/// What an OAuth grant decided about the access token it is redeemed for.
#[derive(Debug, Clone, Copy)]
pub struct DelegatedGrant<'a> {
    /// The registered client the grant was issued to: the `client_id` the
    /// token endpoint authenticated
    pub client_id: &'a str,
    /// The scope the grant carries
    pub scopes: &'a [String],
    /// The RFC 8707 resource the token is bound to; `None` mints the platform
    /// audience every Pierre surface accepts
    pub audience: Option<&'a str>,
}

/// Authentication result with user context
///
/// The caller's request budget is not carried here: the auth middleware
/// gates on it and reports it to the response's `X-RateLimit-*` headers once
/// the credential's outcome is the request's answer.
#[derive(Debug)]
pub struct AuthResult {
    /// Authenticated user `ID`
    pub user_id: Uuid,
    /// Authentication method used
    pub auth_method: AuthMethod,
    /// Active tenant ID from JWT claims (for multi-tenant user tenant selection)
    /// Users can belong to multiple tenants and this field indicates which tenant
    /// should be used for the current request. Extracted from JWT `active_tenant_id` claim.
    pub active_tenant_id: Option<Uuid>,
    /// The grant this credential carries, for the scope gate at the dispatch
    /// chokepoint.
    ///
    /// A JWT's comes from its own `scope` claim, so a delegated OAuth token
    /// stays as narrow as it was issued. Every other credential — the athlete's
    /// own API key, a linked messaging channel — is the athlete acting
    /// directly, not a delegation, so it carries
    /// [`OAuthScope::self_grant`]. That is not a way in: the role gate runs
    /// independently and still refuses an admin tool to a non-admin.
    pub scopes: Vec<OAuthScope>,
    /// The OAuth client the credential was issued to ([`Claims::client_id`]);
    /// `None` for an API key, a personal MCP token or a channel link, which
    /// no registered client holds, and for a session no client signed in to.
    pub client_id: Option<String>,
    /// The OAuth grant a delegated token was minted under
    /// ([`Claims::grant_id`]); `None` for every other credential.
    pub grant_id: Option<String>,
    /// The consent grant a delegated token was minted under
    /// ([`Claims::client_grant_id`]); `None` for every other credential, and
    /// for a delegated token minted before the claim existed.
    pub client_grant_id: Option<String>,
    /// What a delegated token under a detailed consent grant may reach;
    /// `None` for a plain grant and every other credential.
    pub ceiling: Option<GrantCeiling>,
}

/// Authentication manager for `JWT` tokens and user sessions
#[derive(Debug, Clone)]
pub struct AuthManager {
    token_expiry_hours: i64,
    /// The authorization server's issuer identifier, its metadata's `issuer`
    /// (`OAuth2ServerConfig::issuer_url`), which every `/oauth2/token` access
    /// token carries as `iss` (RFC 9068 §2.2). `None` on a manager built
    /// without it (the CLI, benches, tests), which mints and accepts only
    /// [`PIERRE_MCP_SERVER`], the name sessions and impersonation tokens keep.
    oauth_issuer: Option<String>,
}

impl AuthManager {
    /// The grant a first-party session token carries: [`OAuthScope::self_grant`].
    ///
    /// Spelled once, here, so the session minters cannot drift apart — a minter
    /// that quietly issued a narrower grant would refuse tools the athlete owns.
    /// It takes no `is_admin`: `admin` is in every self grant and confers no
    /// admin, because the role gate decides that independently and both axes
    /// have to pass.
    fn session_scope() -> String {
        OAuthScope::render_granted(&OAuthScope::self_grant())
    }

    /// Create a new authentication manager
    #[must_use]
    pub const fn new(token_expiry_hours: i64) -> Self {
        Self {
            token_expiry_hours,
            oauth_issuer: None,
        }
    }

    /// The manager minting OAuth access tokens under `issuer`, the
    /// authorization server's metadata `issuer`, and accepting it on them.
    #[must_use]
    pub fn with_oauth_issuer(mut self, issuer: impl Into<String>) -> Self {
        self.oauth_issuer = Some(issuer.into());
        self
    }

    /// The `iss` an OAuth access token is minted with: the authorization
    /// server's issuer, or [`PIERRE_MCP_SERVER`] on a manager without one.
    fn oauth_token_issuer(&self) -> &str {
        self.oauth_issuer.as_deref().unwrap_or(PIERRE_MCP_SERVER)
    }

    /// Every `iss` a token this manager validates may carry, before its kind
    /// is read ([`Self::issuer_fits_kind`]).
    fn accepted_issuers(&self) -> Vec<&str> {
        iter::once(PIERRE_MCP_SERVER)
            .chain(self.oauth_issuer.as_deref())
            .collect()
    }

    /// Whether `claims.iss` is one the token's kind may carry. A session or an
    /// impersonation token carries [`PIERRE_MCP_SERVER`] only. An OAuth
    /// access token (a third party's delegation, or a client's own
    /// client-credentials token) carries the authorization server's issuer,
    /// or [`PIERRE_MCP_SERVER`] when an older binary minted it.
    fn issuer_fits_kind(claims: &Claims) -> bool {
        if claims.iss == PIERRE_MCP_SERVER {
            return true;
        }
        claims.impersonator_id.is_none()
            && (claims.is_delegated() || claims.client_credentials_client().is_some())
    }

    /// Generate a `JWT` token for a user with RS256 asymmetric signing
    ///
    /// # Arguments
    /// * `user` - The user to generate a token for
    /// * `jwks_manager` - JWKS manager for signing
    /// * `active_tenant_id` - Optional active tenant ID for this session
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - JWT encoding fails due to invalid claims
    /// - System time is unavailable for timestamp generation
    /// - JWKS manager has no active key
    pub fn generate_token(&self, user: &User, jwks_manager: &JwksManager) -> AppResult<String> {
        self.generate_token_with_tenant(user, jwks_manager, None)
    }

    /// Generate a `JWT` token for a user with a specific active tenant
    ///
    /// # Arguments
    /// * `user` - The user to generate a token for
    /// * `jwks_manager` - JWKS manager for signing
    /// * `active_tenant_id` - The tenant ID to set as active for this session
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - JWT encoding fails due to invalid claims
    /// - System time is unavailable for timestamp generation
    /// - JWKS manager has no active key
    pub fn generate_token_with_tenant(
        &self,
        user: &User,
        jwks_manager: &JwksManager,
        active_tenant_id: Option<String>,
    ) -> AppResult<String> {
        self.generate_session_for(user, jwks_manager, active_tenant_id, MCP, None)
    }

    /// [`Self::generate_token_with_tenant`] for a session a client signed in
    /// to: `client_id` is the first-party app that redeemed the code, or, on
    /// a refresh or restore, the client the session named.
    ///
    /// # Errors
    ///
    /// As [`Self::generate_token_with_tenant`], and when `client_id` names a
    /// client that is not one of Dravr's own apps: a session holds every
    /// scope, and a token naming a third party is that party's delegation.
    pub fn generate_client_session_token(
        &self,
        user: &User,
        jwks_manager: &JwksManager,
        active_tenant_id: Option<String>,
        client_id: Option<&str>,
    ) -> AppResult<String> {
        self.generate_session_for(user, jwks_manager, active_tenant_id, MCP, client_id)
    }

    /// Generate the authorization server's own sign-in token: the one the
    /// `/oauth2/login` form and its Google sign-in set in the
    /// `pierre_session` cookie (carnet#787).
    ///
    /// It carries the self grant, because the consent screen it signs in
    /// approves new grants, but its audience is [`OAUTH2_AUTHORIZE`], which
    /// only [`Self::validate_authorization_session_token`] accepts. A script
    /// that signs in through the hosted login form and lifts the cookie
    /// holds a token chat, REST, MCP and A2A all refuse, rather than a
    /// first-party session.
    ///
    /// # Errors
    ///
    /// As [`Self::generate_token_with_tenant`].
    pub fn generate_authorization_session_token(
        &self,
        user: &User,
        jwks_manager: &JwksManager,
        active_tenant_id: Option<String>,
    ) -> AppResult<String> {
        self.generate_session_for(user, jwks_manager, active_tenant_id, OAUTH2_AUTHORIZE, None)
    }

    /// Mint a self-grant session token for `user` addressed to `audience`,
    /// naming the client it was signed in to, if any.
    ///
    /// Refuses a third-party `client_id` by the rule [`Claims::is_delegated`]
    /// reads a token with, so no token this signs is a delegation holding
    /// the `admin` scope.
    fn generate_session_for(
        &self,
        user: &User,
        jwks_manager: &JwksManager,
        active_tenant_id: Option<String>,
        audience: &str,
        client_id: Option<&str>,
    ) -> AppResult<String> {
        if names_third_party(client_id) {
            return Err(AppError::internal(
                "A session names one of Dravr's own apps or no client",
            ));
        }

        let now = Timestamp::now();
        let expiry = now + Elapsed::hours(self.token_expiry_hours);

        let claims = Claims {
            sub: user.id.to_string(),
            email: user.email.clone(),
            iat: now.epoch_seconds(),
            exp: expiry.epoch_seconds(),
            iss: PIERRE_MCP_SERVER.to_owned(),
            jti: Uuid::new_v4().to_string(),
            scope: Self::session_scope(),
            aud: audience.to_owned(),
            active_tenant_id,
            impersonator_id: None,
            impersonation_session_id: None,
            client_id: client_id.map(str::to_owned),
            grant_id: None,
            client_grant_id: None,
            cnf: None,
            // Only the hosted login's sign-in records when it happened: a
            // step-up reads it, and no other session may satisfy one.
            auth_time: (audience == OAUTH2_AUTHORIZE).then_some(now.epoch_seconds()),
        };

        // Get active RSA key from JWKS manager
        let active_key = jwks_manager.get_active_key()?;
        let encoding_key = active_key.encoding_key()?;

        // Create RS256 header with kid
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(active_key.kid.clone());

        let token = encode(&header, &claims, &encoding_key)
            .map_err(|e| AppError::internal(format!("Failed to encode JWT token: {e}")))?;

        Ok(token)
    }

    /// Generate an impersonation `JWT` token for an admin impersonating another user
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - JWT encoding fails due to invalid claims
    /// - System time is unavailable for timestamp generation
    /// - JWKS manager has no active key
    pub fn generate_impersonation_token(
        &self,
        target_user: &User,
        impersonator_id: Uuid,
        session_id: &str,
        jwks_manager: &JwksManager,
        active_tenant_id: Option<String>,
    ) -> AppResult<String> {
        // Impersonation tokens have shorter expiry (1 hour)
        let expiry = (Timestamp::now() + Elapsed::minutes(SESSION_MINUTES)).epoch_seconds();
        Self::impersonation_token(
            target_user,
            impersonator_id,
            session_id,
            expiry,
            jwks_manager,
            active_tenant_id,
        )
    }

    /// Sign an impersonation token expiring at `expiry` (seconds since the
    /// epoch).
    fn impersonation_token(
        target_user: &User,
        impersonator_id: Uuid,
        session_id: &str,
        expiry: i64,
        jwks_manager: &JwksManager,
        active_tenant_id: Option<String>,
    ) -> AppResult<String> {
        let now = Timestamp::now();
        let claims = Claims {
            sub: target_user.id.to_string(),
            email: target_user.email.clone(),
            iat: now.epoch_seconds(),
            exp: expiry,
            iss: PIERRE_MCP_SERVER.to_owned(),
            jti: Uuid::new_v4().to_string(),
            scope: Self::session_scope(),
            aud: MCP.to_owned(),
            active_tenant_id,
            impersonator_id: Some(impersonator_id.to_string()),
            impersonation_session_id: Some(session_id.to_owned()),
            // An admin acting as the athlete, through no client.
            client_id: None,
            grant_id: None,
            client_grant_id: None,
            cnf: None,
            auth_time: None,
        };

        // Get active RSA key from JWKS manager
        let active_key = jwks_manager.get_active_key()?;
        let encoding_key = active_key.encoding_key()?;

        // Create RS256 header with kid
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(active_key.kid.clone());

        let token = encode(&header, &claims, &encoding_key).map_err(|e| {
            AppError::internal(format!("Failed to encode impersonation JWT token: {e}"))
        })?;

        Ok(token)
    }

    /// Validate the authorization server's own sign-in token, minted by
    /// [`Self::generate_authorization_session_token`]: audience
    /// [`OAUTH2_AUTHORIZE`] and the whole self grant.
    ///
    /// A platform session token is refused here, as this token is refused
    /// everywhere else: each cookie carries the one audience its reader takes.
    ///
    /// # Errors
    ///
    /// A token that fails signature, issuer or expiry checks, whose audience
    /// is not [`OAUTH2_AUTHORIZE`], that names a third-party client, or whose
    /// grant is narrower than the self grant.
    pub fn validate_authorization_session_token(
        &self,
        token: &str,
        jwks_manager: &JwksManager,
    ) -> AppResult<Claims> {
        Self::require_session(self.validate_token_for(token, jwks_manager, &[OAUTH2_AUTHORIZE])?)
    }

    /// `claims` when they are a session ([`Claims::is_session`]); a token a
    /// third party holds, or one with a narrower grant, is not one.
    fn require_session(claims: Claims) -> AppResult<Claims> {
        if claims.is_session() {
            Ok(claims)
        } else {
            Err(AppError::auth_invalid(
                "A delegated access token is not a session",
            ))
        }
    }

    /// Validate a RS256 JWT token using JWKS public keys
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Token signature is invalid
    /// - Token has expired
    /// - Token is malformed or not valid JWT format
    /// - Token header doesn't contain kid (key ID)
    /// - JWKS manager doesn't have the specified key
    /// - Token claims cannot be deserialized
    pub fn validate_token(&self, token: &str, jwks_manager: &JwksManager) -> AppResult<Claims> {
        self.validate_token_for(token, jwks_manager, &[MCP])
    }

    /// Validate a RS256 JWT token as the protected resource known by each of
    /// `resources` (RFC 8707): the platform audience, or a token audience-bound
    /// to one of them. A token bound to any other resource is refused.
    ///
    /// # Errors
    ///
    /// Everything [`Self::validate_token`] refuses, less a token whose audience
    /// is one of `resources`.
    pub fn validate_resource_token(
        &self,
        token: &str,
        jwks_manager: &JwksManager,
        resources: &[&str],
    ) -> AppResult<Claims> {
        self.validate_token_for(token, jwks_manager, &resource_audiences(resources))
    }

    /// Validate a RS256 JWT token whose audience is one of `audiences`, and
    /// whose issuer is one this manager accepts for the token's kind.
    fn validate_token_for(
        &self,
        token: &str,
        jwks_manager: &JwksManager,
        audiences: &[&str],
    ) -> AppResult<Claims> {
        // Extract kid from token header
        let header = jsonwebtoken::decode_header(token)
            .map_err(|e| AppError::auth_invalid(format!("Failed to decode token header: {e}")))?;
        let kid = header.kid.ok_or_else(|| -> AppError {
            AppError::auth_invalid("Token header missing kid (key ID)")
        })?;

        debug!("Validating RS256 JWT token with kid: {}", kid);

        // Get public key from JWKS manager
        let key_pair = jwks_manager.get_key(&kid).ok_or_else(|| -> AppError {
            AppError::auth_invalid(format!("Key not found in JWKS: {kid}"))
        })?;

        let decoding_key = key_pair
            .decoding_key()
            .map_err(|e| AppError::auth_invalid(format!("Failed to get decoding key: {e}")))?;

        let mut validation = Validation::new(Algorithm::RS256);
        validation.validate_exp = true;
        validation.set_audience(audiences);
        validation.set_issuer(&self.accepted_issuers());

        let token_data = decode::<Claims>(token, &decoding_key, &validation).map_err(|e| {
            if is_presented_token_fault(e.kind()) {
                warn!("RS256 JWT validation failed: {:?}", e);
            } else {
                error!("RS256 JWT validation failed: {:?}", e);
            }
            AppError::auth_invalid(format!("JWT validation failed: {e}"))
        })?;

        if !Self::issuer_fits_kind(&token_data.claims) {
            warn!(issuer = %token_data.claims.iss, "A session token carries the OAuth issuer");
            return Err(AppError::auth_invalid(
                "JWT validation failed: InvalidIssuer",
            ));
        }

        Ok(token_data.claims)
    }

    /// Check if token is expired and return error if so
    fn check_token_expiry(
        claims: &Claims,
        current_time: Timestamp,
        expired_at: Timestamp,
    ) -> Result<(), JwtValidationError> {
        if current_time.epoch_seconds() > claims.exp {
            let time_since_expiry = current_time.signed_duration_since(expired_at);
            warn!(
                "JWT token expired for user: {} - Expired {} ago at {}",
                claims.sub,
                humanize_duration(time_since_expiry),
                expired_at.to_rfc3339()
            );
            return Err(JwtValidationError::TokenExpired {
                expired_at,
                current_time,
            });
        }
        Ok(())
    }

    /// Convert JWT library errors to detailed validation errors
    fn convert_jwt_error(e: &JwtError) -> JwtValidationError {
        warn!("JWT token validation failed: {:?}", e);

        match e.kind() {
            ErrorKind::InvalidSignature => {
                warn!("JWT token signature verification failed");
                JwtValidationError::TokenInvalid {
                    reason: "Token signature verification failed".to_owned(),
                }
            }
            ErrorKind::InvalidToken => {
                warn!("JWT token format is invalid: {:?}", e);
                JwtValidationError::TokenMalformed {
                    details: "Token format is invalid".to_owned(),
                }
            }
            ErrorKind::Base64(base64_err) => JwtValidationError::TokenMalformed {
                details: format!("Token contains invalid base64: {base64_err}"),
            },
            ErrorKind::Json(json_err) => JwtValidationError::TokenMalformed {
                details: format!("Token contains invalid JSON: {json_err}"),
            },
            ErrorKind::Utf8(utf8_err) => JwtValidationError::TokenMalformed {
                details: format!("Token contains invalid UTF-8: {utf8_err}"),
            },
            _ => JwtValidationError::TokenInvalid {
                reason: format!("Token validation failed: {e}"),
            },
        }
    }

    /// Validate a RS256 JWT token with detailed error information
    ///
    /// # Errors
    ///
    /// Returns a [`JwtValidationError`] if:
    /// - Token signature is invalid
    /// - Token has expired
    /// - Token is malformed or not valid JWT format
    /// - Token header doesn't contain kid (key ID)
    /// - JWKS manager doesn't have the specified key
    /// - Token claims cannot be deserialized
    pub fn validate_token_detailed(
        &self,
        token: &str,
        jwks_manager: &JwksManager,
    ) -> Result<Claims, JwtValidationError> {
        self.validate_detailed_for(token, jwks_manager, &[MCP])
    }

    /// Validate a RS256 JWT token as the protected resource known by each of
    /// `resources` (RFC 8707), with detailed error information: the platform
    /// audience, or a token audience-bound to one of them. A token bound to any
    /// other resource is refused as invalid.
    ///
    /// # Errors
    ///
    /// Everything [`Self::validate_token_detailed`] refuses, less a token whose
    /// audience is one of `resources`.
    pub fn validate_resource_token_detailed(
        &self,
        token: &str,
        jwks_manager: &JwksManager,
        resources: &[&str],
    ) -> Result<Claims, JwtValidationError> {
        self.validate_detailed_for(token, jwks_manager, &resource_audiences(resources))
    }

    /// Validate a RS256 JWT token whose audience is one of `audiences`, with
    /// detailed error information.
    fn validate_detailed_for(
        &self,
        token: &str,
        jwks_manager: &JwksManager,
        audiences: &[&str],
    ) -> Result<Claims, JwtValidationError> {
        debug!("Validating RS256 JWT token (length: {} chars)", token.len());

        let claims = self.decode_token_claims(token, jwks_manager, audiences)?;
        Self::validate_claims_expiry(&claims)?;

        debug!(
            "RS256 JWT token validation successful for user: {}",
            claims.sub
        );
        Ok(claims)
    }

    /// Decode RS256 JWT token claims without expiration validation, accepting
    /// a token whose audience is one of `audiences` and whose issuer is one
    /// this manager accepts for the token's kind
    fn decode_token_claims(
        &self,
        token: &str,
        jwks_manager: &JwksManager,
        audiences: &[&str],
    ) -> Result<Claims, JwtValidationError> {
        // Extract kid from token header
        let header =
            jsonwebtoken::decode_header(token).map_err(|e| JwtValidationError::TokenMalformed {
                details: format!("Failed to decode token header: {e}"),
            })?;

        let kid = header
            .kid
            .ok_or_else(|| JwtValidationError::TokenMalformed {
                details: "Token header missing kid (key ID)".to_owned(),
            })?;

        // Get public key from JWKS manager
        let key_pair =
            jwks_manager
                .get_key(&kid)
                .ok_or_else(|| JwtValidationError::TokenInvalid {
                    reason: format!("Key not found in JWKS: {kid}"),
                })?;

        let decoding_key =
            key_pair
                .decoding_key()
                .map_err(|e| JwtValidationError::TokenInvalid {
                    reason: format!("Failed to get decoding key: {e}"),
                })?;

        let mut validation_no_exp = Validation::new(Algorithm::RS256);
        validation_no_exp.validate_exp = false;
        validation_no_exp.set_audience(audiences);
        validation_no_exp.set_issuer(&self.accepted_issuers());

        let claims = decode::<Claims>(token, &decoding_key, &validation_no_exp)
            .map(|token_data| token_data.claims)
            .map_err(|e| Self::convert_jwt_error(&e))?;
        if Self::issuer_fits_kind(&claims) {
            Ok(claims)
        } else {
            warn!(issuer = %claims.iss, "A session token carries the OAuth issuer");
            Err(JwtValidationError::TokenInvalid {
                reason: "Token validation failed: InvalidIssuer".to_owned(),
            })
        }
    }

    /// Validate claims expiration with detailed logging
    fn validate_claims_expiry(claims: &Claims) -> Result<(), JwtValidationError> {
        let current_time = Timestamp::now();
        let expired_at = Timestamp::from_epoch_seconds(claims.exp)
            .ok()
            .unwrap_or_else(Timestamp::now);

        debug!(
            "Token validation details - User: {}, Issued: {}, Expires: {}, Current: {}",
            claims.sub,
            Timestamp::from_epoch_seconds(claims.iat)
                .ok()
                .map_or_else(|| "unknown".into(), Timestamp::to_rfc3339),
            expired_at.to_rfc3339(),
            current_time.to_rfc3339()
        );

        Self::check_token_expiry(claims, current_time, expired_at)
    }

    /// Create a user session from a valid user with RS256 token
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - JWT token generation fails
    /// - User data is invalid
    /// - System time is unavailable
    /// - JWKS manager has no active key
    pub fn create_session(
        &self,
        user: &User,
        jwks_manager: &JwksManager,
    ) -> AppResult<UserSession> {
        let jwt_token = self.generate_token(user, jwks_manager)?;
        let expires_at = Timestamp::now() + Elapsed::hours(self.token_expiry_hours);

        Ok(UserSession {
            user_id: user.id,
            jwt_token: SecretString::from(jwt_token),
            expires_at,
            email: user.email.clone(),
        })
    }

    /// Validate authentication request using RS256 and return response
    #[must_use]
    pub fn authenticate(&self, request: &AuthRequest, jwks_manager: &JwksManager) -> AuthResponse {
        match self.validate_token_detailed(request.token.expose_secret(), jwks_manager) {
            Ok(claims) => match parse_uuid(&claims.sub) {
                Ok(user_id) => AuthResponse {
                    authenticated: true,
                    user_id: Some(user_id),
                    error: None,
                },
                Err(e) => {
                    warn!(
                        sub = %claims.sub,
                        issuer = ?claims.iss,
                        error = %e,
                        "Invalid user ID in authentication token"
                    );
                    AuthResponse {
                        authenticated: false,
                        user_id: None,
                        error: Some("Invalid user ID in token".into()),
                    }
                }
            },
            Err(jwt_error) => AuthResponse {
                authenticated: false,
                user_id: None,
                error: Some(jwt_error.to_string()),
            },
        }
    }

    /// Refresh a token if it's still valid (RS256)
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Old token signature is invalid (even if expired)
    /// - Token is malformed
    /// - New token generation fails
    /// - User data is invalid
    /// - JWKS manager has no active key
    pub fn refresh_token(
        &self,
        old_token: &str,
        user: &User,
        jwks_manager: &JwksManager,
    ) -> AppResult<String> {
        // First validate the old token signature (even if expired)
        // This ensures the refresh request is legitimate
        self.decode_token_claims(old_token, jwks_manager, &[MCP])
            .map_err(|e| -> AppError {
                AppError::auth_invalid(format!("Failed to validate old token for refresh: {e}"))
            })?;

        // Generate new token - atomic counter ensures uniqueness
        self.generate_token(user, jwks_manager)
    }

    /// Generate client credentials token with RS256 asymmetric signing
    ///
    /// This method uses RSA private key from JWKS manager for token signing.
    /// Clients can verify tokens using the public key from /.well-known/jwks.json
    ///
    /// No resource owner is involved, so the client is the subject: `sub` and
    /// `client_id` both name it, as RFC 9068 §2.2 has a client-credentials
    /// token do, and [`Claims::client_credentials_client`] reads it back.
    ///
    /// `audience` is the RFC 8707 resource the token is bound to; `None` mints
    /// the platform audience every Pierre surface accepts.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - JWT token generation fails
    /// - System time is unavailable
    /// - JWKS manager has no active key
    pub fn generate_client_credentials_token(
        &self,
        jwks_manager: &JwksManager,
        client_id: &str,
        scopes: &[String],
        active_tenant_id: Option<String>,
        audience: Option<&str>,
    ) -> AppResult<String> {
        let now = Timestamp::now();
        let expiry = now + Elapsed::hours(1); // 1 hour for client credentials

        let claims = Claims {
            sub: client_id.to_owned(),
            email: "client_credentials".to_owned(),
            iat: now.epoch_seconds(),
            exp: expiry.epoch_seconds(),
            iss: self.oauth_token_issuer().to_owned(),
            jti: Uuid::new_v4().to_string(),
            scope: scopes.join(" "),
            aud: audience.unwrap_or(MCP).to_owned(),
            active_tenant_id,
            impersonator_id: None,
            impersonation_session_id: None,
            client_id: Some(client_id.to_owned()),
            grant_id: None,
            client_grant_id: None,
            cnf: None,
            auth_time: None,
        };

        // Get active RSA key from JWKS manager
        let active_key = jwks_manager.get_active_key()?;
        let encoding_key = active_key.encoding_key()?;

        // Create RS256 header with kid
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(active_key.kid.clone());

        let token = encode(&header, &claims, &encoding_key)
            .map_err(|e| AppError::internal(format!("Failed to encode JWT token: {e}")))?;

        Ok(token)
    }
}

/// Generate a random `JWT` secret
///
/// # Errors
/// Returns an error if system RNG fails - this is a critical security failure
/// and the server cannot operate securely without working RNG
pub fn generate_jwt_secret() -> AppResult<[u8; 64]> {
    use ring::rand::{SecureRandom, SystemRandom};

    let rng = SystemRandom::new();
    let mut secret = [0u8; 64];

    rng.fill(&mut secret).map_err(|e| {
        error!(
            "CRITICAL: Failed to generate cryptographically secure JWT secret: {}",
            e
        );
        AppError::internal("System RNG failure - cannot generate secure JWT secret")
    })?;

    Ok(secret)
}

/// The audiences a protected resource known by each of `resources` accepts:
/// the platform audience, then each identifier.
fn resource_audiences<'a>(resources: &[&'a str]) -> Vec<&'a str> {
    iter::once(MCP).chain(resources.iter().copied()).collect()
}
