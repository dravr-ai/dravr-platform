// ABOUTME: Request and response types for authentication routes
// ABOUTME: Defines DTOs for registration, login, OAuth, and user management endpoints
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Authentication request and response types
//!
//! This module contains all DTOs (Data Transfer Objects) used by the authentication
//! routes for serialization and deserialization of API requests and responses.

use pierre_core::models::{DelegationStatus, ProviderAccountRole, UserStatus};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use std::fmt;

/// User registration request
#[derive(Debug, Clone, Deserialize)]
pub struct RegisterRequest {
    /// User's email address
    pub email: String,
    /// User's password (will be hashed)
    pub password: SecretString,
    /// Optional display name for the user
    pub display_name: Option<String>,
}

/// What a new account waits on before it can be used.
///
/// The machine-readable twin of [`RegisterResponse::message`]: the sentence is
/// English, so a client says the same thing in the athlete's own language from
/// this.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum RegistrationActivation {
    /// Nothing: the account is active.
    Active,
    /// The confirmation link mailed to the address activates it.
    ConfirmEmail,
    /// An operator reviews it.
    Review,
}

impl RegistrationActivation {
    /// The English sentence a registration answers with for this outcome.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::Active => "User registered successfully. Your account is ready to use.",
            Self::ConfirmEmail => {
                "User registered successfully. Confirm your email address to activate your account."
            }
            Self::Review => "User registered successfully. Your account is pending admin approval.",
        }
    }
}

/// User registration response
#[derive(Debug, Serialize)]
pub struct RegisterResponse {
    /// Unique identifier for the newly created user
    pub user_id: String,
    /// The address the account signs in with — on an invitation, the one the
    /// invitee never typed
    pub email: String,
    /// User account status at creation time (`pending` or `active`)
    pub user_status: UserStatus,
    /// Display name, if the user supplied one at registration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// What the account waits on, for a client to word in its own language
    pub activation: RegistrationActivation,
    /// Success message for the registration
    pub message: String,
}

/// User login request
#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    /// User's email address
    pub email: String,
    /// User's password
    pub password: SecretString,
    /// Optional IANA timezone (e.g. `"America/Toronto"`). Captured
    /// client-side via `Intl.DateTimeFormat().resolvedOptions().timeZone`
    /// and forwarded on each successful login so the chat prompt can
    /// resolve `{{CURRENT_DATE}}` to the user's local calendar day.
    /// Persisted via `UserRepository::set_timezone` when present.
    #[serde(default)]
    pub timezone: Option<String>,
}

/// Firebase login request - authenticate with Firebase ID token
#[derive(Debug, Deserialize)]
pub struct FirebaseLoginRequest {
    /// Firebase ID token from client-side Firebase SDK
    pub id_token: SecretString,
    /// Optional IANA timezone — see [`LoginRequest::timezone`].
    #[serde(default)]
    pub timezone: Option<String>,
}

/// User info for login response
#[derive(Debug, Serialize)]
pub struct UserInfo {
    /// Primary user identifier (matches frontend User.id)
    pub id: String,
    /// Duplicate user identifier kept for API compatibility with mobile clients
    pub user_id: String,
    /// User's email address
    pub email: String,
    /// User's display name if set
    pub display_name: Option<String>,
    /// Whether the user has admin privileges
    pub is_admin: bool,
    /// User role for permission system (`super_admin`, `admin`, `user`)
    pub role: String,
    /// User account status (`pending`, `active`, `suspended`)
    pub user_status: String,
    /// Active tenant identifier for multi-tenant operations
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tenant_id: Option<String>,
    /// Whether the user has confirmed their email address.
    ///
    /// `None` means this surface did not resolve it — deliberately not `false`,
    /// which would claim the address is unconfirmed when we simply didn't look.
    /// Populated on the login and session-restore paths, which are the ones the
    /// clients read to decide between "confirm your email" and "waiting for
    /// review"; admin listings and other surfaces leave it absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email_verified: Option<bool>,
    /// Why a Pending account waits, when it is because its email belonged to
    /// a deleted account: the live email tombstone the account clears to
    /// continue.
    ///
    /// Resolved on the sign-in and refresh paths for a Pending account only,
    /// the responses the pending screen reads (a Pending account restores no
    /// session); absent everywhere else, and absent when no live tombstone
    /// names the address.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email_tombstone: Option<EmailTombstoneInfo>,
    /// Account creation timestamp (ISO 8601)
    pub created_at: String,
    /// BCP-47 locale preference (fr/en/es/de/pt). Consumed by the web and
    /// mobile Settings UIs so the picker starts at the user's current value.
    pub locale: String,
    /// Coaching persona — output format / cadence preference. Serialized
    /// as `snake_case` enum text so the frontend can drive its persona
    /// picker without translating between `camelCase` and `snake_case`.
    pub coaching_persona: String,
    /// Agent-tier roster access flag — toggled when the user buys / is
    /// granted the Agent persona tier. Surfaces in the Settings UI to
    /// show/hide roster-management screens.
    pub manages_roster: bool,
}

/// The live tombstone a deleted account left on this account's email.
#[derive(Debug, Clone, Serialize)]
pub struct EmailTombstoneInfo {
    /// The tombstone `POST /api/user/import-tombstones/clear` clears.
    pub tombstone_id: String,
    /// When the deleted account's data was deleted (RFC 3339).
    pub deleted_at: String,
}

/// User login response
#[derive(Debug, Serialize)]
pub struct LoginResponse {
    /// JWT authentication token (optional, set in httpOnly cookie)
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(serialize_with = "pierre_core::secret::serialize_exposed_option")]
    pub jwt_token: Option<SecretString>,
    /// CSRF token for request validation (client must include in X-CSRF-Token header)
    #[serde(serialize_with = "pierre_core::secret::serialize_exposed")]
    pub csrf_token: SecretString,
    /// When the token expires (ISO 8601 format)
    pub expires_at: String,
    /// User information
    pub user: UserInfo,
}

/// User profile update request
#[derive(Debug, Deserialize)]
pub struct UpdateProfileRequest {
    /// New display name for the user
    pub display_name: String,
}

/// User profile update response
#[derive(Debug, Serialize)]
pub struct UpdateProfileResponse {
    /// Success message
    pub message: String,
    /// Updated user information
    pub user: UserInfo,
}

/// Analytics consent update request
#[derive(Debug, Deserialize)]
pub struct AnalyticsConsentRequest {
    /// Whether the user consents to anonymized analytics tracking
    pub enabled: bool,
}

/// Locale preference update request
///
/// Accepts a BCP-47 short code (`"fr"`, `"en"`, `"es"`, `"de"`, `"pt"`).
/// The server validates against the set of supported locales; unknown
/// values are rejected with a 400 so the frontend surfaces a clear error
/// rather than silently writing a locale with no compiled-in translations.
#[derive(Debug, Deserialize)]
pub struct UpdateLocaleRequest {
    /// Desired user locale.
    pub locale: String,
}

/// Theme preference update request.
///
/// `"light"` / `"dark"` pin a colour scheme across the user's devices;
/// JSON `null` clears the pin so clients follow the operating system.
/// Anything else is rejected with a 400.
#[derive(Debug, Deserialize)]
pub struct UpdateThemeRequest {
    /// Desired theme pin, or `None` to follow the system.
    pub theme: Option<String>,
}

/// Coaching persona update request.
///
/// Accepts the `snake_case` enum variant — `"casual"`, `"enthusiast"`,
/// `"power_athlete"`, or `"coach"`. Anything outside this set is rejected
/// with a 400 so the frontend surfaces a clear error rather than silently
/// writing a value that won't match any persona block in the LLM prompt.
#[derive(Debug, Deserialize)]
pub struct UpdateCoachingPersonaRequest {
    /// Desired coaching persona.
    pub persona: String,
}

/// Coaching-role update for authenticated users (`PUT /api/user/coaching-role`).
///
/// The onboarding profile-type answer: whether the user coaches others. A
/// role, kept apart from the reply style in [`UpdateCoachingPersonaRequest`]
/// (carnet#827).
#[derive(Debug, Deserialize)]
pub struct UpdateCoachingRoleRequest {
    /// `true` for "I coach others" and "I coach and I train", `false` for
    /// "I'm an athlete".
    pub coaches_others: bool,
}

/// Change password request for authenticated users
#[derive(Debug, Deserialize)]
pub struct ChangePasswordRequest {
    /// Current password for verification
    pub current_password: SecretString,
    /// New password to set
    pub new_password: SecretString,
}

/// Complete a password reset using a one-time reset token
///
/// The token is issued by an admin via the password reset endpoint.
/// Users present the token along with their chosen new password.
#[derive(Debug, Deserialize)]
pub struct CompleteResetRequest {
    /// One-time reset token provided by the admin
    pub reset_token: SecretString,
    /// New password chosen by the user
    pub new_password: SecretString,
}

/// Request to accept an invitation: the invitee chooses their own password.
///
/// Not `Debug`: it carries the invitation token and the chosen password, and a
/// `{:?}` anywhere would log both.
#[derive(Deserialize)]
pub struct AcceptInvitationRequest {
    /// The `<selector>.<verifier>` token from the invitation link
    pub token: String,
    /// The password the invitee chose
    pub password: String,
    /// Optional display name for the account
    pub display_name: Option<String>,
}

impl fmt::Debug for AcceptInvitationRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AcceptInvitationRequest")
            .field("token", &"[REDACTED]")
            .field("password", &"[REDACTED]")
            .field("display_name", &self.display_name)
            .finish()
    }
}

/// Request to initiate a self-service password reset
///
/// Sends a 6-digit code to the user's email address. Always returns success
/// regardless of whether the email exists (anti-enumeration).
#[derive(Debug, Deserialize)]
pub struct ForgotPasswordRequest {
    /// Email address to send the reset code to
    pub email: String,
}

/// Response for the forgot-password endpoint
#[derive(Debug, Serialize)]
pub struct ForgotPasswordResponse {
    /// Human-readable status message (always the same for anti-enumeration)
    pub message: String,
}

/// Session restore response for authenticated users
///
/// Returned by `GET /api/auth/session` to restore sessions using httpOnly cookies.
/// Provides user info and a fresh JWT for WebSocket authentication.
#[derive(Debug, Serialize)]
pub struct SessionResponse {
    /// Authenticated user information
    pub user: UserInfo,
    /// Fresh JWT token for WebSocket authentication (not stored in cookies)
    #[serde(serialize_with = "pierre_core::secret::serialize_exposed")]
    pub access_token: SecretString,
    /// Fresh CSRF token for request validation
    #[serde(serialize_with = "pierre_core::secret::serialize_exposed")]
    pub csrf_token: SecretString,
}

/// User stats response for dashboard
#[derive(Debug, Serialize)]
pub struct UserStatsResponse {
    /// Number of connected fitness providers
    pub connected_providers: i64,
    /// Number of days the user has been active
    pub days_active: i64,
}

/// Logout request body.
///
/// Optional: the web app logs out with no body, because its session is the
/// cookie the response clears. A device holding a refresh token names it
/// here so the server revokes it rather than leaving a live credential
/// behind on a phone that just said goodbye.
#[derive(Debug, Deserialize)]
pub struct LogoutRequest {
    /// The refresh token this device holds, to revoke with its whole family.
    pub refresh_token: Option<SecretString>,
}

/// `OAuth2` token request for first-party clients — RFC 6749 §4.1.3
/// authorization-code grant (with RFC 7636 PKCE) and §6 refresh grant,
/// form-encoded.
///
/// `code`, `redirect_uri` and `code_verifier` belong to the code grant and
/// `refresh_token` to the refresh grant, so each is optional here and the
/// handler requires the ones its grant needs.
#[derive(Deserialize)]
pub struct OAuth2TokenRequest {
    /// Grant type - `authorization_code` to redeem a first-party sign-in,
    /// `refresh_token` to exchange one
    pub grant_type: String,
    /// The authorization code the hosted login page issued
    pub code: Option<String>,
    /// The `redirect_uri` the code was issued to, which it must match
    pub redirect_uri: Option<String>,
    /// The PKCE verifier of the challenge the code was issued for (RFC 7636)
    pub code_verifier: Option<String>,
    /// The refresh token to exchange, for the `refresh_token` grant
    pub refresh_token: Option<SecretString>,
    /// `OAuth2` client identifier: one of Dravr's own apps for the
    /// authorization-code grant (carnet#787); the refresh grant reads none.
    pub client_id: Option<String>,
    /// Requested `OAuth2` scopes (optional, space-separated)
    pub scope: Option<String>,
    /// The iOS app's App Attest key id, beside its attestation or assertion
    /// (carnet#810)
    pub app_attest_key_id: Option<String>,
    /// Apple's attestation of a new key, on an install's first sign-in
    pub app_attest_attestation: Option<String>,
    /// An assertion by the registered key, on every later sign-in
    pub app_attest_assertion: Option<String>,
    /// The Android app's Play Integrity token, requested over the SHA-256 of
    /// the code (carnet#810); never beside the App Attest fields
    pub play_integrity_token: Option<SecretString>,
}

impl fmt::Debug for OAuth2TokenRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OAuth2TokenRequest")
            .field("grant_type", &self.grant_type)
            .field("code", &self.code.as_ref().map(|_| "[REDACTED]"))
            .field("redirect_uri", &self.redirect_uri)
            .field(
                "code_verifier",
                &self.code_verifier.as_ref().map(|_| "[REDACTED]"),
            )
            .field("refresh_token", &self.refresh_token)
            .field("client_id", &self.client_id)
            .field("scope", &self.scope)
            .field("app_attest_key_id", &self.app_attest_key_id)
            .field("app_attest_attestation", &self.app_attest_attestation)
            .field("app_attest_assertion", &self.app_attest_assertion)
            .field("play_integrity_token", &self.play_integrity_token)
            .finish()
    }
}

/// `OAuth2` token response per RFC 6749 Section 5.1
/// Extended with optional user info for frontend compatibility
#[derive(Debug, Serialize)]
pub struct OAuth2TokenResponse {
    /// The access token issued by the authorization server
    #[serde(serialize_with = "pierre_core::secret::serialize_exposed")]
    pub access_token: SecretString,
    /// The type of the token issued (always "Bearer")
    pub token_type: String,
    /// The lifetime in seconds of the access token
    pub expires_in: i64,
    /// Optional refresh token for obtaining new access tokens
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(serialize_with = "pierre_core::secret::serialize_exposed_option")]
    pub refresh_token: Option<SecretString>,
    /// The scope of the access token (space-separated)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    // --- Pierre extensions (allowed per RFC 6749 Section 5.1) ---
    /// User information (Pierre extension for frontend compatibility)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<UserInfo>,
    /// CSRF token for web clients (Pierre extension)
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(serialize_with = "pierre_core::secret::serialize_exposed_option")]
    pub csrf_token: Option<SecretString>,
}

/// `OAuth2` error response per RFC 6749 Section 5.2
#[derive(Debug, Serialize)]
pub struct OAuth2ErrorResponse {
    /// Error code per RFC 6749 (e.g., `invalid_grant`, `invalid_client`)
    pub error: String,
    /// Human-readable description of the error
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_description: Option<String>,
}

/// OAuth authorization response for provider auth URLs
#[derive(Serialize)]
pub struct OAuthAuthorizationResponse {
    /// URL to redirect user to for OAuth authorization
    pub authorization_url: String,
    /// CSRF state token for validating callback
    pub state: String,
    /// Human-readable instructions for the user
    pub instructions: String,
    /// How long the authorization URL is valid (minutes)
    pub expires_in_minutes: i64,
}

impl fmt::Debug for OAuthAuthorizationResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OAuthAuthorizationResponse")
            // The URL carries `state` in its query string.
            .field("authorization_url", &"[REDACTED]")
            .field("state", &"[REDACTED]")
            .field("instructions", &self.instructions)
            .field("expires_in_minutes", &self.expires_in_minutes)
            .finish()
    }
}

/// Connection status for fitness providers
#[derive(Debug, Serialize)]
pub struct ConnectionStatus {
    /// Name of the fitness provider (e.g., "strava", "garmin")
    pub provider: String,
    /// Whether the user is connected to this provider
    pub connected: bool,
    /// How this provider was connected (e.g., "oauth", "synthetic", "manual")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connection_type: Option<String>,
    /// When the connection expires (ISO 8601 format)
    pub expires_at: Option<String>,
    /// Space-separated list of granted OAuth scopes
    pub scopes: Option<String>,
}

/// Provider status for the unified providers endpoint
///
/// Returns all available providers (OAuth and non-OAuth) with their connection status.
/// This is the source of truth for frontend provider displays.
#[derive(Debug, Serialize)]
pub struct ProviderStatus {
    /// Provider identifier (e.g., "strava", "synthetic")
    pub provider: String,
    /// Human-readable display name (e.g., "Strava", "Synthetic")
    pub display_name: String,
    /// The one line under the provider's name, in the reader's locale. Every
    /// surface shows this text as served; none keeps a description of its own.
    pub description: String,
    /// Whether this provider requires OAuth authentication
    pub requires_oauth: bool,
    /// Whether the user is connected to this provider
    pub connected: bool,
    /// The backend whose stored connection serves a connected card — for the
    /// Strava card either `"strava"` (the native OAuth grant) or `"sciotte"`
    /// (the mirror session). A client re-authing the card compares it with
    /// the backend it is about to connect to tell a reconnect from a switch.
    /// Absent while the card is not connected.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connected_backend: Option<String>,
    /// How the connection behind a connected card was made, as
    /// [`ConnectionType`](pierre_core::models::ConnectionType) names it:
    /// `"oauth"` for a provider authorization, `"manual"` for a pasted
    /// credential (an Intervals.icu API key), `"delegated"` for a card read
    /// through a group coach's account. A client reconnecting the card goes
    /// back the same way: an Intervals.icu card linked by API key reconnects
    /// through the key form, since a coach's roster reads need the key and
    /// an authorization cannot replace it. Absent while the card is not
    /// connected.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connection_type: Option<String>,
    /// Whether a connected provider's session is no longer usable and the user
    /// must reconnect (`ConnectionStatus::NeedsReauth`/`Revoked`). Meaningful only
    /// when `connected` is true — a dead scrape session or a non-recoverable OAuth
    /// refresh failure. Lets the connection screen show "Reconnect needed" instead
    /// of a healthy-looking "Connected" for a session that no longer works.
    pub needs_reauth: bool,
    /// Provider capabilities (e.g., `["activities", "sleep"]`)
    pub capabilities: Vec<String>,
    /// For a user-facing provider with more than one auth backend (Strava can be
    /// served by shared-app OAuth or by the Sciotte mirror), the backend a NEW
    /// connection should use: `"oauth"` while shared seats remain, otherwise
    /// `"mirror"`. `None` for single-backend providers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recommended_backend: Option<String>,
    /// Remaining shared-app OAuth athlete seats for this provider when it is
    /// seat-limited (Strava). `None` for providers without a seat cap.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seats_left: Option<u32>,
    /// Whether connecting this provider first needs the account to accept its
    /// exposure notice — TrainingPeaks, until the current notice version is
    /// accepted. The client shows the notice with a required checkbox before
    /// the credentials and posts `tos_consent: true` with the login; the login
    /// is refused without it.
    pub consent_required: bool,
    /// The card's only way in is a credential login that is closed: the
    /// client offers neither a connect nor a reconnect, and shows instead that
    /// signing in is paused. A connected card keeps serving from the session
    /// it already holds. Absent for every card that can still be connected.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub connect_paused: bool,
    /// Connecting needs an OAuth app of the athlete's own: no app of theirs,
    /// of their tenant's or of the server's can authorize this provider for
    /// them, so an OAuth start would reach the provider with no client it
    /// knows. The client opens the own-app setup (`POST
    /// /api/users/oauth-apps`) instead. `false` for a provider without OAuth.
    ///
    /// Intervals.icu has no apps of an athlete's own, and an API key links
    /// it without one: there `true` says only that its authorization is not
    /// offered, and the client opens the key form, never an own-app setup.
    pub own_app_required: bool,
    /// The redirect URL an athlete registers in their own OAuth app for this
    /// provider: this server's callback, the one every authorization
    /// presents. Absent for a provider without OAuth.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oauth_callback_url: Option<String>,
    /// The account's consent to AI use of this provider's data, for a
    /// provider whose notice is one (WHOOP's owner authorization): `true`
    /// while it is given in the current version, `false` while it is not —
    /// never given, outdated or withdrawn — and then no model reads this
    /// provider's data. Withdrawn with `DELETE` and given again with `PUT` on
    /// `/api/providers/{provider}/ai-consent`. Absent for every provider that
    /// asks no such consent of this account.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ai_consent: Option<bool>,
    /// What kind of account the user's own connection signed in with, once
    /// the provider has reported it: `"coach"` for a TrainingPeaks coach
    /// account, which keeps no calendar of its own, so the card can say where
    /// its athletes' workouts are read instead. Absent until the role is read,
    /// and for every provider that reports none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_role: Option<ProviderAccountRole>,
    /// The link through which the user's group coach reads this provider for
    /// them — a coaching platform (TrainingPeaks, Intervals.icu), read through
    /// the coach's own account once the user confirms. The confirmed link,
    /// else the newest one awaiting the user's answer. Absent when there is
    /// none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delegation: Option<ProviderDelegation>,
}

/// A group coach's link to the user's provider, as the provider card shows it.
#[derive(Debug, Serialize)]
pub struct ProviderDelegation {
    /// The link's id, which the group's link routes take
    pub connection_id: String,
    /// The group the link was made in
    pub group_id: String,
    /// The group's name
    pub group_name: String,
    /// The coach's name: their display name, else their email
    pub coach_display_name: String,
    /// `proposed` (waiting for the user) or `confirmed`
    pub status: DelegationStatus,
    /// Whether the coach must reconnect their own account before the user's
    /// workouts can be read again. The user has nothing to reconnect.
    pub coach_needs_reauth: bool,
    /// Why a confirmed link reads nothing until the coach links the user
    /// again: `athlete_email_missing`, `athlete_email_mismatch` or
    /// `member_email_unverified`. `null` while it reads, and for a proposal.
    pub read_refused: Option<&'static str>,
}

/// Response for the /api/providers endpoint
#[derive(Debug, Serialize)]
pub struct ProvidersStatusResponse {
    /// List of all available providers with their status
    pub providers: Vec<ProviderStatus>,
}
