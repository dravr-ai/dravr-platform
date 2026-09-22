// ABOUTME: Strava shared-app OAuth pool — app selection at authorize + credential resolution at exchange/refresh
// ABOUTME: The athlete's own issuing app, else env STRAVA_CLIENT_ID then DB pool apps; resolves an issuing app's secret
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Strava shared-app OAuth **pool** logic.
//!
//! Strava caps how many athletes a single OAuth app may connect and does not
//! expose that cap via the API. To grow capacity beyond the env-default
//! `STRAVA_CLIENT_ID` app, operators register additional apps in the
//! `strava_oauth_app_pool` table. This module decides which app an
//! authorization uses (the pool app that issued the athlete's token while it
//! has room, otherwise the env app first, then pool apps in order) and
//! resolves the matching `client_secret` for a token whose issuing app is
//! already known (at code exchange from the pinned state, at refresh from the
//! stored token). The env app is the implicit member with attribution `None`.
//!
//! A seat is an athlete's grant at Strava, so a token holds one until that
//! grant is no longer usable: a `revoked` connection, or a `needs_reauth` one
//! for any reason but our own client credentials, frees its seat, and
//! reconnecting re-arms the connection and takes the seat back. A
//! `needs_reauth` over our own client credentials leaves the grant live at
//! Strava and keeps its seat. An athlete whose grant holds a seat reconnects
//! on its app; when a reconnect does land on another app than one that issued
//! a token of theirs, the callback revokes the old grant once the new token is
//! stored, so no athlete is counted on two apps.

use std::collections::HashMap;

use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{StravaPoolApp, TenantId};
use pierre_database::backends::OAuthTokenRepository;
use uuid::Uuid;

use crate::config::oauth::{get_oauth_config, strava_oauth_seat_cap};

/// The Strava OAuth app chosen for an authorization.
#[derive(Debug, Clone)]
pub struct SelectedStravaApp {
    /// Attribution to persist on the client-state (and later the token). `None`
    /// = the env-default app; `Some(client_id)` = a pool app.
    pub attribution: Option<String>,
    /// `client_id` to embed in the authorize URL (env or pool).
    pub client_id: String,
    /// Matching `client_secret`, used immediately at the code exchange.
    pub client_secret: String,
}

/// Aggregate Strava OAuth seat capacity across the env app and the pool.
#[derive(Debug, Clone, Copy)]
pub struct StravaSeatSummary {
    /// Sum of the env app's cap and every enabled pool app's cap.
    pub total: u32,
    /// Distinct non-BYO athletes whose grant is still usable — their
    /// connection is neither `revoked` nor `needs_reauth` for anything but our
    /// own client credentials — occupying those seats, each app's count capped
    /// at its `seat_cap`.
    pub used: u32,
}

impl StravaSeatSummary {
    /// Seats still free across the whole pool.
    #[must_use]
    pub const fn left(&self) -> u32 {
        self.total.saturating_sub(self.used)
    }
}

/// Env-default app credentials from the environment.
fn env_app() -> AppResult<(String, String)> {
    let cfg = get_oauth_config("strava");
    match (cfg.client_id, cfg.client_secret) {
        (Some(id), Some(secret)) => Ok((id, secret)),
        _ => Err(AppError::invalid_input(
            "Strava env OAuth app not configured (STRAVA_CLIENT_ID/STRAVA_CLIENT_SECRET)",
        )),
    }
}

/// Seat usage split into (env-app usage, per-pool-app usage). The env bucket is
/// the `None` attribution (env-issued + pre-pool legacy tokens). A token whose
/// grant is known dead holds no seat; `excluded_user`, when set, is left out
/// of every bucket.
async fn usage(
    oauth_tokens: &dyn OAuthTokenRepository,
    excluded_user: Option<Uuid>,
) -> AppResult<(u32, HashMap<String, u32>)> {
    let mut env_used = 0u32;
    let mut per_app = HashMap::new();
    for (app, n) in oauth_tokens
        .count_strava_seat_usage_by_app(excluded_user)
        .await?
    {
        match app {
            None => env_used = env_used.saturating_add(n),
            Some(cid) => {
                per_app.insert(cid, n);
            }
        }
    }
    Ok((env_used, per_app))
}

/// Whether `app` has a free seat under `per_app` usage.
fn pool_app_has_room(app: &StravaPoolApp, per_app: &HashMap<String, u32>) -> bool {
    per_app.get(&app.client_id).copied().unwrap_or(0) < app.seat_cap
}

/// The selection for a listed pool app, with its decrypted secret.
async fn pool_selection(
    oauth_tokens: &dyn OAuthTokenRepository,
    app: &StravaPoolApp,
) -> AppResult<SelectedStravaApp> {
    let secret = oauth_tokens
        .get_strava_pool_app_secret(&app.client_id)
        .await?
        .ok_or_else(|| {
            AppError::invalid_input("Strava pool app secret missing for a listed app")
        })?;
    Ok(SelectedStravaApp {
        attribution: Some(app.client_id.clone()),
        client_id: app.client_id.clone(),
        client_secret: secret,
    })
}

/// Choose the Strava OAuth app for an athlete's authorization, first connect
/// or reconnect.
///
/// Every app's seats are counted for everyone except this athlete, so the
/// athlete's own token — live or dead — never counts against the app they are
/// choosing. The athlete's stored token (a seat-holding one first, then the
/// one in `tenant_id`, then their newest in any tenant, since Strava counts
/// them per app whatever our tenant) decides where they start:
///
/// 1. A token that still holds its seat keeps the athlete on its app, the env
///    app or an enabled pool app, whether or not that app has room: Strava
///    already counts them there, so the authorization adds no athlete to it,
///    and sending them elsewhere would put them on two apps.
/// 2. A token that holds no seat on an enabled pool app reconnects on that
///    app while it has room.
/// 3. Otherwise the fill order applies: the env-default app first (keeps the
///    common path on env, no DB decrypt), then enabled pool apps in insertion
///    order, each until its `seat_cap` is reached.
///
/// A disabled pool app is never chosen, so its athletes move off it as they
/// reconnect. Landing an athlete on another app than the one that issued a
/// token of theirs is a switch the callback completes by revoking the old
/// grant once the new token is stored (`strava_reconnect`).
///
/// # Errors
/// Returns an error when the env app is chosen but unconfigured, or when every
/// app is at capacity (the caller should be gating on [`strava_seat_summary`]
/// and so only reach here while a seat remains).
pub async fn select_strava_app(
    oauth_tokens: &dyn OAuthTokenRepository,
    user_id: Uuid,
    tenant_id: TenantId,
) -> AppResult<SelectedStravaApp> {
    let (env_used, per_app) = usage(oauth_tokens, Some(user_id)).await?;
    let pool_apps = oauth_tokens.list_strava_pool_apps(true).await?;

    if let Some(current) = oauth_tokens
        .list_strava_token_apps(user_id, tenant_id)
        .await?
        .into_iter()
        .next()
    {
        match current.attribution.as_deref() {
            None if current.holds_seat => return env_selection(),
            None => {}
            Some(client_id) => {
                if let Some(app) = pool_apps.iter().find(|app| {
                    app.client_id == client_id
                        && (current.holds_seat || pool_app_has_room(app, &per_app))
                }) {
                    return pool_selection(oauth_tokens, app).await;
                }
            }
        }
    }

    if env_used < strava_oauth_seat_cap() {
        return env_selection();
    }
    if let Some(app) = pool_apps
        .iter()
        .find(|app| pool_app_has_room(app, &per_app))
    {
        return pool_selection(oauth_tokens, app).await;
    }
    Err(AppError::invalid_input(
        "No Strava OAuth seats available across the env app and pool",
    ))
}

/// The env-default app as a selection.
fn env_selection() -> AppResult<SelectedStravaApp> {
    let (client_id, client_secret) = env_app()?;
    Ok(SelectedStravaApp {
        attribution: None,
        client_id,
        client_secret,
    })
}

/// The Strava `client_id` an attribution names: the pool app's own id, or
/// the env app's for `None`. `None` when the env app is unconfigured.
#[must_use]
pub fn strava_client_id(attribution: Option<&str>) -> Option<String> {
    attribution.map_or_else(
        || get_oauth_config("strava").client_id,
        |id| Some(id.to_owned()),
    )
}

/// Whether two attributions name the same Strava application.
///
/// Compared by the `client_id` each resolves to rather than by attribution,
/// so a pool entry registered under the env app's own `client_id` reads as
/// the env app it is: Strava holds one grant per athlete and application, and
/// telling the two apart would revoke the grant a reconnect just made.
#[must_use]
pub fn same_strava_app(a: Option<&str>, b: Option<&str>) -> bool {
    a == b || strava_client_id(a) == strava_client_id(b)
}

/// Resolve `(client_id, client_secret)` for a KNOWN attribution.
///
/// Used at code exchange (the state's pinned app) and at refresh (a stored
/// token's issuing app). `None` (env) resolves to the env app; a pool id
/// resolves to that app's decrypted secret. An unknown/removed pool id falls
/// back to the env app so an orphaned token degrades rather than hard-failing
/// (only a deleted-with-live-athletes app hits this, which the delete path
/// warns against).
///
/// # Errors
/// Returns an error when the env app is unconfigured or the repository fails.
pub async fn resolve_strava_credentials(
    oauth_tokens: &dyn OAuthTokenRepository,
    attribution: Option<&str>,
) -> AppResult<(String, String)> {
    if let Some(cid) = attribution {
        if let Some(secret) = oauth_tokens.get_strava_pool_app_secret(cid).await? {
            return Ok((cid.to_owned(), secret));
        }
    }
    env_app()
}

/// Total and used Strava OAuth seats across the env app and the pool.
///
/// The connect recommender uses this to keep offering OAuth while any seat
/// (env or pool) remains, before falling back to the Sciotte mirror.
///
/// # Errors
/// Returns an error only on a repository failure.
pub async fn strava_seat_summary(
    oauth_tokens: &dyn OAuthTokenRepository,
) -> AppResult<StravaSeatSummary> {
    let (env_used, per_app) = usage(oauth_tokens, None).await?;
    let env_cap = strava_oauth_seat_cap();
    let mut total = env_cap;
    let mut used = env_used.min(env_cap);
    for app in oauth_tokens.list_strava_pool_apps(true).await? {
        total = total.saturating_add(app.seat_cap);
        let app_used = per_app
            .get(&app.client_id)
            .copied()
            .unwrap_or(0)
            .min(app.seat_cap);
        used = used.saturating_add(app_used);
    }
    Ok(StravaSeatSummary { total, used })
}
