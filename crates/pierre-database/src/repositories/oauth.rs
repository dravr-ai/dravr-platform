// ABOUTME: Repository trait definitions for the OAuth tokens, OAuth2 server, client state, provider connections domain
// ABOUTME: Split out of repositories.rs as part of Finding B (per-domain repository modules)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::AppResult;

use pierre_core::models::TenantId;
use pierre_core::models::{
    ConnectionType, DeviceAuthorization, OAuth2AuthCode, OAuth2Client, OAuth2ClientSweep,
    OAuth2RefreshToken, OAuth2State, OAuthClientGrant, OAuthClientState, ProviderAccountRole,
    ProviderConnection, ReauthMark, StravaPoolApp, StravaSeatHolder, StravaTokenApp, UserOAuthApp,
    UserOAuthToken,
};
use uuid::Uuid;

/// OAuth token storage repository (tenant-scoped, includes OAuth apps and sync tracking)
#[async_trait]
pub trait OAuthTokenRepository: Send + Sync {
    /// Store or update user OAuth token for a tenant-provider combination
    async fn upsert_token(&self, token: &UserOAuthToken) -> AppResult<()>;
    /// Store `token` only while the row it replaces is still the one the
    /// caller read: none at all when `expected_id` is `None`, else the row
    /// whose `id` is `expected_id` (every store writes a fresh `id`, so a row
    /// another writer stored since carries a different one). Returns whether
    /// the token was stored; `false` means another write landed first and
    /// nothing changed.
    async fn replace_token_if_current(
        &self,
        token: &UserOAuthToken,
        expected_id: Option<&str>,
    ) -> AppResult<bool>;
    /// Get user OAuth token for a specific tenant-provider combination
    async fn get_token(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) -> AppResult<Option<UserOAuthToken>>;
    /// Get all OAuth tokens for a user, optionally scoped to a specific tenant
    async fn get_tokens(
        &self,
        user_id: Uuid,
        tenant_id: Option<TenantId>,
    ) -> AppResult<Vec<UserOAuthToken>>;
    /// Get all OAuth tokens for a tenant-provider combination
    async fn get_tenant_provider_tokens(
        &self,
        tenant_id: TenantId,
        provider: &str,
    ) -> AppResult<Vec<UserOAuthToken>>;
    /// Resolve the single user who owns a provider-side account id.
    ///
    /// Maps a provider's own user identifier (e.g. a Strava athlete id delivered
    /// in a webhook `owner_id`) back to the `(user_id, tenant_id)` of the matching
    /// stored token. Returns `None` when no token carries that `provider_user_id`,
    /// which callers must treat as "unknown owner" — never as a signal to fan out
    /// to every connected user.
    async fn find_user_by_provider_user_id(
        &self,
        provider: &str,
        provider_user_id: &str,
    ) -> AppResult<Option<(Uuid, String)>>;
    /// List Strava shared-app pool apps — the extra DB-configured apps beside
    /// the env `STRAVA_CLIENT_ID` app. Secrets are never included. When
    /// `only_enabled` is true, disabled apps are omitted (the connect-selection
    /// path); otherwise all rows are returned (admin listing).
    async fn list_strava_pool_apps(&self, only_enabled: bool) -> AppResult<Vec<StravaPoolApp>>;

    /// Decrypt and return a pool app's `client_secret`, or `None` when the
    /// `client_id` is not in the pool. Used at token exchange and refresh to use
    /// the same app that minted the token.
    async fn get_strava_pool_app_secret(&self, client_id: &str) -> AppResult<Option<String>>;

    /// Distinct-user seat usage grouped by the issuing Strava app. Each entry
    /// is `(oauth_app_client_id, distinct_user_count)`; the `None` key is the
    /// env-default app (NULL attribution + legacy tokens).
    ///
    /// A "seat" is one athlete whose grant on the platform's shared Strava
    /// application is still authorized. Users who registered their own (BYO)
    /// OAuth app run on their own athlete quota and hold none, and neither does
    /// a token whose connection is `revoked` or is `needs_reauth` for any
    /// reason but our own client credentials; a `needs_reauth` over those
    /// leaves the grant live at Strava and keeps its seat. The
    /// count is intentionally cross-tenant: the shared app's athlete cap is a
    /// single global limit enforced upstream across every tenant that uses it.
    /// `excluded_user`, when set, is left out of every bucket, so the authorize
    /// path can score an athlete's reconnect against everyone else.
    async fn count_strava_seat_usage_by_app(
        &self,
        excluded_user: Option<Uuid>,
    ) -> AppResult<Vec<(Option<String>, u32)>>;

    /// The Strava app each of a user's stored tokens names, the tenant it is
    /// stored in and whether it holds a seat, read from the attribution column
    /// without decrypting the tokens: tokens holding a seat first, then the
    /// one stored in `tenant_id`, then the most recently written, since Strava
    /// counts the athlete per app whatever our tenant. Empty when the user
    /// holds no Strava token.
    ///
    /// `holds_seat` is the seat counts' own filter, so an athlete this names
    /// as holding a seat is one those counts include.
    async fn list_strava_token_apps(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Vec<StravaTokenApp>>;

    /// Every stored Strava token with its holder's email, the issuing app, the
    /// matching connection's status and whether it holds a shared-app seat —
    /// the per-athlete view behind [`Self::count_strava_seat_usage_by_app`].
    ///
    /// `counts_as_seat` is computed by the same filter those counts apply, so
    /// the listing and the counts agree on every athlete. Cross-tenant, like
    /// the counts: the seat cap is one limit across every tenant.
    async fn list_strava_seat_holders(&self) -> AppResult<Vec<StravaSeatHolder>>;

    /// The `(tenant_id, provider)` of every token a user holds, across every
    /// tenant, read without decrypting the tokens.
    ///
    /// The enumeration a complete removal walks: a token that no longer
    /// decrypts is still listed, so the disconnect that clears it still runs.
    async fn list_token_providers(&self, user_id: Uuid) -> AppResult<Vec<(String, String)>>;

    /// Insert or update a pool app, encrypting `client_secret` at rest with the
    /// same AES-256-GCM envelope used for user tokens.
    async fn upsert_strava_pool_app(
        &self,
        client_id: &str,
        client_secret: &str,
        seat_cap: u32,
        label: Option<&str>,
    ) -> AppResult<()>;

    /// Enable or disable a pool app; disabled apps are skipped for new connects.
    async fn set_strava_pool_app_enabled(&self, client_id: &str, enabled: bool) -> AppResult<()>;

    /// Remove a pool app. Tokens it already issued keep their attribution and
    /// their refresh will fail once the secret is gone — only delete an app
    /// whose athletes have migrated or disconnected.
    async fn delete_strava_pool_app(&self, client_id: &str) -> AppResult<()>;

    /// Delete user OAuth token for a tenant-provider combination
    async fn delete_token(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) -> AppResult<()>;
    /// Delete all OAuth tokens for a user within a tenant scope
    async fn delete_tokens(&self, user_id: Uuid, tenant_id: TenantId) -> AppResult<()>;
    /// Store a refreshed access and refresh token over `stored`, the row the
    /// refresh read; the row keeps its `id` and every other column.
    ///
    /// Returns `false`, writing nothing, when that row is no longer the one
    /// stored for its user, tenant and provider: a reconnect replaced it
    /// (every store writes a fresh `id`) while the refresh was in flight, and
    /// the refreshed pair belongs to the grant the reconnect superseded.
    async fn refresh_token(
        &self,
        stored: &UserOAuthToken,
        access_token: &str,
        refresh_token: Option<&str>,
        expires_at: Option<DateTime<Utc>>,
    ) -> AppResult<bool>;
    /// Store user OAuth app credentials (`client_id`, `client_secret`)
    async fn store_user_oauth_app(
        &self,
        user_id: Uuid,
        provider: &str,
        client_id: &str,
        client_secret: &str,
        redirect_uri: &str,
    ) -> AppResult<()>;
    /// Get user OAuth app credentials for a provider
    async fn get_user_oauth_app(
        &self,
        user_id: Uuid,
        provider: &str,
    ) -> AppResult<Option<UserOAuthApp>>;
    /// List all OAuth app providers configured for a user
    async fn list_user_oauth_apps(&self, user_id: Uuid) -> AppResult<Vec<UserOAuthApp>>;
    /// Remove user OAuth app credentials for a provider
    async fn remove_user_oauth_app(&self, user_id: Uuid, provider: &str) -> AppResult<()>;
    /// Get last sync timestamp for a provider within a specific tenant
    async fn get_provider_last_sync(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) -> AppResult<Option<DateTime<Utc>>>;
    /// Update last sync timestamp for a provider within a specific tenant
    async fn update_provider_last_sync(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        sync_time: DateTime<Utc>,
    ) -> AppResult<()>;
}

/// OAuth 2.0 server repository (RFC 7591)
#[async_trait]
pub trait OAuth2ServerRepository: Send + Sync {
    /// Store an OAuth 2.0 client registration unless `ceiling` registrations
    /// are already pending.
    ///
    /// A registration is pending while it carries an `expires_at` (every row
    /// dynamic registration writes does) and no refresh token has ever been
    /// issued through it. The count and the insert are one statement: exact on
    /// `SQLite`, whose writers are serialized; on `PostgreSQL` registrations
    /// racing for the last slot can each see it free, so the ceiling can be
    /// passed by at most the number of registrations in flight at once.
    /// Returns `false`, storing nothing, when the ceiling is reached; a
    /// ceiling of `0` therefore refuses every registration.
    async fn store_client_within_ceiling(
        &self,
        client: &OAuth2Client,
        ceiling: u64,
    ) -> AppResult<bool>;
    /// Delete the client registrations retention no longer keeps: those whose
    /// `expires_at` is before `expired_before`, and those still pending (see
    /// [`store_client_within_ceiling`](Self::store_client_within_ceiling))
    /// that were created before `unauthorized_before`. A row without an
    /// `expires_at` was not written by dynamic registration and is never
    /// deleted here. Codes, refresh tokens and states go with their client by
    /// cascade; consent grants naming a client that no longer exists are
    /// deleted in the same transaction.
    async fn delete_stale_clients(
        &self,
        expired_before: DateTime<Utc>,
        unauthorized_before: DateTime<Utc>,
    ) -> AppResult<OAuth2ClientSweep>;
    /// Get OAuth 2.0 client by `client_id`
    async fn get_client(&self, client_id: &str) -> AppResult<Option<OAuth2Client>>;
    /// Store OAuth 2.0 authorization code
    async fn store_auth_code(&self, auth_code: &OAuth2AuthCode) -> AppResult<()>;
    /// Store OAuth 2.0 refresh token, and stamp its client's
    /// `last_authorized_at` with the token's `created_at` in the same
    /// transaction — a refresh token is only ever issued for a user, so this is
    /// the record that a user authorized the client, and what takes the client
    /// out of the pending set.
    async fn store_refresh_token(&self, refresh_token: &OAuth2RefreshToken) -> AppResult<()>;
    /// Atomically consume OAuth 2.0 authorization code (check-and-set in single operation)
    async fn consume_auth_code(
        &self,
        code: &str,
        client_id: &str,
        redirect_uri: &str,
        now: DateTime<Utc>,
    ) -> AppResult<Option<OAuth2AuthCode>>;
    /// Atomically consume OAuth 2.0 refresh token (check-and-revoke in single operation)
    async fn consume_refresh_token(
        &self,
        token: &str,
        client_id: &str,
        now: DateTime<Utc>,
    ) -> AppResult<Option<OAuth2RefreshToken>>;
    /// Look up a refresh token by its value (without `client_id` constraint)
    async fn get_refresh_token_by_value(
        &self,
        token: &str,
    ) -> AppResult<Option<OAuth2RefreshToken>>;
    /// Store `OAuth2` state for CSRF protection
    async fn store_state(&self, state: &OAuth2State) -> AppResult<()>;
    /// Consume `OAuth2` state (atomically check and mark as used)
    async fn consume_state(
        &self,
        state_value: &str,
        client_id: &str,
        now: DateTime<Utc>,
    ) -> AppResult<Option<OAuth2State>>;
    /// Persist a user's consent to an MCP OAuth client.
    ///
    /// Inserts a new active grant with `granted_at = now` and `revoked_at = NULL`.
    /// The caller supplies `grant.id` (a uuid string). If an active grant for the
    /// same `(user_id, tenant_id, client_id, scope)` already exists, the insert is
    /// a no-op (the active partial-unique index makes re-consent idempotent).
    async fn store_client_grant(&self, grant: &OAuthClientGrant) -> AppResult<()>;
    /// Find the active grant for a `(user, tenant, client, scope)` tuple.
    ///
    /// Returns the un-revoked grant matching all four fields, or `None` when no
    /// active grant exists — the authorize path treats `None` as "show the
    /// consent screen".
    async fn find_active_client_grant(
        &self,
        user_id: &str,
        tenant_id: &str,
        client_id: &str,
        scope: &str,
    ) -> AppResult<Option<OAuthClientGrant>>;
    /// List a user's active (un-revoked) client grants within a tenant.
    ///
    /// Ordered by `granted_at` descending (most recent first). Backs the user's
    /// "connected apps" view.
    async fn list_client_grants(
        &self,
        user_id: &str,
        tenant_id: &str,
    ) -> AppResult<Vec<OAuthClientGrant>>;
    /// Revoke a client grant, verifying ownership via `user_id` + `tenant_id`.
    ///
    /// Soft-deletes by setting `revoked_at = now` only when the grant is owned by
    /// the caller and still active. Returns `Ok(true)` when a row changed,
    /// `Ok(false)` when nothing matched (unknown id, wrong owner, or already
    /// revoked).
    async fn revoke_client_grant(
        &self,
        id: &str,
        user_id: &str,
        tenant_id: &str,
    ) -> AppResult<bool>;

    /// Store a new pending device authorization (RFC 8628).
    async fn create_device_authorization(&self, da: &DeviceAuthorization) -> AppResult<()>;
    /// Look up a device authorization by the SHA-256 hash of its `device_code`.
    async fn get_device_authorization_by_code_hash(
        &self,
        device_code_hash: &str,
    ) -> AppResult<Option<DeviceAuthorization>>;
    /// Look up a device authorization by its `user_code` (operator-entered code).
    async fn get_device_authorization_by_user_code(
        &self,
        user_code: &str,
    ) -> AppResult<Option<DeviceAuthorization>>;
    /// Mark a pending device authorization approved by a super-admin.
    ///
    /// Returns `true` when a still-pending row was updated; `false` when nothing
    /// matched (unknown `user_code`, or already approved/denied).
    async fn approve_device_authorization(
        &self,
        user_code: &str,
        approved_by: &str,
    ) -> AppResult<bool>;
    /// Mark a pending device authorization denied. Returns `true` if a pending
    /// row was updated.
    async fn deny_device_authorization(&self, user_code: &str) -> AppResult<bool>;
    /// Delete a device authorization by `device_code` hash (single-use consume).
    ///
    /// Returns `true` if a row was deleted — the token endpoint mints the admin
    /// token only when this returns `true`, so a duplicate poll can never mint
    /// twice.
    async fn delete_device_authorization(&self, device_code_hash: &str) -> AppResult<bool>;
}

/// OAuth client-side state management repository
#[async_trait]
pub trait OAuthClientStateRepository: Send + Sync {
    /// Store OAuth client-side state for CSRF protection and PKCE verifier storage
    async fn store_oauth_client_state(&self, state: &OAuthClientState) -> AppResult<()>;
    /// Consume OAuth client state atomically (verify and mark as used)
    async fn consume_oauth_client_state(
        &self,
        state_value: &str,
        provider: &str,
        now: DateTime<Utc>,
    ) -> AppResult<Option<OAuthClientState>>;
    /// Delete the client states that expired without ever being consumed, and
    /// report how many went per provider.
    ///
    /// An OAuth launch the athlete never completed leaves exactly one such row —
    /// `expires_at` in the past with `used` still false — so the per-provider
    /// counts are the abandoned-launch signal. A row that expired *after* being
    /// consumed is the ordinary residue of a flow that succeeded and is left
    /// alone, as is an unexpired row a callback may still redeem.
    ///
    /// Entries are `(provider, deleted_rows)` sorted by provider name, so the
    /// output is stable across calls and backends; an empty vec means nothing was
    /// reaped, never that a provider reaped zero.
    async fn reap_expired_oauth_client_states(
        &self,
        now: DateTime<Utc>,
    ) -> AppResult<Vec<(String, u64)>>;
}

/// Provider connection management repository
#[async_trait]
pub trait ProviderConnectionRepository: Send + Sync {
    /// Register a provider connection (upsert)
    async fn register_connection(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        connection_type: &ConnectionType,
        metadata: Option<&str>,
    ) -> AppResult<()>;
    /// Remove a provider connection
    async fn remove_connection(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) -> AppResult<()>;
    /// Remove a provider connection only when its type is
    /// [`ConnectionType::Delegated`]: ending a delegated link can never delete
    /// the member's own connection to the same provider. Returns whether a
    /// row went.
    async fn remove_delegated_connection(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) -> AppResult<bool>;
    /// Record the kind of account the connection signed in with, as the
    /// provider reported it. Returns whether a connection row was updated;
    /// `false` when the user has no such connection.
    async fn set_account_role(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        role: ProviderAccountRole,
    ) -> AppResult<bool>;
    /// Get all provider connections for a user
    async fn get_for_user(
        &self,
        user_id: Uuid,
        tenant_id: Option<TenantId>,
    ) -> AppResult<Vec<ProviderConnection>>;
    /// Check if a specific provider is connected for a user (cross-tenant)
    async fn is_connected(&self, user_id: Uuid, provider: &str) -> AppResult<bool>;
    /// Mark a provider connection as just-used, updating `last_used_at = now()`.
    ///
    /// Called from the read path (chat tool execution, REST activity fetches) so the
    /// resolver can pick the most-recently-active backend when a subsequent tool call
    /// omits the provider argument. Best-effort: a failure to write should not abort
    /// the user's request — log and continue. No-op when the (user, tenant, provider)
    /// row does not exist.
    async fn touch_last_used(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) -> AppResult<()>;
    /// Resolve the user's most-recently-used *usable* provider connection.
    ///
    /// Health first: a connection whose `status` requires re-auth is elected only when
    /// the user has no `active` one, so a dead connection never shadows a healthy
    /// sibling that can still answer. Then a connection whose `account_role` is
    /// [`ProviderAccountRole::Coach`] goes after the others: a coach account has no
    /// calendar of its own, so a coach who also connected another provider gets
    /// their own workouts from it. Among equally ranked rows, returns the freshest
    /// `last_used_at` (NULLs last), falling back to the freshest `connected_at` when no
    /// row has been touched yet. Tenant scope is honored when `tenant_id` is provided;
    /// otherwise the lookup is cross-tenant. Returns `None` when the user has no
    /// provider connections at all.
    ///
    /// LIMITATION(registre#133): election reads recency and health, never capability, so a
    /// healthy strain-and-recovery provider connected after a distance provider is elected
    /// primary for activity queries it cannot answer well. Deciding whether "primary" is
    /// per-athlete or per-tool is the open question there.
    async fn resolve_most_recent(
        &self,
        user_id: Uuid,
        tenant_id: Option<TenantId>,
    ) -> AppResult<Option<ProviderConnection>>;
    /// Mark a connection as needing re-authentication after an attempt that
    /// began at `attempt_started_at` found its credential or session dead.
    ///
    /// Transitions `status` to `needs_reauth` and records the token-free error class in
    /// `last_error`. Guarded so the transition timestamp reflects the first failure, not
    /// every retry, and so a connection reconnected or re-armed after the attempt began
    /// is left as it is: the attempt's verdict is on the credential it read, not on the
    /// one that replaced it. `error_code` is a short OAuth error class (e.g.
    /// `invalid_request`) — NEVER token material. No-op when the row does not exist.
    ///
    /// Returns what the connection is now, so a caller that follows the flag with a
    /// reconnect prompt sends it only to a connection that needs one:
    /// [`ReauthMark::ReconnectedSince`] is a connection the guard left active.
    async fn mark_needs_reauth(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        error_code: Option<&str>,
        attempt_started_at: DateTime<Utc>,
    ) -> AppResult<ReauthMark>;
    /// Flip the connection `token` belongs to to `needs_reauth` for a refresh
    /// of `token` the provider refused, only while `token` is still its stored
    /// row exactly as the refresh read it: the same `id` and `updated_at`.
    ///
    /// A reconnect stores its token under a fresh `id` and re-arms the
    /// connection, and a refresh that landed meanwhile rewrote the row under the
    /// same `id`, so a refusal of the grant a reconnect replaced, or of a
    /// refresh token a concurrent refresh already spent, leaves the connection
    /// as it is. Returns whether the connection flipped: `false` when it was
    /// already `needs_reauth`, has no row, or its token changed since the read.
    async fn mark_needs_reauth_if_token_current(
        &self,
        token: &UserOAuthToken,
        error_code: &str,
    ) -> AppResult<bool>;
    /// Re-arm a connection after a successful (re)connect or token refresh.
    ///
    /// Transitions `status` back to `active` and clears the disconnect notification
    /// marker so a future disconnect notifies again. No-op when already active or when
    /// the row does not exist.
    async fn mark_active(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) -> AppResult<()>;
    /// Atomically claim the one-time disconnect notification for a `needs_reauth`
    /// connection.
    ///
    /// Sets `notified_at` only when the connection is `needs_reauth` and not yet notified,
    /// returning whether this call won the claim. Drives a single out-of-band reconnect
    /// nudge per disconnect; the marker is cleared on reconnect via [`Self::mark_active`].
    async fn claim_reauth_notification(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) -> AppResult<bool>;
}
