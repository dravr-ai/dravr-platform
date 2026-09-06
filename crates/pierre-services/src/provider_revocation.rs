// ABOUTME: Upstream grant revocation + provider-data purge for the disconnect chokepoint
// ABOUTME: Best-effort by contract — every bail logs and local deletion always proceeds

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! What "disconnect" owes the provider, split out of `oauth_flow`.
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
use pierre_core::http_client::{api_client, SharedHttpError};
use pierre_core::models::{TenantId, UserOAuthToken};
use pierre_providers::utils::refresh_oauth_token;
use pierre_runtime_context::DataContext;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::oauth_flow::OAuthService;

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
/// LIMITATION(registre#50): `revocation_shape` returns `None` for `coros`
/// because COROS publishes no API documentation — its OAuth endpoints in the
/// registry are placeholders, so its revocation surface cannot be confirmed
/// and a COROS disconnect stays local-delete-only.
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
/// Reads the stored token, resolves the client credentials through the
/// service (the same user→tenant→env chain that minted the grant, pinned to
/// the pool app the token names so a pool-app token revokes under its own
/// client) and spends the token against the provider.
pub async fn revoke_for_disconnect(
    service: &OAuthService,
    user_id: Uuid,
    tenant_id: TenantId,
    backend: &str,
) {
    let Some(shape) = revocation_shape(service, backend) else {
        debug!(
            user_id = %user_id,
            backend = %backend,
            "Backend holds no upstream grant to revoke; local deletion is the whole disconnect"
        );
        return;
    };
    let Some(token) = stored_token(&service.data, user_id, tenant_id, backend).await else {
        return;
    };
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
            return;
        }
    };
    revoke_upstream_grant(&shape, &creds, token, user_id, tenant_id, backend).await;
}

/// Revoke the user's grant at the provider, best-effort.
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
) {
    let result = match shape {
        RevocationShape::TokenRevocation {
            revoke_url,
            token_type_hint,
        } => {
            let Some((revoke_token, hint)) = revocation_material(token, user_id, backend) else {
                return;
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
                return;
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
                return;
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
    log_revocation_outcome(result, user_id, tenant_id, backend);
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

/// The stored token row a revocation spends, or `None` — with its reason
/// logged — when there is none, which the best-effort caller treats as
/// "skip, local deletion proceeds".
async fn stored_token(
    data: &DataContext,
    user_id: Uuid,
    tenant_id: TenantId,
    backend: &str,
) -> Option<UserOAuthToken> {
    match data
        .repos()
        .oauth_tokens
        .get_token(user_id, tenant_id, backend)
        .await
    {
        Ok(Some(token)) => Some(token),
        Ok(None) => {
            debug!(
                user_id = %user_id,
                backend = %backend,
                "No stored token at disconnect; nothing to revoke upstream"
            );
            None
        }
        Err(e) => {
            warn!(
                user_id = %user_id,
                backend = %backend,
                error = %e,
                "Could not read stored token for upstream revocation; local deletion proceeds"
            );
            None
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
        token_url,
        &creds.client_id,
        &creds.client_secret,
        &refresh,
        backend,
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

/// Report the revocation attempt: success at INFO, every failure shape at
/// WARN — never an error, because the caller's contract is best-effort and
/// local deletion proceeds regardless.
fn log_revocation_outcome(
    result: Result<reqwest::Response, SharedHttpError>,
    user_id: Uuid,
    tenant_id: TenantId,
    backend: &str,
) {
    match result {
        Ok(response) if response.status().is_success() => info!(
            user_id = %user_id,
            tenant_id = %tenant_id,
            backend = %backend,
            "Revoked the user's grant at the provider"
        ),
        Ok(response) => warn!(
            user_id = %user_id,
            backend = %backend,
            status = %response.status(),
            "Provider answered revocation with non-success; local deletion proceeds"
        ),
        Err(e) => warn!(
            user_id = %user_id,
            backend = %backend,
            error = %e,
            "Provider unreachable for revocation; local deletion proceeds"
        ),
    }
}
