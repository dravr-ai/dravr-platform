// ABOUTME: The provider authorization URL every tool and the chat's recovery mint, behind its notice and seat checks
// ABOUTME: And the link a write refused for want of a live grant carries, so the athlete can connect it again
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Minting a provider's OAuth authorization URL outside a connect surface.
//!
//! [`mint_oauth_authorize_url`] is the one mint: `connect_provider`, the
//! chat's reconnect recovery, and a write refused because the provider's own
//! grant is owed ([`with_grant_link`], JF's directive 8) all go through it,
//! so each holds the notice precondition and Garmin's seat cap.

use pierre_auth::garmin_seats::require_garmin_seat;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use pierre_providers::backend_resolver::GRANT_OWED_REASON;
use pierre_services::oauth_flow::{AuthUrlOptions, OAuthService};
use pierre_services::provider_notice::require_notice_accepted;
use serde_json::Value;
use tracing::{info, warn};
use uuid::Uuid;

use crate::runtime::ToolRuntime;

/// Mint a provider OAuth authorization URL and persist its CSRF state row.
///
/// Shared by `connect_provider` (interactive connect) and the chat auth-recovery stage
/// (reconnect a dead OAuth provider). Applies the provider's notice precondition
/// ([`require_notice_accepted`]) first, then mints the URL through the authorize builder
/// every surface shares ([`OAuthService::get_auth_url`]): the client the code exchange will
/// resolve, the provider's endpoints, scopes and PKCE, and the state row the callback
/// redeems. Returns `(authorization_url, state)`.
///
/// Neither caller can carry an acceptance: the notice (WHOOP's owner authorization) is
/// accepted by ticking it on a connect surface that shows it — the app's connect screens
/// or the hosted connect picker — never by a model's tool argument. So a provider whose
/// notice is outstanding for the account is refused here, and nothing is minted.
///
/// Garmin's OAuth takes a seat of a capped pool, so its authorization URL is minted only
/// while [`require_garmin_seat`] lets this athlete through — the rule every other surface
/// that starts Garmin's OAuth reads. With the pool full the caller offers the usr/pwd
/// sign-in instead.
///
/// # Errors
///
/// Returns the precondition's refusal (`details.action = "accept_provider_notice"`) while
/// the provider's notice is outstanding, the seat refusal ([`is_no_seat_refusal`]) for a
/// Garmin athlete with no grant while the pool is full, or an error if no OAuth app (the
/// user's own, the tenant's or the server's) is configured for the provider, the
/// authorization URL cannot be generated, or the CSRF state row cannot be persisted.
pub async fn mint_oauth_authorize_url(
    resources: &dyn ToolRuntime,
    user_id: Uuid,
    tenant_id: TenantId,
    provider: &str,
    redirect_url: Option<&str>,
) -> AppResult<(String, String)> {
    let brand = resources
        .provider_registry()
        .get_descriptor(provider)
        .map_or_else(|| provider.to_owned(), |d| d.display_name().to_owned());
    require_notice_accepted(
        resources.repos(),
        tenant_id.as_uuid(),
        user_id,
        provider,
        &brand,
        false,
    )
    .await?;
    require_garmin_seat(
        resources.repos().oauth_tokens.as_ref(),
        resources.repos().provider_connections.as_ref(),
        &resources.config().oauth,
        user_id,
        tenant_id,
        provider,
    )
    .await?;

    let authorization = OAuthService::new(resources.data(), resources.config().clone())
        .get_auth_url(
            user_id,
            tenant_id,
            provider,
            AuthUrlOptions {
                return_redirect: redirect_url,
            },
        )
        .await?;

    Ok((authorization.authorization_url, authorization.state))
}

/// `refusal` with the authorization link of the grant it names.
///
/// Only a write refused because the provider's own grant is owed
/// (`backend_resolver::grant_owed_refusal`) gains one; any other error is
/// returned as it is.
///
/// The link is the grant's own authorization, never the hosted sign-in: the
/// athlete's session already serves the reads, and only the grant writes.
/// It rides the message the model reads and `details.authorization_url`.
/// A link that cannot be minted (no seat left on Garmin's client, no OAuth
/// app) leaves the refusal without one, which still names the connection.
pub async fn with_grant_link(
    resources: &dyn ToolRuntime,
    user_id: Uuid,
    tenant_id: TenantId,
    mut refusal: AppError,
) -> AppError {
    let grant = refusal
        .details
        .as_deref()
        .filter(|details| details.get("reason").and_then(Value::as_str) == Some(GRANT_OWED_REASON))
        .and_then(|details| details.get("provider"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let Some(grant) = grant else {
        return refusal;
    };
    match mint_oauth_authorize_url(resources, user_id, tenant_id, &grant, None).await {
        Ok((url, _state)) => {
            info!(
                user_id = %user_id,
                provider = %grant,
                "Write refused for an owed grant; its authorization link offered"
            );
            refusal.message = format!("{} Authorization link: {url}", refusal.message);
            if let Some(Value::Object(details)) = refusal.details.as_deref_mut() {
                details.insert("authorization_url".to_owned(), Value::String(url));
            }
        }
        Err(e) => warn!(
            user_id = %user_id,
            provider = %grant,
            error = %e,
            "Write refused for an owed grant; no authorization link could be minted"
        ),
    }
    refusal
}
