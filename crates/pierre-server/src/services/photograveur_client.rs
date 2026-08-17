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
use std::time::Duration;

use photograveur::RenderBlock;
use pierre_core::errors::{AppError, ErrorCode};
use reqwest::Client;
use serde_json::json;
use tracing::debug;

/// Environment variable naming the press service.
///
/// Absent means the capability is off: charts still render in the app, and the
/// messaging path simply does not offer them. That is the correct default for
/// a developer running the stack without the service.
pub const PHOTOGRAVEUR_URL_ENV: &str = "PHOTOGRAVEUR_URL";

/// How long to wait for a press.
///
/// The service is scale-to-zero, so a cold start is the slow case. Beyond this
/// the athlete is waiting on an image rather than a reply, and the reply is
/// what matters — the caller drops the chart and sends the prose.
const PRESS_TIMEOUT: Duration = Duration::from_secs(20);

/// Client for the photograveur press service.
#[derive(Debug, Clone)]
pub struct PhotograveurClient {
    http: Client,
    base_url: Option<String>,
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
        if base_url.is_none() {
            debug!("photograveur not configured; messaging charts are off");
        }
        Self { http, base_url }
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

        let response = self
            .http
            .post(format!("{base}/render"))
            .timeout(PRESS_TIMEOUT)
            .json(&json!({ "block": block, "theme": theme }))
            .send()
            .await
            .map_err(|e| {
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
