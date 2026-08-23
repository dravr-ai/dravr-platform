// ABOUTME: Upstream grant revocation + provider-data purge for the disconnect chokepoint
// ABOUTME: Best-effort by contract — every bail logs and local deletion always proceeds

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! What "disconnect" owes the provider, split out of `oauth_flow` so the
//! chokepoint stays a thin orchestrator: revoke the grant upstream (Strava
//! API Policy §2.1 makes consent withdrawal an obligation) and delete the
//! provider-derived cached rows (§7.4's deletion clock — immediate is
//! stronger than the 30-day ceiling and simpler than a sweeper).

use pierre_auth::oauth2_client::OAuth2Config;
use pierre_core::http_client::{api_client, SharedHttpError};
use pierre_core::models::TenantId;
use pierre_runtime_context::DataContext;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::oauth_flow::OAuthService;

/// The disconnect chokepoint's revocation step: resolve the client
/// credentials through the service (the same user→tenant→env chain that
/// minted the grant, so pool-app tokens revoke under their own client) and
/// spend the stored token against the provider's revoke endpoint.
pub async fn revoke_for_disconnect(
    service: &OAuthService,
    user_id: Uuid,
    tenant_id: TenantId,
    backend: &str,
) {
    if backend != "strava" {
        return;
    }
    let creds = match service
        .create_oauth_config_with_user(backend, user_id, Some(tenant_id.as_uuid()), None)
        .await
    {
        Ok(creds) => Some(creds),
        Err(e) => {
            warn!(
                user_id = %user_id,
                backend = %backend,
                error = %e,
                "No OAuth client credentials for upstream revocation; local deletion proceeds"
            );
            None
        }
    };
    let revoke_url = service.config().strava_api_config().revoke_url.clone();
    revoke_upstream_grant(
        &service.data,
        &revoke_url,
        creds,
        user_id,
        tenant_id,
        backend,
    )
    .await;
}

/// Revoke the user's grant at the provider, best-effort.
///
/// LIMITATION(registre#50): `revoke_upstream_grant` reaches upstream for the
/// `strava` backend only — fitbit/whoop/garmin/coros disconnects stay
/// local-delete-only, and sciotte backends are scrape sessions with no OAuth
/// grant to revoke.
///
/// Strava: POST the June-2026 `/oauth/revoke` endpoint with the client
/// credentials as HTTP Basic auth and the stored refresh token (falling back
/// to the access token) as `token` — revoking either kills the whole grant,
/// and Strava answers 200 whether or not the token still existed. The token
/// value itself is never logged. `creds` arrive from the same resolution
/// chain that minted the grant, so pool-app tokens revoke under their own
/// client; `None` (already logged by the resolver's caller) skips the call.
pub async fn revoke_upstream_grant(
    data: &DataContext,
    revoke_url: &str,
    creds: Option<OAuth2Config>,
    user_id: Uuid,
    tenant_id: TenantId,
    backend: &str,
) {
    if backend != "strava" {
        return;
    }
    let Some((revoke_token, hint)) = revocation_material(data, user_id, tenant_id, backend).await
    else {
        return;
    };
    let Some(creds) = creds else {
        return;
    };

    let result = api_client()
        .post(revoke_url)
        .basic_auth(&creds.client_id, Some(&creds.client_secret))
        .form(&[("token", revoke_token.as_str()), ("token_type_hint", hint)])
        .send()
        .await;
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

/// The token material a revocation call spends: the stored refresh token when
/// present (revoking it kills the whole grant), else the access token.
/// `None` — with its reason logged — when there is nothing usable, which the
/// best-effort caller treats as "skip, local deletion proceeds".
async fn revocation_material(
    data: &DataContext,
    user_id: Uuid,
    tenant_id: TenantId,
    backend: &str,
) -> Option<(String, &'static str)> {
    let token = match data
        .repos()
        .oauth_tokens
        .get_token(user_id, tenant_id, backend)
        .await
    {
        Ok(Some(token)) => token,
        Ok(None) => {
            debug!(
                user_id = %user_id,
                backend = %backend,
                "No stored token at disconnect; nothing to revoke upstream"
            );
            return None;
        }
        Err(e) => {
            warn!(
                user_id = %user_id,
                backend = %backend,
                error = %e,
                "Could not read stored token for upstream revocation; local deletion proceeds"
            );
            return None;
        }
    };

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
