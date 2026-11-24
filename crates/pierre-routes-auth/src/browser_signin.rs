// ABOUTME: A provider sign-in run in the athlete's own browser: how each provider is signed in to, and the session it hands over
// ABOUTME: The extension or the app's WebView opens the provider's page, then posts that provider's cookies here (carnet#1617)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The sign-in that runs in the athlete's own browser (carnet#1617).
//!
//! A provider can refuse a sign-in from a cloud browser and still serve reads
//! to the session it signed in: Garmin's SSO has put a reCAPTCHA in front of
//! every sign-in from Dravr's egress since 2026-11-16 while sciotte's reads
//! from the same egress pass. So the sign-in moves to the athlete: the Dravr
//! browser extension, or the app's `WebView`, opens the provider's own sign-in
//! page, waits for its signed-in landing and hands Dravr that provider's
//! cookies. sciotte then reads with them as after any sign-in.
//!
//! - `GET /api/providers/browser-signin` is sciotte's
//!   [`BrowserSigninCatalog`]: per provider the page to open, the URLs that
//!   say the athlete is signed in and the domain of the session's cookies.
//!   sciotte's provider configs are its one source.
//! - `POST /api/providers/browser-signin/{target}` takes the cookies. Only
//!   cookies set under the provider's domain are accepted, and the session is
//!   stored only once a read through it succeeds, then exactly as a sciotte
//!   sign-in's ([`store_sciotte_session`]): its exposure notice first, the
//!   connection, the notice, the warm-up.
//!
//! Both answer only the athlete's own session (web cookie or app bearer): a
//! channel's link-token opens a hosted page, which runs no browser of the
//! athlete's that this could read.

use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::response::Response;
use axum::Json;
use dravr_sciotte::client::ClientError;
use dravr_sciotte::models::{AuthSession, CookieData};
use dravr_sciotte::wire::{BrowserSignin, BrowserSigninCatalog};
use dravr_tronc::dates::Timestamp;
use pierre_core::errors::{AppError, ErrorCode};
use pierre_providers::sciotte_error;
use pierre_providers::sciotte_provider::SciotteTarget;
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use tracing::{info, warn};
use uuid::Uuid;

use crate::sciotte::store_sciotte_session;
use crate::sciotte_caller;
use crate::sciotte_terms::require_provider_terms;
use crate::AuthRoutesContext;

/// The most cookies one hand-over may carry. A signed-in provider sets a few
/// dozen at most (Garmin: 14 on 2026-11-17).
const MAX_COOKIES: usize = 100;

/// The longest cookie value accepted, a browser's own per-cookie limit.
const MAX_COOKIE_VALUE_BYTES: usize = 4096;

/// One cookie as the athlete's browser read it. Its value is a live session
/// credential, so it is a [`SecretString`] and prints `[REDACTED]`.
#[derive(Debug, Deserialize)]
pub(crate) struct BrowserCookie {
    name: String,
    value: SecretString,
    domain: String,
    #[serde(default = "root_path")]
    path: String,
    #[serde(default)]
    secure: bool,
    #[serde(default)]
    http_only: bool,
    /// Seconds since the Unix epoch, as the browser reports it; absent for a
    /// session cookie.
    #[serde(default)]
    expires: Option<f64>,
}

fn root_path() -> String {
    "/".to_owned()
}

/// Body of `POST /api/providers/browser-signin/{target}`.
#[derive(Debug, Deserialize)]
pub(crate) struct BrowserSigninRequest {
    cookies: Vec<BrowserCookie>,
    /// The athlete accepted the provider's exposure notice on this connect.
    #[serde(default)]
    tos_consent: bool,
    /// The offered consents the athlete ticked on this connect (Garmin's AI
    /// use and workout transfer).
    #[serde(default)]
    consents: Vec<String>,
}

/// `GET /api/providers/browser-signin`: how each provider is signed in to
/// from the athlete's own browser, as sciotte serves it.
pub(crate) async fn handle_browser_signin_catalog(
    State(resources): State<AuthRoutesContext>,
    headers: HeaderMap,
) -> Result<Json<BrowserSigninCatalog>, AppError> {
    own_session(&resources, &headers, None).await?;
    let client = resources.provider_registry.require_sciotte_login_client()?;
    let catalog = client
        .browser_signin()
        .await
        .map_err(sciotte_error::to_app_error)?;
    Ok(Json(catalog))
}

/// `POST /api/providers/browser-signin/{target}`: store the session the
/// athlete's browser signed in to `target` (`garmin`, `strava`,
/// `trainingpeaks`, `coros`), once a read through it succeeds.
pub(crate) async fn handle_browser_signin(
    State(resources): State<AuthRoutesContext>,
    headers: HeaderMap,
    Path(target): Path<String>,
    Json(request): Json<BrowserSigninRequest>,
) -> Result<Response, AppError> {
    let (user_id, tenant_id) = own_session(
        &resources,
        &headers,
        Some("hand over a provider sign-in from the browser"),
    )
    .await?;
    let target = target_named(&target)?;
    require_provider_terms(
        &resources,
        user_id,
        tenant_id,
        target,
        request.tos_consent,
        &request.consents,
    )
    .await?;

    let entry = catalog_entry(&resources, target).await?;
    let session = session_of(&request.cookies, &entry.cookie_domain)?;
    verify_session(&resources, &session, target).await?;

    info!(
        user_id = %user_id,
        provider = target.provider_name(),
        cookies = session.cookies.len(),
        "Browser sign-in handed over a session; storing it"
    );
    store_sciotte_session(
        &resources,
        user_id,
        tenant_id,
        &session,
        target.provider_name(),
    )
    .await
}

/// The athlete's own session, never a channel's link-token; refused to an
/// admin's support session when `connecting` names a sign-in.
async fn own_session(
    resources: &AuthRoutesContext,
    headers: &HeaderMap,
    connecting: Option<&str>,
) -> Result<(Uuid, Uuid), AppError> {
    let (user_id, tenant_id, link) =
        sciotte_caller::authenticate(resources, headers, connecting).await?;
    if link.is_some() {
        return Err(AppError::new(
            ErrorCode::PermissionDenied,
            "A sign-in from your browser needs you signed in to Dravr itself",
        ));
    }
    Ok((user_id, tenant_id))
}

/// The provider `target` names, refusing any other: unlike a sign-in's
/// target, nothing here defaults to Strava.
fn target_named(target: &str) -> Result<SciotteTarget, AppError> {
    match target {
        "garmin" => Ok(SciotteTarget::Garmin),
        "strava" => Ok(SciotteTarget::Strava),
        "trainingpeaks" => Ok(SciotteTarget::TrainingPeaks),
        "coros" => Ok(SciotteTarget::Coros),
        other => Err(AppError::invalid_input(format!(
            "No browser sign-in for provider {other:?}"
        ))),
    }
}

/// sciotte's [`BrowserSignin`] for `target`.
async fn catalog_entry(
    resources: &AuthRoutesContext,
    target: SciotteTarget,
) -> Result<BrowserSignin, AppError> {
    let client = resources.provider_registry.require_sciotte_login_client()?;
    let catalog = client
        .browser_signin()
        .await
        .map_err(sciotte_error::to_app_error)?;
    let provider = target.scraper_provider_name();
    catalog
        .providers
        .into_iter()
        .find(|entry| entry.provider == provider)
        .ok_or_else(|| {
            AppError::invalid_input(format!("{provider} offers no sign-in from your browser"))
        })
}

/// The session `cookies` make, refusing a hand-over that is empty, too large
/// or carries a cookie set under any domain but `cookie_domain`.
fn session_of(cookies: &[BrowserCookie], cookie_domain: &str) -> Result<AuthSession, AppError> {
    if cookies.is_empty() {
        return Err(AppError::invalid_input(
            "The browser handed over no cookies: sign in to the provider first",
        ));
    }
    if cookies.len() > MAX_COOKIES {
        return Err(AppError::invalid_input(format!(
            "At most {MAX_COOKIES} cookies are accepted"
        )));
    }
    let mut kept = Vec::with_capacity(cookies.len());
    for cookie in cookies {
        let host = cookie.domain.trim_start_matches('.');
        if host != cookie_domain && !host.ends_with(&format!(".{cookie_domain}")) {
            return Err(AppError::invalid_input(format!(
                "Cookie {:?} is set under {:?}, not under {cookie_domain}",
                cookie.name, cookie.domain
            )));
        }
        let value = cookie.value.expose_secret();
        if cookie.name.is_empty() || value.len() > MAX_COOKIE_VALUE_BYTES {
            return Err(AppError::invalid_input(format!(
                "Cookie {:?} has no name or a value over {MAX_COOKIE_VALUE_BYTES} bytes",
                cookie.name
            )));
        }
        kept.push(CookieData {
            name: cookie.name.clone(),
            value: value.to_owned(),
            domain: cookie.domain.clone(),
            path: cookie.path.clone(),
            secure: cookie.secure,
            http_only: cookie.http_only,
            expires: cookie.expires,
        });
    }
    Ok(AuthSession {
        session_id: format!("browser-{}", Uuid::new_v4().simple()),
        cookies: kept,
        created_at: Timestamp::now(),
        expires_at: None,
    })
}

/// Read the athlete's profile through `session` on the service its reads go
/// to: a session the provider does not accept is refused before anything is
/// stored, and the copy the read service imported is dropped, since it holds
/// the provider's cookies and no stored session will ever read through it.
///
/// The refusal is a `400`, never a `401`: a `401` tells the app the athlete's
/// own Dravr session failed, and the client re-authenticates or signs them
/// out. A fault of the read service keeps its own code.
async fn verify_session(
    resources: &AuthRoutesContext,
    session: &AuthSession,
    target: SciotteTarget,
) -> Result<(), AppError> {
    let client = resources
        .provider_registry
        .require_sciotte_reads()?
        .for_session(&session.session_id);
    let provider = target.scraper_provider_name();
    let session_id = client
        .import_session(session, provider)
        .await
        .map_err(sciotte_error::to_app_error)?;
    let Err(error) = client.get_athlete(&session_id).await else {
        return Ok(());
    };
    if let Err(e) = client.delete_session(&session_id).await {
        warn!(
            provider,
            error = %e,
            "Could not drop a browser sign-in's session from the read service; it lapses with its idle lifetime"
        );
    }
    match error {
        ClientError::SessionExpired { .. } | ClientError::SessionNotFound { .. } => {
            warn!(provider, error = %error, "Browser sign-in handed over a session the provider did not accept");
            Err(AppError::invalid_input(
                "The provider did not accept this sign-in; sign in again",
            ))
        }
        other => Err(sciotte_error::to_app_error(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cookie(name: &str, domain: &str) -> BrowserCookie {
        cookie_valued(name, domain, "v")
    }

    fn cookie_valued(name: &str, domain: &str, value: &str) -> BrowserCookie {
        BrowserCookie {
            name: name.to_owned(),
            value: SecretString::from(value.to_owned()),
            domain: domain.to_owned(),
            path: root_path(),
            secure: true,
            http_only: true,
            expires: None,
        }
    }

    #[test]
    fn cookies_under_the_providers_domain_make_the_session() {
        let cookies = [
            cookie("CASTGC", "sso.garmin.com"),
            cookie("GARMIN-SSO", ".garmin.com"),
            cookie("SESSIONID", "connect.garmin.com"),
        ];
        let session = session_of(&cookies, "garmin.com").unwrap();
        assert_eq!(
            session.cookies.len(),
            3,
            "bug: a cookie under garmin.com was dropped from the session"
        );
        assert!(
            session.session_id.starts_with("browser-"),
            "bug: a browser sign-in's session id is {}",
            session.session_id
        );
    }

    #[test]
    fn a_cookie_of_another_domain_refuses_the_hand_over() {
        for domain in ["strava.com", "evilgarmin.com", "garmin.com.evil.test"] {
            let cookies = [
                cookie("SESSIONID", "connect.garmin.com"),
                cookie("x", domain),
            ];
            assert!(
                session_of(&cookies, "garmin.com").is_err(),
                "bug: a cookie under {domain} was accepted for garmin.com"
            );
        }
    }

    #[test]
    fn an_empty_or_oversized_hand_over_is_refused() {
        assert!(
            session_of(&[], "garmin.com").is_err(),
            "bug: a hand-over with no cookies was accepted"
        );
        let mut many: Vec<_> = (0..MAX_COOKIES)
            .map(|i| cookie(&format!("c{i}"), "garmin.com"))
            .collect();
        assert_eq!(
            session_of(&many, "garmin.com")
                .map(|s| s.cookies.len())
                .ok(),
            Some(MAX_COOKIES),
            "bug: a hand-over of exactly the most cookies accepted was refused"
        );
        many.push(cookie("one-too-many", "garmin.com"));
        assert!(
            session_of(&many, "garmin.com").is_err(),
            "bug: a hand-over of one cookie over the limit was accepted"
        );
    }

    #[test]
    fn a_cookie_with_no_name_or_an_oversized_value_is_refused() {
        let longest = "x".repeat(MAX_COOKIE_VALUE_BYTES);
        let kept = session_of(
            &[cookie_valued("SESSIONID", "garmin.com", &longest)],
            "garmin.com",
        );
        assert_eq!(
            kept.map(|s| s.cookies[0].value.len()).ok(),
            Some(MAX_COOKIE_VALUE_BYTES),
            "bug: a value of exactly the longest accepted length was refused"
        );
        let over = format!("{longest}x");
        assert!(
            session_of(
                &[cookie_valued("SESSIONID", "garmin.com", &over)],
                "garmin.com"
            )
            .is_err(),
            "bug: a value one byte over the limit was accepted"
        );
        assert!(
            session_of(&[cookie("", "garmin.com")], "garmin.com").is_err(),
            "bug: a cookie with no name was accepted"
        );
    }

    #[test]
    fn only_the_four_providers_are_named() {
        for (name, backend) in [
            ("garmin", "sciotte_garmin"),
            ("strava", "sciotte"),
            ("trainingpeaks", "sciotte_trainingpeaks"),
            ("coros", "sciotte_coros"),
        ] {
            assert_eq!(
                target_named(name).unwrap().provider_name(),
                backend,
                "bug: {name} names the wrong provider"
            );
        }
        assert!(target_named("whoop").is_err(), "nothing defaults to Strava");
    }

    #[test]
    fn a_cookies_debug_hides_its_value() {
        let printed = format!(
            "{:?}",
            cookie_valued("SESSIONID", "connect.garmin.com", "live-session-credential")
        );
        assert!(
            !printed.contains("live-session-credential"),
            "bug: a cookie's Debug printed its value: {printed}"
        );
        assert!(
            printed.contains("[REDACTED]"),
            "bug: a cookie's Debug does not mark its value redacted: {printed}"
        );
    }
}
