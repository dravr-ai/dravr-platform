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
//!
//! The transport is tronc's [`ServiceClient`]: it mints the Google ID token
//! addressed to the press (none on loopback), refuses a press that is neither
//! `https` nor on loopback, and sends every call under its own request id.

use std::fmt;
use std::time::Duration;

use dravr_tronc::service_client::{ServiceClient, ServiceError};
use photograveur::RenderBlock;
use pierre_core::errors::{AppError, ErrorCode};
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

/// The service name errors and logs use.
const SERVICE: &str = "photograveur";

/// How long to wait for a press.
///
/// The service is scale-to-zero, so a cold start is the slow case. Beyond this
/// the athlete is waiting on an image rather than a reply, and the reply is
/// what matters — the caller drops the chart and sends the prose.
const PRESS_TIMEOUT: Duration = Duration::from_secs(20);

/// Client for the photograveur press service.
///
/// Cheap to clone: [`ServiceClient`] clones share one connection pool and one
/// identity-token cache, so the token is minted per hour, not per handler.
#[derive(Clone)]
pub struct PhotograveurClient {
    /// `None` when the press is not configured, or configured so that no
    /// request to it could be accepted.
    client: Option<ServiceClient>,
}

impl fmt::Debug for PhotograveurClient {
    /// Written by hand because the client holds a credential source: once
    /// minted, a live identity token sits in its cache, and a log line that
    /// formats the client must not carry it. The press URL and whether a
    /// client is configured are the only parts worth reporting.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PhotograveurClient")
            .field(
                "base_url",
                &self.client.as_ref().map(ServiceClient::base_url),
            )
            .field("client", &self.client.as_ref().map(|_| "<configured>"))
            .finish()
    }
}

impl PhotograveurClient {
    /// Build a client from the environment.
    ///
    /// Returns a disabled client when [`PHOTOGRAVEUR_URL_ENV`] is unset, so the
    /// server runs identically with and without the service configured.
    ///
    /// A press URL that is not on loopback needs [`PHOTOGRAVEUR_AUDIENCE_ENV`]
    /// and `https`. Missing either, the client is disabled with one warning
    /// rather than enabled: a press that refuses every request would present
    /// as every chart silently becoming prose, the failure the press wiring
    /// exists to end.
    #[must_use]
    pub fn from_env() -> Self {
        let client = match ServiceClient::from_env(
            SERVICE,
            PHOTOGRAVEUR_URL_ENV,
            PHOTOGRAVEUR_AUDIENCE_ENV,
            PRESS_TIMEOUT,
        ) {
            Ok(Some(client)) => {
                info!(url = client.base_url(), "photograveur press configured");
                Some(client)
            }
            Ok(None) => {
                debug!("photograveur not configured; messaging charts are off");
                None
            }
            Err(error) => {
                warn!(
                    error = %error,
                    "photograveur is misconfigured; disabling the press rather than sending \
                     requests it will refuse"
                );
                None
            }
        };
        Self { client }
    }

    /// Whether a press service is configured.
    ///
    /// The messaging fidelity negotiator consults this: offering a channel a
    /// media URL that would 404 is worse than sending prose.
    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        self.client.is_some()
    }

    /// Press one resolved block into PNG bytes.
    ///
    /// `locale` reaches the press as a BCP-47 tag for the text it originates
    /// itself. A chart's labels are already localised when its Scene is
    /// resolved here; a route's legend is not, because nothing on a route
    /// names the track, so the press chooses the words and needs to know
    /// which language.
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] when the service is unconfigured, unreachable, or
    /// answers with a non-success status.
    pub async fn press(
        &self,
        block: &RenderBlock,
        theme: &str,
        locale: &str,
    ) -> Result<Vec<u8>, AppError> {
        let client = self.client.as_ref().ok_or_else(|| {
            AppError::new(
                ErrorCode::ConfigError,
                "photograveur is not configured; cannot press a chart",
            )
        })?;

        let body = json!({ "block": block, "theme": theme, "locale": locale });
        let response = client
            .post_json("press", "/render", &body)
            .send()
            .await
            .map_err(|error| press_error(&error))?;

        let status = response.status();
        if !status.is_success() {
            // The body carries the service's own reason; it is operator-facing
            // and contains no athlete data, so it is safe to surface.
            let detail = String::from_utf8_lossy(response.bytes());
            return Err(AppError::new(
                ErrorCode::ExternalServiceError,
                format!("photograveur returned {status}: {detail}"),
            ));
        }
        Ok(response.bytes().to_vec())
    }
}

/// The [`AppError`] a press that produced no usable answer reports.
///
/// Matched without a wildcard so a variant tronc adds fails this build until
/// it is given its wording.
fn press_error(error: &ServiceError) -> AppError {
    let message = match error {
        ServiceError::Identity { .. } => {
            format!("could not mint an identity token for photograveur: {error}")
        }
        ServiceError::Transport { .. } => format!("photograveur unreachable: {error}"),
        ServiceError::Shed { .. }
        | ServiceError::Unfinished { .. }
        | ServiceError::Body { .. }
        | ServiceError::Decode { .. } => format!("photograveur press failed: {error}"),
    };
    AppError::new(ErrorCode::ExternalServiceError, message)
}
