// ABOUTME: The platform's refresh of a stored provider token: descriptor-driven, one flight per connection, written by compare-and-swap
// ABOUTME: Decides what a landed, raced, refused or failed refresh leaves the caller and the connection
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A refresh spends the stored refresh token, and a provider that rotates
//! refresh tokens (WHOOP, Nolio) honours each one once. Three things keep two
//! refreshes of one connection from costing the athlete their grant:
//!
//! 1. **One flight per connection, in this process.** Concurrent callers for
//!    the same tenant, user and provider share one refresh
//!    ([`SingleFlight`]); the others get its result and call nobody. The
//!    refresh runs as a task of its own, so a caller that is dropped while it
//!    is in flight (a client that disconnected, an outer timeout) cannot stop
//!    it between the provider's answer and the write: the provider has
//!    rotated by then, and a pair that is never stored is a grant lost.
//! 2. **A compare-and-swap write.** The refreshed pair lands only over the row
//!    exactly as the refresh read it. A refresh that lost that swap, to a
//!    reconnect or to another server instance's refresh, re-reads the row and
//!    returns the winner's token.
//! 3. **A bounded wait before believing a refusal.** A refresher outside
//!    this process's flight (another server instance, a provider client
//!    refreshing inside an API call) shares nothing with it, so the loser of
//!    that race is refused for presenting the token the winner just spent,
//!    possibly before the winner's write has landed. A refusal is therefore a
//!    verdict only on a row that is still unchanged after [`WINNER_WINDOW`].
//!
//! What the refresh posts comes from the provider's descriptor
//! ([`ProviderDescriptor::oauth_refresh`]), so a provider is refreshed by
//! declaring its grant there, with nothing to add here.
//!
//! [`ProviderDescriptor::oauth_refresh`]: pierre_providers::spi::ProviderDescriptor::oauth_refresh

use super::single_flight::{Lost, SingleFlight};
use super::{AuthService, OAuthError, TokenData};
use crate::protocol::reauth_notice::notify_needs_reauth;
use crate::protocol::refresh_failure::classify_refresh_failure;
use chrono::Utc;
use pierre_core::http_client::api_client;
use pierre_core::models::{connection_needs_reauth, TenantId, UserOAuthToken};
use pierre_providers::owner_id::owner_id_for_access_token;
use pierre_providers::spi::OAuthRefresh;
use pierre_providers::utils::{refresh_oauth_token, RefreshRequest};
use pierre_providers::CredentialKind;
use std::sync::{Arc, LazyLock};
use std::time::Duration;
use tokio::time::sleep;
use tracing::{info, warn};
use uuid::Uuid;

/// The connection a refresh is for: tenant, user and provider slug.
type Connection = (TenantId, Uuid, String);

/// What a refresh leaves its caller, and every caller that shared its flight.
type RefreshOutcome = Result<Option<TokenData>, OAuthError>;

/// The refreshes in flight in this process. It is process-wide because
/// [`AuthService`] is built per call: a map on the service would be a map per
/// caller, and would share nothing.
static REFRESHES_IN_FLIGHT: LazyLock<SingleFlight<Connection, RefreshOutcome>> =
    LazyLock::new(SingleFlight::new);

/// The pauses between re-reads of a row whose refresh the provider refused,
/// after one immediate re-read: the window in which a refresh that spent the
/// refresh token first may still land its write.
///
/// **Who the winner can be.** Never another caller of this flow in this
/// process: they share the flight, and the flight runs to its end even when
/// every caller is dropped, so its pair is stored before the key is free for
/// the next refresh. What is left is a refresher outside the flight: another
/// server instance's flight, and a provider client refreshing on its own
/// inside an API call (`refresh_token_if_needed`), whose pair is written back
/// through the same compare-and-swap. A reconnect is a winner too, but stores
/// a grant of its own and has nothing in flight to wait for.
///
/// **The bound.** The provider answers one request per refresh token, so by
/// the time this refresh is refused the winner's request has already been
/// answered. What the winner still has to do is receive that answer and run
/// one indexed `UPDATE`: milliseconds, normally. The window therefore covers
/// a winner whose answer-to-write takes up to 1.75 s longer than this
/// refresh took to hear its refusal and re-read the row. The hard limits on
/// those two steps (the API client's 30 s request timeout, the pool's 20 to
/// 30 s acquire timeout) are how long a failure takes, not how long a write
/// takes, and no caller can be held for them; 1.75 s in three widening steps
/// is wide enough for a write delayed by a slow pool acquire or a loaded
/// instance, and about 6% of the 30 s the refresh call itself may already
/// have taken.
///
/// **Past the bound.** A winner slower than the window is flagged
/// `needs_reauth` by its loser, and the athlete is sent one reconnect notice
/// that the winner cannot take back. The grant is not lost: the flip is
/// guarded on the row, so it never lands over a pair already stored, and the
/// winner's pair still lands and is what the next lookup serves. The flag
/// itself is lifted by the next refresh of this flow that lands
/// ([`AuthService::mark_connection_active`]): at once when the winner is
/// another instance's flight, and at the next expiry when it is a provider
/// client, whose write-back stores the pair and no status. A winner that never
/// writes at all (its process died, or its caller was dropped inside a
/// provider client's own refresh, which is not spawned) is not a slow winner
/// but a lost pair, and no window recovers it: the refusal is then the truth.
const WINNER_WINDOW: [Duration; 3] = [
    Duration::from_millis(250),
    Duration::from_millis(500),
    Duration::from_millis(1000),
];

/// What a refresh of a stored token posts: the provider's declared grant, its
/// token endpoint and the stored refresh token.
struct RefreshPlan<'a> {
    /// The provider's registered slug.
    provider: &'a str,
    /// Client authentication and extra form fields, as the descriptor declares.
    grant: OAuthRefresh,
    token_url: &'a str,
    refresh_token: &'a str,
}

impl AuthService {
    /// Refresh the token row `read` of `provider`, or return the token that
    /// replaced it since it was read.
    ///
    /// Returns `None` when a refresh here cannot renew it (the provider
    /// declares no refresh grant, or no refresh token is stored) or the
    /// provider refused the refresh: reconnecting is then the remedy.
    ///
    /// Concurrent calls for the same tenant, user and provider share one
    /// refresh, and all of them get its outcome. The refresh is spawned, with
    /// a service, a row and names of its own, so it runs to its end whether or
    /// not this call is still there to hear of it.
    ///
    /// # Errors
    /// Returns `OAuthError` when the token cannot be read or stored, or the
    /// refresh failed without the provider refusing it. A refresh that ended
    /// without an outcome (it panicked) is [`OAuthError::RefreshUnavailable`]
    /// for everyone who waited on it: nothing says the grant is dead, and the
    /// next call reads the row as that refresh left it.
    pub(super) async fn refresh_stored_token(
        &self,
        user_id: Uuid,
        tenant_id: &str,
        provider: &str,
        read: &UserOAuthToken,
    ) -> Result<Option<TokenData>, OAuthError> {
        let slug = provider.to_ascii_lowercase();
        if self.declared_refresh(&slug).is_none() {
            return Ok(None);
        }
        let tenant = TenantId::parse_str(tenant_id).map_err(|_| {
            OAuthError::DatabaseError(format!("Invalid tenant_id format: {tenant_id}"))
        })?;

        let outcome = REFRESHES_IN_FLIGHT
            .run((tenant, user_id, slug.clone()), || {
                let service = Self::new(Arc::clone(&self.resources));
                let (provider, read) = (provider.to_owned(), read.clone());
                async move {
                    service
                        .refresh_in_flight(user_id, tenant, &provider, &slug, &read)
                        .await
                }
            })
            .await;
        outcome.unwrap_or_else(|Lost| {
            warn!(
                user_id = %user_id,
                provider = %provider,
                "The token refresh ended without an outcome; the connection is left as it is"
            );
            Err(OAuthError::RefreshUnavailable(provider.to_owned()))
        })
    }

    /// The refresh grant `slug`'s descriptor declares and the token endpoint
    /// it is posted to, or `None` when the provider is not registered or
    /// declares none.
    ///
    /// The endpoint is the registry's default configuration, which carries
    /// the `PIERRE_<PROVIDER>_TOKEN_URL` override, and the descriptor's own
    /// for a provider registered without one.
    fn declared_refresh(&self, slug: &str) -> Option<(OAuthRefresh, String)> {
        let registry = self.resources.provider_registry();
        let descriptor = registry.get_descriptor(slug)?;
        let grant = descriptor.oauth_refresh()?;
        let token_url = registry
            .default_config(slug)
            .map(|config| config.token_url.clone())
            .filter(|url| !url.is_empty())
            .or_else(|| {
                descriptor
                    .oauth_endpoints()
                    .map(|endpoints| endpoints.token_url.to_owned())
            })?;
        Some((grant, token_url))
    }

    /// The one refresh a flight runs.
    ///
    /// The row is read again first: a caller can arrive with a row it read
    /// before an earlier flight landed, whose refresh token that flight spent.
    /// A row replaced since `read` and unexpired is the answer, with no call
    /// to the provider; otherwise the row as it stands now is what is
    /// refreshed.
    async fn refresh_in_flight(
        &self,
        user_id: Uuid,
        tenant: TenantId,
        provider: &str,
        slug: &str,
        read: &UserOAuthToken,
    ) -> Result<Option<TokenData>, OAuthError> {
        let Some(current) = self.stored_token(user_id, tenant, provider).await? else {
            return Ok(None);
        };
        if Self::supersedes(&current, read) {
            info!(
                user_id = %user_id,
                provider = %provider,
                "The token was refreshed or replaced since it was read; the stored one stands"
            );
            return Ok(Some(Self::token_data(provider, current)));
        }
        self.refresh_row(user_id, tenant, provider, slug, &current)
            .await
    }

    /// Refresh the stored row `current` and settle what its outcome leaves:
    /// the refreshed token, the token of whoever won the swap, or what a
    /// failure leaves ([`Self::refresh_failed`]). `None` when nothing here can
    /// renew it.
    async fn refresh_row(
        &self,
        user_id: Uuid,
        tenant: TenantId,
        provider: &str,
        slug: &str,
        current: &UserOAuthToken,
    ) -> Result<Option<TokenData>, OAuthError> {
        let Some((grant, token_url)) = self.declared_refresh(slug) else {
            return Ok(None);
        };
        let Some(refresh_token) = current.refresh_token.as_deref().filter(|t| !t.is_empty()) else {
            return Ok(None);
        };
        let plan = RefreshPlan {
            provider: slug,
            grant,
            token_url: &token_url,
            refresh_token,
        };

        info!(
            "Refreshing the token for user {} provider {}",
            user_id, provider
        );

        // Attempt to refresh the token, under the app that issued it
        match self
            .refresh_provider_token(user_id, tenant, &plan, current)
            .await
        {
            // The swap was lost: the winner's token is the one to use.
            Ok(None) => {
                self.superseding_token(user_id, tenant, provider, current)
                    .await
            }
            Ok(Some(mut refreshed_token)) => {
                info!(
                    "Token refreshed successfully for user {} provider {}",
                    user_id, provider
                );
                // A refresh that landed proves the grant alive: re-arm a
                // connection that a refresh on another server instance, which
                // lost the race for this refresh token and outwaited
                // `WINNER_WINDOW`, flipped to needs_reauth. No-op when active.
                self.mark_connection_active(user_id, tenant, provider).await;
                refreshed_token.provider_user_id =
                    self.owner_id_after_refresh(current, &refreshed_token).await;
                Ok(Some(refreshed_token))
            }
            Err(e) => {
                self.refresh_failed(user_id, tenant, provider, current, e)
                    .await
            }
        }
    }

    /// The token stored for the connection now.
    async fn stored_token(
        &self,
        user_id: Uuid,
        tenant: TenantId,
        provider: &str,
    ) -> Result<Option<UserOAuthToken>, OAuthError> {
        self.resources
            .repos()
            .oauth_tokens
            .get_token(user_id, tenant, provider)
            .await
            .map_err(|e| Self::store_failed(user_id, provider, "re-read the token", &e))
    }

    /// Whether `current` is a usable token that replaced the row `read`: a
    /// reconnect's (a fresh `id`) or the pair another refresh of the same row
    /// wrote (a new `updated_at`), and unexpired.
    fn supersedes(current: &UserOAuthToken, read: &UserOAuthToken) -> bool {
        (current.id != read.id || current.updated_at != read.updated_at)
            && current
                .expires_at
                .is_none_or(|expires_at| !Self::is_token_expired(expires_at))
    }

    /// The token that replaced the row `read` a refresh started from while the
    /// refresh was in flight ([`Self::supersedes`]); whatever this refresh
    /// brought back is dropped. `None` while the row stands as it was read.
    async fn superseding_token(
        &self,
        user_id: Uuid,
        tenant: TenantId,
        provider: &str,
        read: &UserOAuthToken,
    ) -> Result<Option<TokenData>, OAuthError> {
        let replacement = self
            .stored_token(user_id, tenant, provider)
            .await?
            .filter(|current| Self::supersedes(current, read));
        if replacement.is_some() {
            info!(
                user_id = %user_id,
                provider = %provider,
                "The token was replaced while its refresh was in flight; the replacement stands"
            );
        }
        Ok(replacement.map(|token| Self::token_data(provider, token)))
    }

    /// The provider-side owner id a refreshed token carries.
    ///
    /// The bearer providers' refresh endpoints return no owner id and the row
    /// update leaves the stored one untouched, so a refreshed token reports the
    /// id the row already holds. A row of a provider whose descriptor declares
    /// the id is read from its API (WHOOP, Garmin) that never captured one
    /// (the token response carries none, and connections made before the OAuth
    /// flow read it have `None`) is filled here: the id is read with the fresh
    /// access token and written back to the row, so the next push event naming
    /// this athlete routes to them. Best-effort — a failed read leaves the row
    /// as it was and the next refresh tries again.
    async fn owner_id_after_refresh(
        &self,
        stored: &UserOAuthToken,
        refreshed: &TokenData,
    ) -> Option<String> {
        if stored.provider_user_id.is_some() {
            return stored.provider_user_id.clone();
        }
        match owner_id_for_access_token(
            self.resources.provider_registry(),
            &stored.provider,
            &refreshed.access_token,
        )
        .await
        {
            Ok(None) => None,
            Ok(Some(owner_id)) => {
                self.persist_owner_id(stored, &owner_id).await;
                Some(owner_id)
            }
            Err(e) => {
                warn!(
                    user_id = %stored.user_id,
                    provider = %stored.provider,
                    error = %e,
                    "provider user id lookup failed after refresh; push events for this connection route only once a later refresh captures it"
                );
                None
            }
        }
    }

    /// Write a captured owner id onto the token row the refresh just updated.
    ///
    /// The id is the only column written. The profile read that produced it
    /// took a round trip to the provider, and another refresh of the row (the
    /// provider's own client on a refused call, another server instance) can
    /// land in that time: writing the pair this refresh holds again would put
    /// a spent refresh token back over the newer one. The write is guarded on
    /// the row's `id`, so a reconnect that replaced the row since, possibly
    /// with another account, stands. A write failure is logged: the in-memory
    /// token still carries the id for this request, and the next refresh
    /// captures it again.
    async fn persist_owner_id(&self, stored: &UserOAuthToken, owner_id: &str) {
        match self
            .resources
            .repos()
            .oauth_tokens
            .record_provider_user_id(stored, owner_id)
            .await
        {
            Ok(true) => info!(
                user_id = %stored.user_id,
                provider = %stored.provider,
                "captured provider user id on token refresh"
            ),
            Ok(false) => info!(
                user_id = %stored.user_id,
                provider = %stored.provider,
                "the token was replaced since its refresh; the captured provider user id is not written over the replacement"
            ),
            Err(e) => warn!(
                user_id = %stored.user_id,
                provider = %stored.provider,
                error = %e,
                "failed to persist the provider user id captured on refresh"
            ),
        }
    }

    /// What a failed refresh of the stored row `stored` leaves the caller.
    ///
    /// Only the provider's answer is judged: a database failure storing the
    /// pair is ours, and is returned as it is. A failure the provider did not
    /// word as a refusal of the grant or of the client (a rate limit, a 5xx, a
    /// transport failure) says nothing about the grant, so the connection is
    /// left as it is and the grant's standing is the connection's (see
    /// [`Self::unrefused_refresh_failed`]). The provider's answer is logged
    /// here and nowhere else, since the error's text reaches the model and the
    /// stored conversation.
    ///
    /// A refusal — a dead or rotated refresh token, a revoked grant, a
    /// rejected client — is first checked against the row
    /// ([`Self::winner_of_the_race`]): a token a reconnect or another refresh
    /// stored meanwhile is returned, and the refusal of what it replaced is no
    /// verdict on it. Only over a row still as it was read does the user have
    /// to reconnect: the connection flips to `needs_reauth`, guarded once more
    /// on `stored` being its token as read.
    async fn refresh_failed(
        &self,
        user_id: Uuid,
        tenant: TenantId,
        provider: &str,
        stored: &UserOAuthToken,
        error: OAuthError,
    ) -> Result<Option<TokenData>, OAuthError> {
        warn!(
            "Token refresh failed for user {} provider {}: {}",
            user_id, provider, error
        );
        let OAuthError::TokenRefreshFailed(answer) = &error else {
            return Err(error);
        };
        let Some(error_code) = classify_refresh_failure(answer) else {
            return self
                .unrefused_refresh_failed(user_id, tenant, provider)
                .await;
        };
        if let Some(winner) = self
            .winner_of_the_race(user_id, tenant, provider, stored)
            .await?
        {
            return Ok(Some(winner));
        }
        if self.flip_to_needs_reauth(stored, error_code).await {
            notify_needs_reauth(&self.resources, user_id, tenant, provider).await;
        }
        // The flip is guarded on the row: one that changed between the last
        // re-read and the flip left the connection alone, and is the answer.
        self.superseding_token(user_id, tenant, provider, stored)
            .await
    }

    /// The token that replaced `stored`, whose refresh the provider refused,
    /// looked for at once and then across [`WINNER_WINDOW`].
    ///
    /// A refresh token that another refresh spent first is refused exactly as
    /// a dead one is, and the row is what tells them apart: the winner writes
    /// its pair over it. The flight rules out another caller of this flow in
    /// this process, so the winner is a reconnect, another server instance, or
    /// a provider client refreshing on its own, and the last two may not have
    /// written yet; hence the wait.
    ///
    /// A connection an earlier refusal already flagged is not waited on: its
    /// grant was refused over a row that outlasted the window once, the
    /// athlete has been told, and every later call on it would pay the window
    /// again to learn the same thing.
    async fn winner_of_the_race(
        &self,
        user_id: Uuid,
        tenant: TenantId,
        provider: &str,
        stored: &UserOAuthToken,
    ) -> Result<Option<TokenData>, OAuthError> {
        if let Some(winner) = self
            .superseding_token(user_id, tenant, provider, stored)
            .await?
        {
            return Ok(Some(winner));
        }
        if self
            .connection_flagged(user_id, tenant, provider)
            .await
            .unwrap_or(false)
        {
            return Ok(None);
        }
        for pause in WINNER_WINDOW {
            sleep(pause).await;
            if let Some(winner) = self
                .superseding_token(user_id, tenant, provider, stored)
                .await?
            {
                return Ok(Some(winner));
            }
        }
        Ok(None)
    }

    /// Whether the connection is one an earlier refusal left `needs_reauth`
    /// (or that was revoked). A status that cannot be read confirms neither,
    /// and is the store's failure.
    async fn connection_flagged(
        &self,
        user_id: Uuid,
        tenant: TenantId,
        provider: &str,
    ) -> Result<bool, OAuthError> {
        let connections = self
            .resources
            .repos()
            .provider_connections
            .get_for_user(user_id, Some(tenant))
            .await
            .map_err(|e| Self::store_failed(user_id, provider, "read the connection status", &e))?;
        Ok(connection_needs_reauth(&connections, provider))
    }

    /// What a refresh the provider failed without refusing leaves the caller.
    ///
    /// The failure revives nothing: a connection an earlier refusal left
    /// `needs_reauth` (or that was revoked) still holds a grant only a
    /// reconnect restores, and the athlete was already told so, so the caller
    /// gets `None`, the reconnect every other surface shows. Any other
    /// connection's grant stands, and the caller gets
    /// [`OAuthError::RefreshUnavailable`]: read as "no token", it would send
    /// every caller that tags a missing token as auth-required (the capture
    /// sweep, the backfill) to flag a live connection and free its seat. A
    /// status that cannot be read confirms neither, and is the store's
    /// failure.
    async fn unrefused_refresh_failed(
        &self,
        user_id: Uuid,
        tenant: TenantId,
        provider: &str,
    ) -> Result<Option<TokenData>, OAuthError> {
        if self.connection_flagged(user_id, tenant, provider).await? {
            info!(
                user_id = %user_id,
                provider = %provider,
                "The refresh failed transiently over a connection an earlier refusal flagged; it still needs reconnecting"
            );
            return Ok(None);
        }
        Err(OAuthError::RefreshUnavailable(provider.to_owned()))
    }

    /// Write the `needs_reauth` flip for a refused refresh of the token row
    /// `stored`, guarded on that row as it was read, and report whether the
    /// connection flipped: the caller nudges the user once when it did.
    ///
    /// The connection row stays in the DB so the user/tenant mapping and
    /// history survive; only its `status` flips, and only while `stored` is
    /// still its token as the refresh read it: a reconnect that replaced it
    /// meanwhile re-armed the connection, and a concurrent refresh that landed
    /// proved the grant alive, so the refusal of what they replaced leaves
    /// either alone. Every outcome is logged; a failed write must not abort
    /// the user's request, and reads as not flipped, so it nudges nobody.
    async fn flip_to_needs_reauth(&self, stored: &UserOAuthToken, error_code: &str) -> bool {
        let (user_id, provider) = (stored.user_id, &stored.provider);
        match self
            .resources
            .repos()
            .provider_connections
            .mark_needs_reauth_if_token_current(stored, error_code)
            .await
        {
            Ok(true) => {
                info!(
                    "Provider {provider} flipped to needs_reauth for user {user_id} ({error_code})"
                );
                true
            }
            Ok(false) => {
                info!(
                    "Provider {provider} left as it was for user {user_id} ({error_code}): already needs_reauth, or its token changed since the refresh read it"
                );
                false
            }
            Err(e) => {
                warn!("Failed to persist needs_reauth for user {user_id} provider {provider}: {e}");
                false
            }
        }
    }

    /// Re-arm a provider connection to `active` after a refresh that landed
    /// (best-effort).
    ///
    /// No-op when the connection is already active or the row does not exist. A write
    /// failure must not abort the user's request — log and continue.
    async fn mark_connection_active(&self, user_id: Uuid, tenant: TenantId, provider: &str) {
        if let Err(e) = self
            .resources
            .repos()
            .provider_connections
            .mark_active(user_id, tenant, provider)
            .await
        {
            warn!("Failed to re-arm connection for user {user_id} provider {provider}: {e}");
        }
    }

    /// Post the refresh `plan` describes and store the new pair over `stored`,
    /// the row it was read from.
    ///
    /// A refresh token only refreshes under the client that issued it. A Strava
    /// token a shared-pool app issued names that app, and is refreshed under
    /// its credentials whatever the user or tenant has configured since; any
    /// other token resolves user-specific → tenant-level → env var defaults,
    /// the chain that issued it.
    ///
    /// Returns `None` when the refreshed pair was not stored because `stored`
    /// is no longer the row in place as it was read: a reconnect replaced it
    /// while the refresh was in flight, or another refresh of it landed first,
    /// and that token stands.
    ///
    /// # Errors
    /// Returns `OAuthError` if token refresh or database operations fail
    async fn refresh_provider_token(
        &self,
        user_id: Uuid,
        tenant: TenantId,
        plan: &RefreshPlan<'_>,
        stored: &UserOAuthToken,
    ) -> Result<Option<TokenData>, OAuthError> {
        let provider = plan.provider;
        let issuing_app = stored.oauth_app_client_id.as_deref();
        let (client_id, client_secret) = self
            .issuing_client_credentials(user_id, Some(&tenant.to_string()), provider, issuing_app)
            .await
            .map_err(OAuthError::TokenRefreshFailed)?;

        // The same refresh the provider's own client sends, through the one
        // refresh shell: its vendor's client authentication and extra fields.
        let refreshed = refresh_oauth_token(
            api_client(),
            &RefreshRequest::described(
                provider,
                plan.token_url,
                &client_id,
                &client_secret,
                plan.refresh_token,
                plan.grant,
            ),
        )
        .await
        .map_err(|e| OAuthError::TokenRefreshFailed(e.to_string()))?;

        let new_access_token = refreshed.access_token.ok_or_else(|| {
            OAuthError::TokenRefreshFailed(format!(
                "the {provider} token endpoint answered without an access token"
            ))
        })?;
        // A provider that sends no refresh token back leaves the stored one
        // standing: RFC 6749 section 6 makes issuing a new one optional.
        let new_refresh_token = refreshed
            .refresh_token
            .unwrap_or_else(|| plan.refresh_token.to_owned());
        let new_expires_at = refreshed.expires_at;

        // Swap the pair in over the row this refresh read
        let landed = self
            .resources
            .repos()
            .oauth_tokens
            .refresh_token(
                stored,
                &new_access_token,
                Some(&new_refresh_token),
                new_expires_at,
            )
            .await
            .map_err(|e| Self::store_failed(user_id, provider, "store the refreshed token", &e))?;
        if !landed {
            return Ok(None);
        }

        // Return the refreshed token data. The refresh endpoints of the bearer
        // providers (strava/whoop/garmin) return no owner id; the caller fills it
        // from the stored row (`owner_id_after_refresh`). A refresh does not
        // re-issue scopes, so the stored set carries across.
        Ok(Some(TokenData {
            provider: provider.to_owned(),
            access_token: new_access_token,
            refresh_token: new_refresh_token,
            expires_at: new_expires_at.unwrap_or_else(Utc::now),
            scopes: stored.scope.clone().unwrap_or_default(),
            provider_user_id: None,
            oauth_app_client_id: stored.oauth_app_client_id.clone(),
            row_id: stored.id.clone(),
            kind: CredentialKind::OAuthBearer,
        }))
    }
}
