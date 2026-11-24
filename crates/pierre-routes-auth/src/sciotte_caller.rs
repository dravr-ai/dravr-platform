// ABOUTME: Who is calling a sciotte sign-in route: a Pierre session, or a channel's provider link-token
// ABOUTME: Carries a link-token's channel context so a sign-in it started posts back to its conversation
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use axum::http::HeaderMap;
use pierre_core::errors::AppError;
use pierre_middleware::provider_link_token::{
    extract_bearer_link_token, verify_link_token, ProviderLinkTokenClaims,
};
use pierre_runtime_context::require_tenant_in;
use uuid::Uuid;

use crate::AuthRoutesContext;

/// Emit a fire-and-forget webhook announcing the successful provider connection.
/// The channel link context tells the downstream bot where to post the confirmation.
pub(crate) fn emit_provider_linked_webhook(
    user_id: Uuid,
    tenant_id: Uuid,
    provider_name: &str,
    ctx: &LinkContext,
) {
    use crate::provider_link_webhook::{spawn_emit, ProviderLinkedEvent};
    let event = ProviderLinkedEvent::new(
        user_id,
        tenant_id,
        provider_name.to_owned(),
        ctx.target.clone(),
        ctx.channel.clone(),
        ctx.channel_thread.clone(),
    );
    spawn_emit(event);
}

/// Channel context carried by a provider link-token — used to emit the
/// post-back webhook after a successful connection.
#[derive(Debug, Clone)]
pub(crate) struct LinkContext {
    /// Channel slug the token was minted for (e.g. "slack")
    pub channel: String,
    /// Optional channel thread/DM id so the bot can reply in the original thread
    pub channel_thread: Option<String>,
    /// Target platform (strava / garmin / trainingpeaks / coros) from the claims
    pub target: String,
}

/// Extract authenticated `user_id` and `tenant_id` from request headers.
///
/// Accepts two authentication methods, tried in order:
/// 1. A channel-initiated provider link-token (`Authorization: Bearer <link-token>`),
///    minted in-process by the messaging pipeline or the backfill notifier. This
///    is the path used by the hosted Sciotte login page embedded in chat
///    channels, and it reaches only the login, 2FA, OTP and cancel routes.
/// 2. A standard Pierre session (cookie or Bearer JWT) — used by the web/mobile
///    `SciotteLoginModal` component.
///
/// When the link-token path is taken, the returned `LinkContext` carries the
/// channel metadata needed to emit a post-back webhook on success.
///
/// `connecting` names the step when it signs in to a provider (a login, a
/// second factor, a code, a browser hand-over): an admin's support session
/// is refused it, since the session it stores outlives the support session
/// (carnet#1634). `None` for what leaves nothing behind, such as a cancel.
pub(crate) async fn authenticate(
    resources: &AuthRoutesContext,
    headers: &HeaderMap,
    connecting: Option<&str>,
) -> Result<(Uuid, Uuid, Option<LinkContext>), AppError> {
    if let Some(claims) = try_verify_link_token_from_headers(resources, headers) {
        let user_id = Uuid::parse_str(&claims.sub).map_err(|_| {
            AppError::auth_invalid("Provider link-token has invalid user_id in sub claim")
        })?;
        let tenant_id = Uuid::parse_str(&claims.tid).map_err(|_| {
            AppError::auth_invalid("Provider link-token has invalid tenant_id in tid claim")
        })?;
        let link_context = LinkContext {
            channel: claims.channel,
            channel_thread: claims.channel_thread,
            target: claims.tgt,
        };
        return Ok((user_id, tenant_id, Some(link_context)));
    }

    let auth_result = resources
        .auth_middleware
        .authenticate_request_with_headers(headers)
        .await?;
    if let Some(action) = connecting {
        auth_result
            .refuse_impersonation(resources.repos.admin_audit.as_ref(), action)
            .await?;
    }
    let tenants = resources.repos.tenants.as_ref();
    let tenant = require_tenant_in(tenants, &auth_result).await?;
    Ok((auth_result.user_id, tenant.as_uuid(), None))
}

/// If an `Authorization: Bearer ...` header contains a Sciotte-scoped link-token,
/// verify it against the server's admin JWT secret and return the claims.
/// Returns None when the header is absent, not a Bearer token, or fails verification
/// as a link-token (so the caller falls through to normal auth).
fn try_verify_link_token_from_headers(
    resources: &AuthRoutesContext,
    headers: &HeaderMap,
) -> Option<ProviderLinkTokenClaims> {
    let header_value = headers.get("authorization").and_then(|h| h.to_str().ok());
    let token = extract_bearer_link_token(header_value)?;
    verify_link_token(token, &resources.admin_jwt_secret, "sciotte").ok()
}
