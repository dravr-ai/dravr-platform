// ABOUTME: The hosted sign-in an OAuth start of a dead grant is turned into, unless that sign-in is what failed
// ABOUTME: Shared by the web launch route, the mobile init and the hosted connect (carnet#1494, carnet#1667)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::AuthRoutesContext;
use pierre_core::errors::AppError;
use pierre_core::models::TenantId;
use pierre_middleware::provider_link_token::mint_hosted_signin_url;
use pierre_providers::backend_resolver;
use tracing::info;
use uuid::Uuid;

/// The hosted sign-in an OAuth start of `provider` is turned to, when the
/// athlete's grant of it is dead in `tenant_id`; `None` otherwise, and always
/// `None` for a start that falls back from that sign-in (`sign_in_blocked`).
///
/// A dead grant reconnects through its mirror's hosted sign-in
/// (carnet#1494), whatever client asks for the OAuth, unless that sign-in
/// could not complete and the start says so (`sign_in_blocked`, carnet#1667):
/// a shipped mobile build and the web card both still start it for a grant
/// they read as connected.
/// The start answers with the sign-in in the authorization's place, so no
/// client needs to change; the link-token is minted for `channel`. A grant
/// that writes what its session cannot (Garmin's) beside a signed-in session
/// starts its own authorization: the sign-in already serves (directive 8).
///
/// # Errors
/// The mint's error when the link-token cannot be encoded.
pub(crate) async fn dead_grant_sign_in_url(
    resources: &AuthRoutesContext,
    user_id: Uuid,
    tenant_id: TenantId,
    provider: &str,
    channel: &str,
    sign_in_blocked: bool,
) -> Result<Option<String>, AppError> {
    if sign_in_blocked {
        return Ok(None);
    }
    let Some(grant) =
        backend_resolver::flagged_grant(&resources.repos, user_id, tenant_id, provider).await
    else {
        return Ok(None);
    };
    let Some(target) = backend_resolver::reconnect_target(grant) else {
        return Ok(None);
    };
    if backend_resolver::grant_writes_beside_mirror(&resources.provider_registry, grant).is_some()
        && backend_resolver::grant_beside_session(&resources.repos, user_id, tenant_id, grant).await
    {
        return Ok(None);
    }
    info!(
        user_id = %user_id,
        provider = %provider,
        "OAuth start of a dead grant: answering with the hosted sign-in"
    );
    mint_hosted_signin_url(
        &resources.config.base_url,
        user_id,
        tenant_id.as_uuid(),
        target,
        channel,
        &resources.admin_jwt_secret,
    )
    .map(Some)
}
