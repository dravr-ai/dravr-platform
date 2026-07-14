// ABOUTME: Strava shared-app OAuth pool — app selection at authorize + credential resolution at exchange/refresh
// ABOUTME: Env-default STRAVA_CLIENT_ID app first, then DB pool apps; resolves the issuing app's secret for a given token
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Strava shared-app OAuth **pool** logic.
//!
//! Strava caps how many athletes a single OAuth app may connect and does not
//! expose that cap via the API. To grow capacity beyond the env-default
//! `STRAVA_CLIENT_ID` app, operators register additional apps in the
//! `strava_oauth_app_pool` table. This module decides which app a new
//! connection uses (filling the env app first, then pool apps in order) and
//! resolves the matching `client_secret` for a token whose issuing app is
//! already known (at code exchange from the pinned state, at refresh from the
//! stored token). The env app is the implicit member with attribution `None`.

use std::collections::HashMap;

use pierre_core::errors::{AppError, AppResult};
use pierre_database::backends::OAuthTokenRepository;

use crate::config::oauth::{get_oauth_config, strava_oauth_seat_cap};

/// The Strava OAuth app chosen for a new authorization.
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
    /// Distinct non-BYO athletes currently occupying those seats.
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
/// the `None` attribution (env-issued + pre-pool legacy tokens).
async fn usage(oauth_tokens: &dyn OAuthTokenRepository) -> AppResult<(u32, HashMap<String, u32>)> {
    let mut env_used = 0u32;
    let mut per_app = HashMap::new();
    for (app, n) in oauth_tokens.count_strava_seat_usage_by_app().await? {
        match app {
            None => env_used = env_used.saturating_add(n),
            Some(cid) => {
                per_app.insert(cid, n);
            }
        }
    }
    Ok((env_used, per_app))
}

/// Choose the Strava OAuth app for a NEW connection.
///
/// Fills the env-default app first (keeps the common path on env, no DB
/// decrypt), then enabled pool apps in insertion order, each until its
/// `seat_cap` is reached.
///
/// # Errors
/// Returns an error when the env app is unconfigured, or when every app is at
/// capacity (the caller should be gating on [`strava_seat_summary`] and so only
/// reach here while a seat remains).
pub async fn select_strava_app(
    oauth_tokens: &dyn OAuthTokenRepository,
) -> AppResult<SelectedStravaApp> {
    let (env_used, per_app) = usage(oauth_tokens).await?;
    if env_used < strava_oauth_seat_cap() {
        let (client_id, client_secret) = env_app()?;
        return Ok(SelectedStravaApp {
            attribution: None,
            client_id,
            client_secret,
        });
    }
    for app in oauth_tokens.list_strava_pool_apps(true).await? {
        let used = per_app.get(&app.client_id).copied().unwrap_or(0);
        if used < app.seat_cap {
            let secret = oauth_tokens
                .get_strava_pool_app_secret(&app.client_id)
                .await?
                .ok_or_else(|| {
                    AppError::invalid_input("Strava pool app secret missing for a listed app")
                })?;
            return Ok(SelectedStravaApp {
                attribution: Some(app.client_id.clone()),
                client_id: app.client_id,
                client_secret: secret,
            });
        }
    }
    Err(AppError::invalid_input(
        "No Strava OAuth seats available across the env app and pool",
    ))
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
    let (env_used, per_app) = usage(oauth_tokens).await?;
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
