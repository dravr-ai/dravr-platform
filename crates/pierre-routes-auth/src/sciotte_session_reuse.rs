// ABOUTME: Decides whether a Sciotte credential login is answered from the session stored moments before
// ABOUTME: Only a double tap or refresh-after-connect reuses it; only one observed dead seeds the sign-in's browser
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::{Duration as ChronoDuration, Utc};
use dravr_sciotte::models::AuthSession;
use pierre_core::constants::oauth_providers::SCIOTTE_TRAININGPEAKS;
use pierre_core::errors::AppError;
use pierre_core::models::{connection_needs_reauth, TenantId, UserOAuthToken};
use pierre_providers::token_writeback::StoredTokenRow;
use secrecy::ExposeSecret;
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

/// What a credential login does with the session stored for its provider.
pub(crate) enum StoredSessionUse {
    /// The session was written within [`SESSION_REUSE_WINDOW`] and is not
    /// observed dead (see [`SessionHealth`]): the login is answered from it,
    /// with this response.
    Reused(Response),
    /// The login signs in with the credentials submitted.
    SignIn {
        /// The caller's own stored session, sent to seed the login browser,
        /// and only when the platform has observed it dead: its `expires_at`
        /// is in the past, or the caller's connection to the same provider is
        /// flagged `needs_reauth` or `revoked` because a read through it
        /// failed at the provider. A session the provider refused has dead
        /// sign-in cookies, so seeding them cannot sign the browser in to
        /// the old account, while its device-trust cookies still let the
        /// provider see the device it trusted at that sign-in rather than a
        /// stranger it challenges.
        ///
        /// A stored session not observed dead — no past `expires_at` and an
        /// `active` connection — is never sent, however old it is. Its
        /// sign-in cookies may still be live, and the reconnect it is part of
        /// may be a coach switching to another account: seeded with account
        /// A's live cookies, the provider could skip its sign-in form, and
        /// the session stored afterwards would be account A's filed under
        /// account B's credentials. Such a login goes in with `None`.
        ///
        /// `None` also on a fresh connect, and when the stored blob does not
        /// deserialise.
        prior_session: Option<AuthSession>,
    },
}

/// What the platform has observed about a stored session's liveness.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SessionHealth {
    /// Nothing says it is dead: no expiry already past, and its connection is
    /// `active`. Its sign-in cookies may still be live.
    NotObservedDead,
    /// Its `expires_at` is past, or the caller's connection to its provider
    /// requires re-authorizing because a read through it failed at the
    /// provider. Its sign-in cookies cannot sign anyone in.
    ObservedDead,
}

/// Whether `session` carries an expiry already past. A session without one is
/// "unknown, assume usable": not expired.
fn has_expired(session: &AuthSession) -> bool {
    session
        .expires_at
        .is_some_and(|expires_at| expires_at <= Utc::now())
}

/// Read what the platform has observed about `session`: its own expiry, and
/// the status of the caller's connection to `provider_name`, read by
/// `(user_id, tenant, provider_name)` so only the caller's own connection to
/// that provider can count.
///
/// Sciotte sessions carry no `expires_at` in practice, so the connection is
/// what records a session the provider refused: a scrape answered
/// `session_expired` flags it `needs_reauth`.
///
/// # Errors
///
/// Only a hard DB failure.
async fn session_health(
    resources: &AuthRoutesContext,
    session: &AuthSession,
    user_id: Uuid,
    tenant: TenantId,
    provider_name: &str,
) -> Result<SessionHealth, AppError> {
    if has_expired(session) {
        return Ok(SessionHealth::ObservedDead);
    }
    let connections = resources
        .repos
        .provider_connections
        .get_for_user(user_id, Some(tenant))
        .await?;
    Ok(if connection_needs_reauth(&connections, provider_name) {
        SessionHealth::ObservedDead
    } else {
        SessionHealth::NotObservedDead
    })
}

/// Whether the session read from `token` answers a login on its own: written
/// within [`SESSION_REUSE_WINDOW`] and not observed dead.
fn answers_login(
    token: &UserOAuthToken,
    health: SessionHealth,
    user_id: Uuid,
    provider_name: &str,
) -> bool {
    // Only a session written moments ago answers the login; an older one is
    // exactly what a reconnect is replacing.
    if Utc::now() - token.updated_at > SESSION_REUSE_WINDOW {
        info!(
            %user_id,
            provider = %provider_name,
            "Stored Sciotte session predates the reuse window — running a fresh login"
        );
        return false;
    }

    // A session whose expiry is already past, or whose connection a refused
    // read flagged, is certainly dead. Without Chrome we can't probe the
    // cookies, but these cheap local checks stop us returning "connected" for
    // a session the next scrape would reject (the reconnect-loop trap). A
    // session with no expiry on an `active` connection is "unknown, assume
    // usable" — the service re-auths on import if the cookies turn out stale.
    if health == SessionHealth::ObservedDead {
        info!(
            %user_id,
            provider = %provider_name,
            "Stored Sciotte session is observed dead — falling through to fresh login"
        );
        return false;
    }
    true
}

/// Decide what a credential login does with the caller's stored Sciotte
/// session for `provider_name`.
///
/// A session written within [`SESSION_REUSE_WINDOW`] answers the login — the
/// double tap / refresh-after-connect case, which then costs no scraper work.
/// Any older session is not reused: the login runs with the credentials
/// submitted, so a dead session without an expiry, or a coach switching
/// accounts, gets a real sign-in (and the session store's side effects, such
/// as the roster-cache invalidation) instead of a stale "connected". That
/// sign-in carries the old session as its prior session only when the
/// platform has observed the old session dead — its `expires_at` is past, or
/// the caller's connection to the provider is `needs_reauth` or `revoked` —
/// so the provider sees the device it trusted last time rather than a
/// stranger it challenges. A session not observed dead is never sent: its
/// cookies may still sign the browser in to the old account, which on a
/// coach's account switch would store account A's session under account B's
/// credentials.
///
/// A recent session is still checked locally before reuse: the blob must
/// deserialise, and it must not be observed dead.
///
/// The session and the connection are both read by
/// `(user_id, tenant_id, provider_name)`: only the caller's own session can
/// answer the login or seed its browser, and only the caller's own connection
/// to that provider can mark it dead.
///
/// # Errors
///
/// Only a hard DB failure.
pub(crate) async fn stored_session_use(
    resources: &AuthRoutesContext,
    user_id: Uuid,
    tenant_id: Uuid,
    provider_name: &str,
) -> Result<StoredSessionUse, AppError> {
    let tenant = TenantId::from_uuid(tenant_id);
    let Some(token) = resources
        .repos
        .oauth_tokens
        .get_token(user_id, tenant, provider_name)
        .await?
    else {
        return Ok(StoredSessionUse::SignIn {
            prior_session: None,
        });
    };

    // Guard on deserializability so a corrupt blob falls through to a fresh
    // login rather than short-circuiting into an unusable "connected" state,
    // or being sent to seed a login browser.
    let Ok(session) = serde_json::from_str::<AuthSession>(token.access_token.expose_secret())
    else {
        warn!(
            %user_id,
            provider = %provider_name,
            "Stored Sciotte session blob is not deserialisable — falling through to fresh login"
        );
        return Ok(StoredSessionUse::SignIn {
            prior_session: None,
        });
    };

    let health = session_health(resources, &session, user_id, tenant, provider_name).await?;
    if !answers_login(&token, health, user_id, provider_name) {
        // Only a session observed dead seeds the sign-in: one that may be live
        // could sign the browser in to the account a coach is switching away
        // from.
        let prior_session = (health == SessionHealth::ObservedDead).then_some(session);
        return Ok(StoredSessionUse::SignIn { prior_session });
    }

    info!(
        %user_id,
        provider = %provider_name,
        "Reusing stored Sciotte session — short-circuiting login"
    );
    // A TrainingPeaks connection made before account roles were recorded is
    // probed now, with the session it already holds.
    if provider_name == SCIOTTE_TRAININGPEAKS {
        match StoredTokenRow::of(&token) {
            Ok(read_from) => {
                spawn_role_probe_if_unread(resources, user_id, tenant_id, session, read_from);
            }
            Err(e) => warn!(
                %user_id,
                error = %e,
                "TrainingPeaks role probe skipped: the stored row cannot be named"
            ),
        }
    }
    Ok(StoredSessionUse::Reused(
        Json(serde_json::json!({
            "status": "connected",
            "provider": provider_name,
            "short_circuit": true,
        }))
        .into_response(),
    ))
}
