// ABOUTME: Calls the photograveur press service to turn a resolved Scene into a PNG
// ABOUTME: HTTP rather than a library call so resvg and its fonts stay out of this binary

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The press client.
//!
//! Geometry runs in-process — `photograveur` is pure math and links here — but
//! rasterising needs resvg and a font stack, which would put this binary over
//! the 80 MB gate CI enforces. So the press runs in its own scale-to-zero
//! container and this crate posts a resolved Scene to it.
//!
//! Sending a Scene rather than a spec is what makes the app and a messaging
//! channel show the same chart: the service has no geometry to disagree with.

use std::env;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use dravr_tronc::iam::IdTokenSource;
use photograveur::RenderBlock;
use pierre_core::errors::{AppError, ErrorCode};
use reqwest::Client;
use serde_json::json;
use tracing::{debug, info, warn};

/// Environment variable naming the press service.
///
/// Absent means the capability is off: charts still render in the app, and the
/// messaging path simply does not offer them. That is the correct default for
/// a developer running the stack without the service.
pub const PHOTOGRAVEUR_URL_ENV: &str = "PHOTOGRAVEUR_URL";

/// Environment variable naming the audience to address identity tokens to.
///
/// Must match what the press accepts, which terraform guarantees by setting
/// both from one local. A stable custom audience rather than the service URL,
/// so neither side depends on a value Cloud Run generates at creation.
pub const PHOTOGRAVEUR_AUDIENCE_ENV: &str = "PHOTOGRAVEUR_AUDIENCE";

/// How long to wait for a press.
///
/// The service is scale-to-zero, so a cold start is the slow case. Beyond this
/// the athlete is waiting on an image rather than a reply, and the reply is
/// what matters — the caller drops the chart and sends the prose.
const PRESS_TIMEOUT: Duration = Duration::from_secs(20);

/// Whether a press URL points at this machine's own loopback.
///
/// Only a developer's own process can be there — a Cloud Run service never is,
/// from the platform's point of view — so a loopback press is the one case
/// where skipping the ID token cannot widen anything.
fn is_loopback(url: &str) -> bool {
    let host = url
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .split(['/', ':'])
        .next()
        .unwrap_or_default();
    matches!(host, "127.0.0.1" | "localhost" | "::1" | "[::1]")
}

/// Client for the photograveur press service.
#[derive(Clone)]
pub struct PhotograveurClient {
    http: Client,
    base_url: Option<String>,
    /// Mints the identity token the press requires.
    ///
    /// `Arc` because the client is cloned per request handler and the source
    /// caches a token behind a lock — cloning the source itself would give each
    /// clone its own cache and mint a token per handler instead of per hour.
    tokens: Option<Arc<IdTokenSource>>,
}

impl fmt::Debug for PhotograveurClient {
    /// Written by hand rather than derived because this holds a credential
    /// source. `IdTokenSource` caches a live identity token, and a derived
    /// `Debug` would put it in any log line that formats the client — the
    /// logging-hygiene rule this repo enforces exists for exactly that. Whether
    /// a token source is configured is the only part worth reporting.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PhotograveurClient")
            .field("http", &self.http)
            .field("base_url", &self.base_url)
            .field("tokens", &self.tokens.as_ref().map(|_| "<configured>"))
            .finish()
    }
}

/// Resolve the identity-token source for a press, or `None` when it needs none.
///
/// Both or neither, with loopback as the one exemption. A remote URL without an
/// audience would post unsigned requests the press refuses, which surfaces as
/// every chart silently becoming prose — the failure the press wiring exists to
/// end. Treating it as "not configured" makes the misconfiguration visible in
/// one log line instead of as an absence nobody notices.
fn token_source(
    base_url: Option<&String>,
    audience: Option<String>,
    local: bool,
    http: &Client,
) -> Option<Arc<IdTokenSource>> {
    match (base_url, audience) {
        _ if local => None,
        (Some(_), Some(audience)) => Some(Arc::new(IdTokenSource::new(audience, http.clone()))),
        (Some(_), None) => {
            warn!(
                "{PHOTOGRAVEUR_URL_ENV} is set but {PHOTOGRAVEUR_AUDIENCE_ENV} is not; \
                 disabling the press rather than sending requests it will refuse"
            );
            None
        }
        (None, _) => None,
    }
}

impl PhotograveurClient {
    /// Build a client from the environment.
    ///
    /// Returns a disabled client when [`PHOTOGRAVEUR_URL_ENV`] is unset, so the
    /// server runs identically with and without the service configured.
    #[must_use]
    pub fn from_env(http: Client) -> Self {
        let base_url = env::var(PHOTOGRAVEUR_URL_ENV)
            .ok()
            .map(|url| url.trim_end_matches('/').to_owned())
            .filter(|url| !url.is_empty());
        let audience = env::var(PHOTOGRAVEUR_AUDIENCE_ENV)
            .ok()
            .filter(|a| !a.is_empty());

        // A press on the caller's own loopback is a developer running one, and
        // it cannot be reached by anything else — so it needs no ID token. A
        // deployed press is never at 127.0.0.1 from the platform's side, which
        // is what makes this safe to key on rather than a "dev mode" flag
        // somebody could set in production.
        let local = base_url.as_deref().is_some_and(is_loopback);

        let tokens = token_source(base_url.as_ref(), audience, local, &http);

        if base_url.is_none() {
            debug!("photograveur not configured; messaging charts are off");
        }
        if local {
            info!(
                url = base_url.as_deref().unwrap_or_default(),
                "photograveur on loopback; calling it unauthenticated (development)"
            );
        }

        Self {
            http,
            base_url: if local || tokens.is_some() {
                base_url
            } else {
                None
            },
            tokens,
        }
    }

    /// Whether a press service is configured.
    ///
    /// The messaging fidelity negotiator consults this: offering a channel a
    /// media URL that would 404 is worse than sending prose.
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.base_url.is_some()
    }

    /// Press one resolved block into PNG bytes.
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] when the service is unconfigured, unreachable, or
    /// answers with a non-success status.
    pub async fn press(&self, block: &RenderBlock, theme: &str) -> Result<Vec<u8>, AppError> {
        let base = self.base_url.as_ref().ok_or_else(|| {
            AppError::new(
                ErrorCode::ConfigError,
                "photograveur is not configured; cannot press a chart",
            )
        })?;

        // A deployed press carries a Google-signed token naming this workload
        // and addressed to the press. Minting is cached for the token's life,
        // so this is a lock read on all but the first call each hour.
        //
        // `None` here is the loopback case and nothing else: the constructor
        // keeps `base_url` only when the press is local OR a token source
        // exists, so having a base and no tokens means a developer's press on
        // 127.0.0.1, which takes the request unauthenticated. Demanding a token
        // anyway made every local chart 500 while the startup log cheerfully
        // announced the unauthenticated mode it never actually used.
        let token = match self.tokens.as_ref() {
            Some(tokens) => Some(tokens.token().await.map_err(|e| {
                AppError::new(
                    ErrorCode::ExternalServiceError,
                    format!("could not mint an identity token for photograveur: {e}"),
                )
            })?),
            None => None,
        };

        let request = self
            .http
            .post(format!("{base}/render"))
            .timeout(PRESS_TIMEOUT)
            .json(&json!({ "block": block, "theme": theme }));
        let request = match token.as_deref() {
            Some(token) => request.bearer_auth(token),
            None => request,
        };

        let response = request.send().await.map_err(|e| {
            AppError::new(
                ErrorCode::ExternalServiceError,
                format!("photograveur unreachable: {e}"),
            )
        })?;

        let status = response.status();
        if !status.is_success() {
            // The body carries the service's own reason; it is operator-facing
            // and contains no athlete data, so it is safe to surface.
            let detail = response.text().await.unwrap_or_default();
            return Err(AppError::new(
                ErrorCode::ExternalServiceError,
                format!("photograveur returned {status}: {detail}"),
            ));
        }

        let bytes = response.bytes().await.map_err(|e| {
            AppError::new(
                ErrorCode::ExternalServiceError,
                format!("photograveur response body failed: {e}"),
            )
        })?;
        Ok(bytes.to_vec())
    }
}
