// ABOUTME: OAuth token models for secure credential storage and notifications
// ABOUTME: UserOAuthToken, UserOAuthApp, OAuthNotification, and session types
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use secrecy::SecretString;
use std::fmt;

use dravr_tronc::dates::Timestamp;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::constants::time::TOKEN_REFRESH_WINDOW_MINUTES;
use crate::errors::{AppError, AppResult};
use dravr_tronc::dates::Elapsed;

/// Whether a provider access token expiring at `expires_at` is due for
/// refresh now: it expires within [`TOKEN_REFRESH_WINDOW_MINUTES`].
///
/// Every refresh decision on a provider token reads this one window: the
/// platform's stored-token refresh and each provider client's own refresh.
#[must_use]
pub fn refresh_due(expires_at: Timestamp) -> bool {
    expires_at <= Timestamp::now() + Elapsed::minutes(TOKEN_REFRESH_WINDOW_MINUTES)
}

/// OAuth application credentials for protocol initialization (MCP and A2A)
///
/// Transmitted by clients during protocol handshake to provide OAuth
/// application credentials for specific providers. Used by both the MCP
/// and A2A protocol initialization flows.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthAppCredentials {
    /// OAuth client ID
    #[serde(rename = "clientId")]
    pub client_id: String,
    /// OAuth client secret
    #[serde(rename = "clientSecret")]
    #[serde(serialize_with = "crate::secret::serialize_exposed")]
    pub client_secret: SecretString,
}

/// The `token_type` a stored row carries when its `access_token` is a personal
/// API key the athlete pasted rather than an OAuth grant (intervals.icu).
pub const API_KEY_TOKEN_TYPE: &str = "api_key";

/// User OAuth token for tenant-provider combination
///
/// Stores user's personal OAuth tokens for accessing fitness providers
/// within their tenant's application context. Each user can have one token
/// per tenant-provider combination (e.g., user's Strava token in tenant A).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserOAuthToken {
    /// Unique identifier for this token record
    pub id: String,
    /// User who owns this token
    pub user_id: Uuid,
    /// Tenant context for this token
    pub tenant_id: String,
    /// Provider name (strava, whoop, etc.)
    pub provider: String,
    /// Encrypted OAuth access token
    #[serde(serialize_with = "crate::secret::serialize_exposed")]
    pub access_token: SecretString,
    /// Encrypted OAuth refresh token (optional for some providers)
    #[serde(serialize_with = "crate::secret::serialize_exposed_option")]
    pub refresh_token: Option<SecretString>,
    /// Token type (usually "Bearer")
    pub token_type: String,
    /// When the access token expires
    pub expires_at: Option<Timestamp>,
    /// Granted OAuth scopes
    pub scope: Option<String>,
    /// The user's identifier on the provider side, when the provider needs one
    /// distinct from the access token (e.g. Intervals.icu's `athlete_id`, used
    /// as the HTTP Basic username). `None` for pure OAuth bearer providers.
    /// Stored in plaintext — it is not a secret and is queryable.
    pub provider_user_id: Option<String>,
    /// For a Strava token minted through the shared-app OAuth pool, the
    /// `client_id` of the pool app that issued it — so token refresh uses that
    /// app's `client_secret`. `None` means the env-default `STRAVA_CLIENT_ID`
    /// app (or a pre-pool legacy token), which refreshes with the env secret.
    /// Not a secret: `client_id` is public (it appears in the authorize URL).
    pub oauth_app_client_id: Option<String>,
    /// When this token was first stored
    pub created_at: Timestamp,
    /// When this token was last updated
    pub updated_at: Timestamp,
}

impl UserOAuthToken {
    /// Create a new user OAuth token
    #[must_use]
    pub fn new(
        user_id: Uuid,
        tenant_id: String,
        provider: String,
        access_token: String,
        refresh_token: Option<String>,
        expires_at: Option<Timestamp>,
        scope: Option<String>,
    ) -> Self {
        let now = Timestamp::now();
        Self {
            id: Uuid::new_v4().to_string(),
            user_id,
            tenant_id,
            provider,
            access_token: SecretString::from(access_token),
            refresh_token: refresh_token.map(SecretString::from),
            token_type: "Bearer".to_owned(),
            expires_at,
            scope,
            provider_user_id: None,
            oauth_app_client_id: None,
            created_at: now,
            updated_at: now,
        }
    }

    /// Attach the provider-side user identifier (e.g. Intervals.icu `athlete_id`).
    ///
    /// Builder-style so the common OAuth path keeps calling [`Self::new`]
    /// unchanged; only API-key providers that carry a separate user id set this.
    #[must_use]
    pub fn with_provider_user_id(mut self, provider_user_id: impl Into<String>) -> Self {
        self.provider_user_id = Some(provider_user_id.into());
        self
    }

    /// Attach the shared-app pool `client_id` that issued this Strava token, so
    /// refresh resolves the matching `client_secret`. Builder-style: the
    /// env-default app leaves it `None`.
    #[must_use]
    pub fn with_oauth_app_client_id(mut self, client_id: Option<String>) -> Self {
        self.oauth_app_client_id = client_id;
        self
    }

    /// Check if the access token is expired
    #[must_use]
    pub fn is_expired(&self) -> bool {
        self.expires_at
            .is_some_and(|expires_at| Timestamp::now() > expires_at)
    }
}

/// User OAuth app credentials for cloud deployment
///
/// Each user can configure their own OAuth application credentials
/// for each provider (Strava, WHOOP, etc.) to work in cloud deployments
/// where server-wide environment variables won't work.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserOAuthApp {
    /// Unique identifier for this OAuth app configuration
    pub id: String,
    /// User who owns this OAuth app configuration
    pub user_id: Uuid,
    /// OAuth provider name (strava, whoop, etc.)
    pub provider: String,
    /// OAuth client ID from the provider
    pub client_id: String,
    /// OAuth client secret from the provider (encrypted)
    #[serde(serialize_with = "crate::secret::serialize_exposed")]
    pub client_secret: SecretString,
    /// OAuth redirect URI configured with the provider
    pub redirect_uri: String,
    /// When this configuration was created
    pub created_at: Timestamp,
    /// When this configuration was last updated
    pub updated_at: Timestamp,
}

impl UserOAuthApp {
    /// Create a new user OAuth app configuration
    #[must_use]
    pub fn new(
        user_id: Uuid,
        provider: String,
        client_id: String,
        client_secret: String,
        redirect_uri: String,
    ) -> Self {
        let now = Timestamp::now();
        Self {
            id: Uuid::new_v4().to_string(),
            user_id,
            provider,
            client_id,
            client_secret: SecretString::from(client_secret),
            redirect_uri,
            created_at: now,
            updated_at: now,
        }
    }
}

/// User session for `MCP` protocol authentication
///
/// Contains `JWT` token and user context for secure `MCP` communication.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserSession {
    /// User `ID` this session belongs to
    pub user_id: Uuid,
    /// `JWT` token for authentication
    #[serde(serialize_with = "crate::secret::serialize_exposed")]
    pub jwt_token: SecretString,
    /// When the session expires
    pub expires_at: Timestamp,
    /// User's email for display
    pub email: String,
}

/// Authentication request for `MCP` protocol
///
/// Clients send this to authenticate with the `MCP` server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthRequest {
    /// `JWT` token for authentication
    #[serde(serialize_with = "crate::secret::serialize_exposed")]
    pub token: SecretString,
}

/// Authentication response for `MCP` protocol
///
/// Server responds with user context and available capabilities.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthResponse {
    /// Whether authentication was successful
    pub authenticated: bool,
    /// User `ID` if authenticated
    pub user_id: Option<Uuid>,
    /// Error message if authentication failed
    pub error: Option<String>,
}

/// Type of provider connection
///
/// Distinguishes how a provider was connected to enable type-specific behavior
/// (e.g., OAuth connections have tokens that expire, synthetic connections do not).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionType {
    /// Connected via OAuth 2.0 token exchange
    OAuth,
    /// Connected via synthetic/test data seeding
    Synthetic,
    /// Connected via manual configuration
    Manual,
    /// Served through another user's credential: a member's TrainingPeaks
    /// read through the session of the coach a confirmed
    /// `delegated_connections` link names. The member holds no token of
    /// their own for it.
    Delegated,
}

impl ConnectionType {
    /// Convert from database string representation
    ///
    /// # Errors
    ///
    /// Returns `AppError` if the string is not a valid connection type.
    pub fn from_str_value(s: &str) -> AppResult<Self> {
        match s {
            "oauth" => Ok(Self::OAuth),
            "synthetic" => Ok(Self::Synthetic),
            "manual" => Ok(Self::Manual),
            "delegated" => Ok(Self::Delegated),
            other => Err(AppError::invalid_input(format!(
                "Unknown connection type: {other}"
            ))),
        }
    }

    /// Convert to database string representation
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::OAuth => "oauth",
            Self::Synthetic => "synthetic",
            Self::Manual => "manual",
            Self::Delegated => "delegated",
        }
    }
}

impl fmt::Display for ConnectionType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What kind of account a provider connection signed in with, as the
/// provider itself reports it.
///
/// A TrainingPeaks coach account keeps no training calendar of its own, so a
/// read of the account's own workouts has nothing to return; knowing the role
/// lets a read refuse in words before any scrape, and lets the recency
/// election prefer the user's other connections. `None` on a connection means
/// the role has not been read yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderAccountRole {
    /// The account's owner trains: it has a calendar of its own.
    Athlete,
    /// The account coaches others and has no calendar of its own.
    Coach,
}

impl ProviderAccountRole {
    /// Convert from the database string representation; `None` for a value
    /// outside the vocabulary.
    #[must_use]
    pub fn from_str_opt(s: &str) -> Option<Self> {
        match s {
            "athlete" => Some(Self::Athlete),
            "coach" => Some(Self::Coach),
            _ => None,
        }
    }

    /// Convert to the database string representation.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Athlete => "athlete",
            Self::Coach => "coach",
        }
    }
}

impl fmt::Display for ProviderAccountRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Lifecycle status of a provider connection.
///
/// Presence of a `provider_connections` row means the provider was connected at some
/// point; `ConnectionStatus` distinguishes a usable connection from one whose OAuth
/// token refresh failed non-recoverably (`NeedsReauth`) or that the user/provider
/// revoked (`Revoked`). Read by the connection-status tool, the group context builder,
/// and the web/mobile connection screens so a dead connection stops looking healthy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionStatus {
    /// Connection is usable: token valid or refreshable.
    Active,
    /// Token refresh failed non-recoverably; the user must re-authorize the provider.
    NeedsReauth,
    /// The user or provider revoked access (e.g. WHOOP `user.deauthorized` webhook).
    Revoked,
}

impl ConnectionStatus {
    /// Convert from database string representation. Unknown values fall back to
    /// `Active` so a forward-compatible status written by a newer node never makes
    /// an existing connection read as broken.
    #[must_use]
    pub fn from_str_value(s: &str) -> Self {
        match s {
            "needs_reauth" => Self::NeedsReauth,
            "revoked" => Self::Revoked,
            _ => Self::Active,
        }
    }

    /// Convert to database string representation.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::NeedsReauth => "needs_reauth",
            Self::Revoked => "revoked",
        }
    }

    /// Whether this status requires the user to re-authorize before data can flow.
    #[must_use]
    pub const fn requires_reauth(&self) -> bool {
        matches!(self, Self::NeedsReauth | Self::Revoked)
    }
}

impl fmt::Display for ConnectionStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The reason recorded on a connection whose athlete withdrew, at the
/// provider, the permission the grant exists for (Garmin's activity export).
///
/// Only a reconnect, or the provider reporting the permission granted again,
/// clears it: a token refresh proves the grant alive, never the permission
/// back (carnet#1386).
pub const PERMISSION_WITHDRAWN: &str = "permission_withdrawn";

/// What flagging a connection `needs_reauth` after a failed attempt found.
///
/// The flag is guarded on when the attempt began, so a caller that goes on to
/// tell the athlete to reconnect has to know which of these it got: a
/// connection reconnected while the attempt ran is healthy, and a reconnect
/// prompt sent over it contradicts the connection the app shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReauthMark {
    /// The connection flipped to `needs_reauth`.
    Flagged,
    /// It already required re-authorizing (`needs_reauth` or `revoked`).
    AlreadyFlagged,
    /// It was reconnected or re-armed after the attempt began, and stands
    /// active: the failure was the credential the attempt read, not its own.
    ReconnectedSince,
    /// The user has no such connection.
    NoConnection,
}

/// Provider connection record: single source of truth for provider connectivity
///
/// Tracks whether a provider (OAuth, synthetic, or manual) is connected for a user.
/// All provider types register in this table, eliminating the need to query
/// separate tables (`user_oauth_tokens`, `synthetic_activities`) for connection status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConnection {
    /// Unique identifier for this connection record
    pub id: String,
    /// User who owns this connection
    pub user_id: Uuid,
    /// Tenant context for multi-tenant isolation
    pub tenant_id: String,
    /// Provider name (e.g., "strava", "garmin", "synthetic")
    pub provider: String,
    /// How this provider was connected
    pub connection_type: ConnectionType,
    /// When the connection was established
    pub connected_at: Timestamp,
    /// Most recent time this provider actually served data for the user (chat tool reads,
    /// REST activity fetches). Distinct from `connected_at` (one-time registration) and
    /// from `user_oauth_tokens.last_sync` (background sync orchestrator). Drives the
    /// per-user resolver that picks the active backend when an LLM tool call omits the
    /// provider argument.
    pub last_used_at: Option<Timestamp>,
    /// Lifecycle status: usable, needs re-auth, or revoked. Defaults to `Active` for
    /// freshly registered connections and flips to `NeedsReauth` when a token refresh
    /// fails non-recoverably.
    pub status: ConnectionStatus,
    /// Optional JSON metadata (e.g., {"source": "seed-synthetic-activities"})
    pub metadata: Option<String>,
    /// The kind of account the connection signed in with, once read from the
    /// provider; `None` until then. Cleared on every (re)connect, since a new
    /// login may be a different account.
    pub account_role: Option<ProviderAccountRole>,
    /// When `status` last changed — for a `needs_reauth` connection, when the
    /// provider started refusing it. `None` for a row whose status never
    /// changed. Read from the database only; not on any wire.
    #[serde(skip)]
    pub status_changed_at: Option<Timestamp>,
}

impl ProviderConnection {
    /// Create a new provider connection record
    #[must_use]
    pub fn new(
        user_id: Uuid,
        tenant_id: String,
        provider: String,
        connection_type: ConnectionType,
    ) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            user_id,
            tenant_id,
            provider,
            connection_type,
            connected_at: Timestamp::now(),
            last_used_at: None,
            status: ConnectionStatus::Active,
            metadata: None,
            account_role: None,
            status_changed_at: None,
        }
    }
}

/// Whether any connection for `provider` in `connections` requires the user to
/// re-authenticate (status `NeedsReauth` or `Revoked`).
///
/// The single home for the "does this provider need reconnecting?" scan: the
/// chat `get_activities` reconnect gate and `AuthService` both call it so the
/// predicate (which statuses count, via [`ConnectionStatus::requires_reauth`])
/// and the per-provider match live in one place.
#[must_use]
pub fn connection_needs_reauth(connections: &[ProviderConnection], provider: &str) -> bool {
    connections
        .iter()
        .any(|c| c.provider == provider && c.status.requires_reauth())
}

/// OAuth notification data structure for tracking OAuth flow completion events
///
/// Used to deliver asynchronous notifications to users about OAuth connection
/// status changes (success/failure of provider connections).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthNotification {
    /// Unique notification ID
    pub id: String,
    /// User ID this notification belongs to
    pub user_id: String,
    /// Provider name (e.g., "strava", "whoop")
    pub provider: String,
    /// Whether OAuth flow succeeded
    pub success: bool,
    /// Notification message text
    pub message: String,
    /// Optional expiration timestamp as ISO 8601 string
    pub expires_at: Option<String>,
    /// When the notification was created
    pub created_at: Timestamp,
    /// When the notification was read (if read)
    pub read_at: Option<Timestamp>,
}

/// A platform-owned Strava OAuth app in the shared-app pool, registered beside
/// the env `STRAVA_CLIENT_ID` app.
///
/// Each application serves a bounded number of athletes.
///
/// The `client_secret` is intentionally absent — it is stored encrypted and is
/// only ever fetched (decrypted) at token exchange/refresh via the repository's
/// `get_strava_pool_app_secret`, never handed around in this struct. `client_id`
/// is public (it appears in the authorize URL). Timestamps are Unix epoch
/// seconds, matching the storage column so reads/writes are identical across
/// backends.
#[derive(Debug, Clone)]
pub struct StravaPoolApp {
    /// Strava application client id (public; primary key in the pool table).
    pub client_id: String,
    /// Strava-approved athlete cap for this app.
    pub seat_cap: u32,
    /// Whether this app is eligible for new connections.
    pub enabled: bool,
    /// Optional admin label (e.g. "dravr-app-2").
    pub label: Option<String>,
    /// Row creation time, Unix epoch seconds.
    pub created_at: i64,
    /// Row last-update time, Unix epoch seconds.
    pub updated_at: i64,
}

/// The Strava app an athlete's stored token names, the tenant it is stored
/// in, and whether that token still holds its seat there (the seat counts'
/// rule, read per athlete).
///
/// The authorize path reads it to keep an athlete on the app Strava already
/// counts them on: a grant that holds a seat costs nothing more to reconnect
/// on, where one that does not has to find room like anyone new. A reconnect
/// reads it to tell a grant Strava still counts from a dead one before it
/// revokes anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StravaTokenApp {
    /// Tenant the token is stored in.
    pub tenant_id: String,
    /// Pool app that issued the token; `None` is the env-default app.
    pub attribution: Option<String>,
    /// Whether the token holds a shared-app seat, by the filter the counts
    /// apply.
    pub holds_seat: bool,
    /// Whether the token's grant is still authorized at Strava, by the rule
    /// the seat filter applies, a user's own OAuth app aside: a user with one
    /// holds no seat and still holds a live grant.
    pub grant_live: bool,
}

/// One stored Strava token and whether it holds a shared-app seat.
///
/// The per-holder view behind the seat counts: the counts answer "how full is
/// each app", this answers "who is in it". `counts_as_seat` applies the same
/// rule the counts do, from the same SQL filter, so a listing and a count can
/// never disagree about one athlete.
#[derive(Debug, Clone)]
pub struct StravaSeatHolder {
    /// The athlete's user id.
    pub user_id: Uuid,
    /// The athlete's account email; `None` for a token whose account row is
    /// gone, which the seat counts still include.
    pub email: Option<String>,
    /// Tenant the token is stored under.
    pub tenant_id: String,
    /// Pool app that issued the token; `None` is the env-default app.
    pub oauth_app_client_id: Option<String>,
    /// Status of the matching provider connection; `None` when the token has
    /// no connection row (it still counts as a seat).
    pub connection_status: Option<ConnectionStatus>,
    /// When the athlete connected: the connection's `connected_at`, else the
    /// token's `created_at` for a token with no connection row.
    pub connected_at: Timestamp,
    /// When the athlete was last active on Dravr; `None` when the account row
    /// is gone.
    ///
    /// This is `users.last_active`: written by every login and session
    /// refresh, by every request the auth middleware authenticates (a session
    /// or `OAuth2`-connector JWT, or an API key; at most every few minutes per
    /// athlete) and by every messaging turn. The seat-reclaim sweeper measures
    /// idle time from it.
    pub last_active: Option<Timestamp>,
    /// Whether this token holds a seat on the shared app. False for a BYO-app
    /// user, a `revoked` connection, and a `needs_reauth` one for any reason
    /// but our own client credentials; a `needs_reauth` over those still holds
    /// its seat.
    pub counts_as_seat: bool,
}

/// The warning the Strava seat-reclaim sweeper sent about one athlete's seat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StravaSeatReclaimWarning {
    /// When it was sent.
    pub warned_at: Timestamp,
    /// Whether it reached the athlete outside the app: a push to at least one
    /// device, or a message on at least one linked chat channel. A warning
    /// only persisted in the in-app list reached nobody, since an idle athlete
    /// by definition does not open the app, so it never justifies a
    /// disconnect.
    pub reached: bool,
}

#[cfg(test)]
mod tests {
    use super::refresh_due;
    use dravr_tronc::dates::{Elapsed, Timestamp};

    /// The window is ten minutes: a token expiring in nine is due, one
    /// expiring in eleven is not, and an expired one is due.
    #[test]
    fn refresh_is_due_inside_the_ten_minute_window_only() {
        assert!(refresh_due(Timestamp::now() + Elapsed::minutes(9)));
        assert!(!refresh_due(Timestamp::now() + Elapsed::minutes(11)));
        assert!(refresh_due(Timestamp::now() - Elapsed::minutes(1)));
    }
}
