// ABOUTME: Refuses a new usr/pwd Garmin sign-in while a seat is free on Garmin's OAuth, for an athlete with no Garmin grant
// ABOUTME: A re-sign of the athlete's own existing usr/pwd Garmin connection always passes, as does every other provider

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Garmin connects through its OAuth while a seat is free (carnet#1454).
//!
//! A client build that still opens the usr/pwd form for Garmin would create
//! a new usr/pwd-only athlete. [`require_garmin_grant`] refuses that sign-in
//! on the server, whatever the client: the athlete is told to go back and
//! choose Garmin, which starts its OAuth. The rule is
//! [`garmin_oauth_required`](pierre_auth::garmin_seats::garmin_oauth_required):
//! with the pool full, the usr/pwd sign-in stays the way in. An athlete who
//! already holds a usr/pwd Garmin connection keeps it: signing it in again
//! after its session died is never refused, so nobody loses their way in
//! while they have not moved. A sign-in beside a Garmin grant (the one-time
//! history copy) passes too, and so does every other provider.

use pierre_auth::garmin_seats::{garmin_oauth_required_for, holds_own_usr_pwd_garmin_connection};
use pierre_core::constants::oauth::providers as oauth_providers;
use pierre_core::errors::{AppError, ErrorCode};
use pierre_core::models::TenantId;
use serde_json::json;
use tracing::{info, warn};
use uuid::Uuid;

use crate::AuthRoutesContext;

/// `details.reason` of a usr/pwd sign-in refused because the provider now
/// connects through its own OAuth.
pub(crate) const OAUTH_REQUIRED_REASON: &str = "oauth_required";

/// Refuse a usr/pwd sign-in to `backend` for `user_id` in `tenant_id` when it
/// would make them a new usr/pwd-only Garmin athlete while a seat is free.
///
/// Refused only when every one holds: `backend` is `sciotte_garmin`, the
/// athlete holds no usr/pwd Garmin connection of their own (a connection or
/// token row) in this tenant, holds no Garmin grant there, a seat is free on
/// the server's Garmin client, and Garmin's OAuth goes first (carnet#1472).
///
/// # Errors
///
/// A [`ErrorCode::PermissionDenied`] refusal with `details.reason`
/// [`OAUTH_REQUIRED_REASON`]. Nothing is refused on a store error: the
/// athlete's way in is kept rather than closed on a guess.
pub(crate) async fn require_garmin_grant(
    resources: &AuthRoutesContext,
    user_id: Uuid,
    tenant_id: TenantId,
    backend: &str,
) -> Result<(), AppError> {
    if backend != oauth_providers::SCIOTTE_GARMIN
        || holds_usr_pwd_connection(resources, user_id, tenant_id).await
        || !garmin_oauth_required_for(
            resources.repos.oauth_tokens.as_ref(),
            resources.repos.provider_connections.as_ref(),
            resources.repos.system_settings.as_ref(),
            &resources.config.oauth,
            user_id,
            tenant_id,
        )
        .await
    {
        return Ok(());
    }
    info!(user_id = %user_id, "Usr/pwd Garmin sign-in refused: a seat is free on Garmin's OAuth");
    let mut refusal = AppError::new(
        ErrorCode::PermissionDenied,
        "Garmin now connects through its official sign-in; connect Garmin instead",
    );
    refusal.details = Some(Box::new(json!({ "reason": OAUTH_REQUIRED_REASON })));
    Err(refusal)
}

/// Whether `user_id` holds a usr/pwd Garmin connection of their own in
/// `tenant_id` ([`holds_own_usr_pwd_garmin_connection`]). A store that
/// cannot be read counts as held, so the sign-in is not refused on a guess.
async fn holds_usr_pwd_connection(
    resources: &AuthRoutesContext,
    user_id: Uuid,
    tenant_id: TenantId,
) -> bool {
    holds_own_usr_pwd_garmin_connection(
        resources.repos.oauth_tokens.as_ref(),
        resources.repos.provider_connections.as_ref(),
        user_id,
        tenant_id,
    )
    .await
    .unwrap_or_else(|e| {
        warn!(user_id = %user_id, error = %e, "Garmin connection lookup failed; the sign-in is not refused");
        true
    })
}
