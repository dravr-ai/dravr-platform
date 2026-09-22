// ABOUTME: Upstream grant revocation + provider-data purge for the disconnect chokepoint and an app switch
// ABOUTME: Never blocks local deletion, and reports whether the provider confirmed the revocation

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! What "disconnect" owes the provider, split out of `oauth_flow`.
//!
//! It also owes the provider the same when a reconnect moves an athlete from
//! one Strava shared-pool app to another: `strava_reconnect` revokes each
//! grant the new token supersedes through [`revoke_stored_grant`], under the
//! app that issued it, so the athlete is not counted against two applications.
//!
//! The chokepoint stays a thin orchestrator; this module revokes the grant
//! upstream (Strava API Policy §2.1 makes consent withdrawal an obligation,
//! and every other OAuth provider's athlete expects the same "this app no
//! longer has my data") and deletes the provider-derived cached rows (§7.4's
//! deletion clock — immediate is stronger than the 30-day ceiling and
//! simpler than a sweeper).

use chrono::{Duration, Utc};
use pierre_auth::oauth2_client::OAuth2Config;
use pierre_core::constants::oauth_providers;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::http_client::{api_client, SharedHttpError};
use pierre_core::models::{TenantId, UserOAuthToken};
use pierre_providers::utils::{refresh_oauth_token, ClientAuth, RefreshRequest};
use pierre_runtime_context::DataContext;
use serde::Serialize;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::oauth_flow::OAuthService;

/// Who asked for a provider to be disconnected, as the `provider.disconnected`
/// notify event reports it.
///
/// The event used to read the same whoever acted, so an athlete leaving, an
/// operator cleaning up and the seat-reclaim sweeper freeing a seat were one
/// indistinguishable count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisconnectReason {
    /// The athlete disconnected, from the app, the chat tool loop or `/mcp`.
    Athlete,
    /// An operator disconnected them, or removed their account.
    Operator,
    /// The seat-reclaim sweeper freed the Strava seat of an idle athlete.
    SeatReclaim,
}

impl DisconnectReason {
    /// The `reason` field value the notify event carries.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Athlete => "athlete",
            Self::Operator => "operator",
            Self::SeatReclaim => "seat_reclaim",
        }
    }
}

/// What happened to a backend's grant at the provider when it was withdrawn.
///
/// Local deletion never waits on this, so it is reported rather than enforced:
/// an operator who asked for a seat to be freed needs to know whether the
/// provider confirmed it, and "the rows are gone" is not that.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", content = "reason", rename_all = "snake_case")]
pub enum RevocationOutcome {
    /// The provider answered the revocation with success.
    Revoked,
    /// There was no upstream grant to withdraw: the backend holds none (an
    /// API key, a scrape session) or no token was stored for it.
    NoGrant,
    /// A grant may still be authorized at the provider: the revocation was
    /// refused, never reached it, or could not be built. The reason names
    /// which, and never carries token material.
    Unconfirmed(String),
}

impl RevocationOutcome {
    /// The outcome of withdrawing several backends of one provider together
    /// (a coalesced pair): unconfirmed if any is, else revoked if any grant
    /// was, else nothing to withdraw.
    #[must_use]
    pub fn combine(outcomes: Vec<Self>) -> Self {
        let mut combined = Self::NoGrant;
        for outcome in outcomes {
            let replaces = match (&combined, &outcome) {
                (Self::Unconfirmed(_), _) | (_, Self::NoGrant) => false,
                (_, Self::Unconfirmed(_) | Self::Revoked) => true,
            };
            if replaces {
                combined = outcome;
            }
        }
        combined
    }
}

/// The wire shape a backend expects "this app no longer has my data" in.
///
/// Endpoints come from where each provider's other endpoints already live:
/// `pierre-config` for Strava, Fitbit and Garmin (`*_REVOKE_URL` env), the
/// provider registry's default config for WHOOP and Terra
/// (`PIERRE_<PROVIDER>_REVOKE_URL` env).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RevocationShape {
    /// RFC 7009 token revocation (Strava, Fitbit): `POST` with the client
    /// credentials as HTTP Basic auth and the stored refresh token (else the
    /// access token) as the `token` form param — revoking either kills the
    /// whole grant. Strava reads `token_type_hint`; Fitbit documents `token`
    /// alone, so the hint is per-provider.
    TokenRevocation {
        /// The provider's revocation endpoint.
        revoke_url: String,
        /// Whether to send `token_type_hint` alongside `token`.
        token_type_hint: bool,
    },
    /// Per-user deregistration (WHOOP `DELETE /v2/user/access`, Garmin
    /// Health API `DELETE /user/registration`): the user's access token as
    /// `Bearer`. Only a live access token is accepted, so an expired one is
    /// refreshed at `token_url` first; the refreshed token is spent and never
    /// persisted, because the row is deleted right after.
    BearerDeregistration {
        /// The provider's deregistration endpoint.
        revoke_url: String,
        /// The provider's token endpoint, for the refresh an expired access
        /// token needs before it can deregister.
        token_url: String,
    },
    /// Terra `DELETE /auth/deauthenticateUser?user_id=…`: authenticated with
    /// the developer credentials as `dev-id` + `x-api-key` headers. The
    /// stored access token *is* the Terra user id (the contract
    /// `TerraProvider::set_credentials` reads it by).
    DeveloperKeyDeregistration {
        /// The provider's deauthentication endpoint.
        revoke_url: String,
    },
}

/// The revocation a backend takes, or `None` for a backend that holds no
/// upstream grant this service can withdraw.
///
/// `intervals_icu` links by a per-athlete API key the athlete pasted — there
/// is no OAuth grant, so deleting the local row is the whole disconnect.
/// `sciotte` and `sciotte_garmin` are scrape sessions: the credential is a
/// browser cookie jar, and there is nothing upstream to revoke either.
///
/// LIMITATION(registre#509): `revocation_shape` returns `None` for `coros`
/// because the COROS provider's OAuth endpoints are placeholders and no COROS
/// credentials are issued, so `POST /oauth2/deauthorize` is not called and a
/// COROS disconnect stays local-delete-only.
#[must_use]
pub fn revocation_shape(service: &OAuthService, backend: &str) -> Option<RevocationShape> {
    let config = service.config();
    match backend {
        oauth_providers::STRAVA => Some(RevocationShape::TokenRevocation {
            revoke_url: config.strava_api_config().revoke_url.clone(),
            token_type_hint: true,
        }),
        oauth_providers::FITBIT => Some(RevocationShape::TokenRevocation {
            revoke_url: config.fitbit_api_config().revoke_url.clone(),
            token_type_hint: false,
        }),
        oauth_providers::GARMIN => {
            let garmin = config.garmin_api_config();
            Some(RevocationShape::BearerDeregistration {
                revoke_url: garmin.revoke_url.clone(),
                token_url: garmin.token_url.clone(),
            })
        }
        oauth_providers::WHOOP => {
            let (revoke_url, token_url) = registry_endpoints(&service.data, backend)?;
            Some(RevocationShape::BearerDeregistration {
                revoke_url,
                token_url,
            })
        }
        oauth_providers::TERRA => {
            let (revoke_url, _) = registry_endpoints(&service.data, backend)?;
            Some(RevocationShape::DeveloperKeyDeregistration { revoke_url })
        }
        _ => None,
    }
}

/// The registry's `(revoke_url, token_url)` for a backend, or `None` — with
/// the reason logged — when the build did not register it or it declares no
/// revocation endpoint.
fn registry_endpoints(data: &DataContext, backend: &str) -> Option<(String, String)> {
    let Some(config) = data.provider_registry().default_config(backend) else {
        debug!(
            backend = %backend,
            "Backend is not registered in this build; nothing to revoke upstream"
        );
        return None;
    };
    let Some(revoke_url) = config.revoke_url.clone() else {
        warn!(
            backend = %backend,
            "Backend registered without a revocation endpoint; local deletion proceeds"
        );
        return None;
    };
    Some((revoke_url, config.token_url.clone()))
}

/// The disconnect chokepoint's revocation step.
///
/// Reads the stored token, then revokes it as [`revoke_stored_grant`] does.
pub async fn revoke_for_disconnect(
    service: &OAuthService,
    user_id: Uuid,
    tenant_id: TenantId,
    backend: &str,
) -> RevocationOutcome {
    let Some(shape) = revocation_shape(service, backend) else {
        debug!(
            user_id = %user_id,
            backend = %backend,
            "Backend holds no upstream grant to revoke; local deletion is the whole disconnect"
        );
        return RevocationOutcome::NoGrant;
    };
    let token = match stored_token(&service.data, user_id, tenant_id, backend).await {
        Ok(Some(token)) => token,
        Ok(None) => return RevocationOutcome::NoGrant,
        Err(outcome) => return outcome,
    };
    revoke_with_shape(service, &shape, token, user_id, tenant_id, backend).await
}

/// Revoke the grant a stored token row holds, under the client that issued it.
///
/// Resolves the client credentials through the service: the pool app the
/// token names when it names one, so a pool-app token revokes under its own
/// client whether or not that app still takes new athletes and whatever user
/// or tenant credentials were configured since; else the user→tenant→env
/// chain that minted the grant. Then spends the token against the provider.
pub async fn revoke_stored_grant(
    service: &OAuthService,
    token: UserOAuthToken,
    tenant_id: TenantId,
) -> RevocationOutcome {
    let backend = token.provider.clone();
    let user_id = token.user_id;
    let Some(shape) = revocation_shape(service, &backend) else {
        return RevocationOutcome::NoGrant;
    };
    revoke_with_shape(service, &shape, token, user_id, tenant_id, &backend).await
}

/// Resolve the issuing client's credentials and send the revocation.
async fn revoke_with_shape(
    service: &OAuthService,
    shape: &RevocationShape,
    token: UserOAuthToken,
    user_id: Uuid,
    tenant_id: TenantId,
    backend: &str,
) -> RevocationOutcome {
    let creds = match service
        .create_oauth_config_with_user(
            backend,
            user_id,
            Some(tenant_id.as_uuid()),
            token.oauth_app_client_id.as_deref(),
        )
        .await
    {
        Ok(creds) => creds,
        Err(e) => {
            warn!(
                user_id = %user_id,
                backend = %backend,
                error = %e,
                "No OAuth client credentials for upstream revocation; local deletion proceeds"
            );
            return RevocationOutcome::Unconfirmed(
                "no OAuth client credentials to revoke with".to_owned(),
            );
        }
    };
    revoke_upstream_grant(shape, &creds, token, user_id, tenant_id, backend).await
}

/// Revoke the user's grant at the provider.
///
/// Builds the request the backend's [`RevocationShape`] calls for from the
/// client credentials and the stored token, sends it once, and reports the
/// outcome. The token value itself is never logged.
pub async fn revoke_upstream_grant(
    shape: &RevocationShape,
    creds: &OAuth2Config,
    token: UserOAuthToken,
    user_id: Uuid,
    tenant_id: TenantId,
    backend: &str,
) -> RevocationOutcome {
    let result = match shape {
        RevocationShape::TokenRevocation {
            revoke_url,
            token_type_hint,
        } => {
            let Some((revoke_token, hint)) = revocation_material(token, user_id, backend) else {
                return RevocationOutcome::Unconfirmed(
                    "the stored token row carries no token material".to_owned(),
                );
            };
            let mut form = vec![("token", revoke_token.as_str())];
            if *token_type_hint {
                form.push(("token_type_hint", hint));
            }
            api_client()
                .post(revoke_url)
                .basic_auth(&creds.client_id, Some(&creds.client_secret))
                .form(&form)
                .send()
                .await
        }
        RevocationShape::BearerDeregistration {
            revoke_url,
            token_url,
        } => {
            let Some(access_token) =
                live_access_token(token, token_url, creds, user_id, backend).await
            else {
                return RevocationOutcome::Unconfirmed(
                    "no live access token to deregister with".to_owned(),
                );
            };
            api_client()
                .delete(revoke_url)
                .bearer_auth(access_token)
                .send()
                .await
        }
        RevocationShape::DeveloperKeyDeregistration { revoke_url } => {
            if token.access_token.is_empty() {
                warn!(
                    user_id = %user_id,
                    backend = %backend,
                    "Stored token row carries no provider user id; nothing to deauthenticate upstream"
                );
                return RevocationOutcome::Unconfirmed(
                    "the stored token row carries no provider user id".to_owned(),
                );
            }
            api_client()
                .delete(revoke_url)
                .header("dev-id", &creds.client_id)
                .header("x-api-key", &creds.client_secret)
                .query(&[("user_id", token.access_token.as_str())])
                .send()
                .await
        }
    };
    revocation_outcome(result, user_id, tenant_id, backend)
}

/// Delete the provider-derived cached activity rows on disconnect.
///
/// Best-effort: by the time this runs the disconnect itself has already
/// succeeded, and an undeleted row still ages out via retention pruning.
pub async fn purge_provider_cache(
    data: &DataContext,
    user_id: Uuid,
    tenant_id: TenantId,
    backend: &str,
) {
    match data
        .repos()
        .activity_cache
        .delete_provider_activities(user_id, &tenant_id, backend)
        .await
    {
        Ok(removed) => info!(
            user_id = %user_id,
            tenant_id = %tenant_id,
            backend = %backend,
            removed,
            "Deleted provider-derived cached activities on disconnect"
        ),
        Err(e) => warn!(
            user_id = %user_id,
            backend = %backend,
            error = %e,
            "Failed to delete cached activities on disconnect; rows age out via retention pruning"
        ),
    }
}

/// The stored token row a revocation spends: `Ok(None)` — logged — when
/// there is none, and the unconfirmed outcome when the row exists but cannot
/// be read, since a grant may stand behind it.
async fn stored_token(
    data: &DataContext,
    user_id: Uuid,
    tenant_id: TenantId,
    backend: &str,
) -> Result<Option<UserOAuthToken>, RevocationOutcome> {
    match data
        .repos()
        .oauth_tokens
        .get_token(user_id, tenant_id, backend)
        .await
    {
        Ok(Some(token)) => Ok(Some(token)),
        Ok(None) => {
            debug!(
                user_id = %user_id,
                backend = %backend,
                "No stored token at disconnect; nothing to revoke upstream"
            );
            Ok(None)
        }
        Err(e) => {
            warn!(
                user_id = %user_id,
                backend = %backend,
                error = %e,
                "Could not read stored token for upstream revocation; local deletion proceeds"
            );
            Err(RevocationOutcome::Unconfirmed(
                "the stored token could not be read".to_owned(),
            ))
        }
    }
}

/// The token material an RFC 7009 revocation spends: the stored refresh
/// token when present (revoking it kills the whole grant), else the access
/// token, paired with its `token_type_hint`. `None` — logged — when the row
/// carries nothing usable.
fn revocation_material(
    token: UserOAuthToken,
    user_id: Uuid,
    backend: &str,
) -> Option<(String, &'static str)> {
    let (revoke_token, hint) = match token.refresh_token.filter(|t| !t.is_empty()) {
        Some(refresh) => (refresh, "refresh_token"),
        None => (token.access_token, "access_token"),
    };
    if revoke_token.is_empty() {
        warn!(
            user_id = %user_id,
            backend = %backend,
            "Stored token row carries no token material; nothing to revoke upstream"
        );
        return None;
    }
    Some((revoke_token, hint))
}

/// The access token a bearer-style deregistration spends: the stored one
/// while it is still live, else one freshly minted from the stored refresh
/// token at `token_url`. A token within a minute of expiry counts as expired
/// so the DELETE does not race the clock. `None` — logged — when neither
/// route yields a usable token.
async fn live_access_token(
    token: UserOAuthToken,
    token_url: &str,
    creds: &OAuth2Config,
    user_id: Uuid,
    backend: &str,
) -> Option<String> {
    let expired = token
        .expires_at
        .is_some_and(|at| at <= Utc::now() + Duration::minutes(1));
    if !expired && !token.access_token.is_empty() {
        return Some(token.access_token);
    }
    let Some(refresh) = token.refresh_token.filter(|t| !t.is_empty()) else {
        warn!(
            user_id = %user_id,
            backend = %backend,
            "Stored access token is expired and no refresh token is stored; nothing usable to deregister with"
        );
        return None;
    };
    match refresh_oauth_token(
        api_client(),
        &RefreshRequest {
            token_url,
            client_id: &creds.client_id,
            client_secret: &creds.client_secret,
            refresh_token: &refresh,
            provider_name: backend,
            client_auth: ClientAuth::FormFields,
            extra_form: &[],
        },
    )
    .await
    {
        Ok(fresh) => {
            let fresh = fresh.access_token.filter(|t| !t.is_empty());
            if fresh.is_none() {
                warn!(
                    user_id = %user_id,
                    backend = %backend,
                    "Token refresh answered without an access token; nothing usable to deregister with"
                );
            }
            fresh
        }
        Err(e) => {
            warn!(
                user_id = %user_id,
                backend = %backend,
                error = %e,
                "Token refresh before deregistration failed; local deletion proceeds"
            );
            None
        }
    }
}

/// Classify and log the revocation attempt: success at INFO, every failure
/// shape at WARN — never an error, because local deletion proceeds
/// regardless; the caller reports the outcome instead. The reason names the
/// HTTP status or that the provider was unreachable, never the transport
/// error's text, which can carry the request URL.
fn revocation_outcome(
    result: Result<reqwest::Response, SharedHttpError>,
    user_id: Uuid,
    tenant_id: TenantId,
    backend: &str,
) -> RevocationOutcome {
    match result {
        Ok(response) if response.status().is_success() => {
            info!(
                user_id = %user_id,
                tenant_id = %tenant_id,
                backend = %backend,
                "Revoked the user's grant at the provider"
            );
            RevocationOutcome::Revoked
        }
        Ok(response) => {
            warn!(
                user_id = %user_id,
                backend = %backend,
                status = %response.status(),
                "Provider answered revocation with non-success; local deletion proceeds"
            );
            RevocationOutcome::Unconfirmed(format!(
                "the provider answered HTTP {}",
                response.status()
            ))
        }
        Err(e) => {
            warn!(
                user_id = %user_id,
                backend = %backend,
                error = %e,
                "Provider unreachable for revocation; local deletion proceeds"
            );
            RevocationOutcome::Unconfirmed("the provider could not be reached".to_owned())
        }
    }
}

/// Rows that survived a disconnect, as `"backend:token"` / `"backend:connection"`.
///
/// The disconnect chokepoint calls this after deleting so it can prove the
/// state actually changed before reporting success. A disconnect that returns
/// Ok while a grant survives is invisible to every signal we keep — the client
/// gets its 204, `provider.disconnected` fires, nothing is logged — so a
/// one-directional resolve that cleared only half a coalesced pair looked
/// exactly like a working disconnect from the outside.
///
/// An empty result is the normal case, including for a repeat click: the
/// second disconnect finds nothing left to survive.
pub async fn surviving_rows(
    data: &DataContext,
    user_id: Uuid,
    tenant_id: TenantId,
    backends: &[String],
) -> Vec<String> {
    let mut survivors = Vec::new();

    for backend in backends {
        if matches!(
            data.repos()
                .oauth_tokens
                .get_token(user_id, tenant_id, backend)
                .await,
            Ok(Some(_))
        ) {
            survivors.push(format!("{backend}:token"));
        }
    }

    if let Ok(connections) = data
        .repos()
        .provider_connections
        .get_for_user(user_id, Some(tenant_id))
        .await
    {
        for backend in backends {
            if connections.iter().any(|c| &c.provider == backend) {
                survivors.push(format!("{backend}:connection"));
            }
        }
    }

    survivors
}

/// Withdraw one backend completely: revoke upstream, then delete the token and
/// connection rows in lockstep and purge the provider-derived cache.
///
/// The two row types are separate sources of truth (`oauth_tokens` drives
/// `resolve_backend` + the scrape session; `provider_connections` drives the
/// "connected" badge and coaching fetch enumeration), so an orphaned connection
/// row shows "Connected" for a dead session and routes the next fetch to a
/// backend with no token.
///
/// Clearing a backend the user never held is a no-op, which is what makes
/// clearing a whole coalesced pair safe.
///
/// Returns what the provider said about the grant. Upstream revocation never
/// blocks local deletion; the outcome is how a caller learns the grant may
/// still stand.
///
/// # Errors
/// Returns a database error if either delete fails.
pub async fn clear_backend(
    service: &OAuthService,
    data: &DataContext,
    user_id: Uuid,
    tenant_id: TenantId,
    backend: &str,
) -> AppResult<RevocationOutcome> {
    // Revoke BEFORE deleting — the stored token is the credential the
    // revocation call spends.
    let outcome = revoke_for_disconnect(service, user_id, tenant_id, backend).await;

    data.repos()
        .oauth_tokens
        .delete_token(user_id, tenant_id, backend)
        .await
        .map_err(|e| AppError::database(format!("Failed to delete OAuth token: {e}")))?;

    data.repos()
        .provider_connections
        .remove_connection(user_id, tenant_id, backend)
        .await
        .map_err(|e| AppError::database(format!("Failed to remove provider connection: {e}")))?;

    purge_provider_cache(data, user_id, tenant_id, backend).await;
    Ok(outcome)
}
