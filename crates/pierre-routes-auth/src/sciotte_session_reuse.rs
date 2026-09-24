// ABOUTME: Decides whether a Sciotte credential login is answered from the session stored moments before
// ABOUTME: Only a double tap or refresh-after-connect reuses it; an older session is replaced by a real sign-in
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::{Duration as ChronoDuration, Utc};
use dravr_sciotte::models::AuthSession;
use pierre_core::constants::oauth_providers::SCIOTTE_TRAININGPEAKS;
use pierre_core::errors::AppError;
use pierre_core::models::{TenantId, UserOAuthToken};
use tracing::{info, warn};
use uuid::Uuid;

use crate::trainingpeaks_account::spawn_role_probe_if_unread;
use crate::AuthRoutesContext;

/// How recently a stored Sciotte session must have been written for a login to
/// reuse it instead of signing in again.
///
/// The reuse exists for one case: the login that stored the session was
/// answered moments ago and the same submission arrives again — a double tap
/// on the submit button, or the page refreshed right after it connected. Both
/// land within seconds of the write, so two minutes covers them with room for a
/// slow client. A deliberate reconnect comes later than that: the user saw a
/// read fail (a session the provider has revoked, which TrainingPeaks sessions
/// never announce through an expiry) or wants a different account, and in both
/// cases the credentials just submitted must be honoured, not the stored
/// session.
const SESSION_REUSE_WINDOW: ChronoDuration = ChronoDuration::minutes(2);

/// The stored session a login may answer from, or `None` when the login must
/// sign in again: the session was written outside [`SESSION_REUSE_WINDOW`], its
/// blob does not deserialise, or it carries an expiry already in the past.
fn reusable_session(
    token: &UserOAuthToken,
    user_id: Uuid,
    provider_name: &str,
) -> Option<AuthSession> {
    // Only a session written moments ago answers the login; an older one is
    // exactly what a reconnect is replacing.
    if Utc::now() - token.updated_at > SESSION_REUSE_WINDOW {
        info!(
            %user_id,
            provider = %provider_name,
            "Stored Sciotte session predates the reuse window — running a fresh login"
        );
        return None;
    }

    // Guard on deserializability so a corrupt blob falls through to a fresh
    // login rather than short-circuiting into an unusable "connected" state.
    let Ok(session) = serde_json::from_str::<AuthSession>(&token.access_token) else {
        warn!(
            %user_id,
            provider = %provider_name,
            "Stored Sciotte session blob is not deserialisable — falling through to fresh login"
        );
        return None;
    };

    // A session whose expiry is already in the past is certainly dead. Without
    // Chrome we can't probe the cookies, but this cheap local check stops us
    // returning "connected" for a session the next scrape would reject (the
    // reconnect-loop trap). A `None` expiry is "unknown, assume usable" — the
    // service re-auths on import if the cookies turn out stale.
    if let Some(expires_at) = session.expires_at {
        if expires_at <= Utc::now() {
            info!(
                %user_id,
                provider = %provider_name,
                "Stored Sciotte session is expired — falling through to fresh login"
            );
            return None;
        }
    }

    Some(session)
}

/// Answer a login from the stored Sciotte session when that session was written
/// within [`SESSION_REUSE_WINDOW`] — the double tap / refresh-after-connect case,
/// which then costs no scraper work. Any older session is ignored and the login
/// runs with the credentials submitted, so a dead session without an expiry, or
/// a coach switching accounts, gets a real sign-in (and the session store's
/// side effects, such as the roster-cache invalidation) instead of a stale
/// "connected".
///
/// A recent session is still checked locally before reuse: the blob must
/// deserialise, and any `expires_at` it carries must be in the future, so a
/// corrupt or known-expired one falls through to a fresh login.
///
/// Returns `Ok(Some(response))` when the short-circuit fires, `Ok(None)` when
/// the caller must proceed with a fresh login (no token, a session written
/// outside the window, an unparseable blob, or a known-expired session), and
/// `Err(_)` only for a hard DB failure.
pub async fn try_reuse_existing_session(
    resources: &AuthRoutesContext,
    user_id: Uuid,
    tenant_id: Uuid,
    provider_name: &str,
) -> Result<Option<Response>, AppError> {
    let tenant = TenantId::from_uuid(tenant_id);
    let Some(token) = resources
        .repos
        .oauth_tokens
        .get_token(user_id, tenant, provider_name)
        .await?
    else {
        return Ok(None);
    };

    let Some(session) = reusable_session(&token, user_id, provider_name) else {
        return Ok(None);
    };

    info!(
        %user_id,
        provider = %provider_name,
        "Reusing stored Sciotte session — short-circuiting login"
    );
    // A TrainingPeaks connection made before account roles were recorded is
    // probed now, with the session it already holds.
    if provider_name == SCIOTTE_TRAININGPEAKS {
        spawn_role_probe_if_unread(resources, user_id, tenant_id, session);
    }
    Ok(Some(
        Json(serde_json::json!({
            "status": "connected",
            "provider": provider_name,
            "short_circuit": true,
        }))
        .into_response(),
    ))
}
