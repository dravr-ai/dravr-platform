// ABOUTME: Hands the Android app's Play Integrity token to Google's decodeIntegrityToken and returns the verdict
// ABOUTME: A token Google refuses is Rejected; Google out of reach (mint, network, 401/403/429/5xx) is Unavailable
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Decoding a Play Integrity token (carnet#810).
//!
//! The Android app requests its token through the standard API, whose tokens
//! are encrypted for Google alone: no key this server could hold opens them.
//! The server posts the token to
//! `POST https://playintegrity.googleapis.com/v1/{packageName}:decodeIntegrityToken`
//! with body `{"integrityToken": "…"}`, authenticated with an access token for
//! the `https://www.googleapis.com/auth/playintegrity` scope, and reads the
//! verdict from the response's `tokenPayloadExternal`. The Cloud project the
//! runtime service account belongs to must be linked to the app in the Play
//! Console, with the Play Integrity API enabled on it.
//!
//! The decoder distinguishes the two ways a decode fails, because the
//! sign-in treats them oppositely: Google answering 400 means the token is
//! not one it minted for this app ([`DecodeError::Rejected`], a refusal),
//! while Google out of reach says nothing about the app
//! ([`DecodeError::Unavailable`], treated as absent evidence). Neither the
//! token nor Google's error body is ever logged.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use pierre_core::gcp_token::{MetadataTokenProvider, TokenProvider};
use pierre_core::http_client::api_client;
use reqwest::StatusCode;
use serde::Deserialize;
use serde_json::json;
use tracing::warn;

use crate::oauth2_server::play_integrity::TokenPayload;

/// The Play Integrity API.
const PLAY_INTEGRITY_API_URL: &str = "https://playintegrity.googleapis.com";

/// The OAuth 2.0 scope `decodeIntegrityToken` requires; the service
/// account's default `cloud-platform` scope does not cover it.
pub const PLAY_INTEGRITY_SCOPE: &str = "https://www.googleapis.com/auth/playintegrity";

/// Per-request timeout for the decode: it sits on the sign-in's path.
const HTTP_TIMEOUT: Duration = Duration::from_secs(10);

/// Why a token could not be decoded into a verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    /// Google refused the token: not one it minted, or not for this app
    Rejected,
    /// Google could not be asked: no access token, no answer, or an answer
    /// that is about this server rather than the token
    Unavailable,
}

/// Turns an integrity token into the verdict it carries.
#[async_trait]
pub trait PlayIntegrityDecoder: Send + Sync {
    /// Decode `token`, as the app's `requestIntegrityCheckAsync` returned it.
    ///
    /// # Errors
    /// [`DecodeError::Rejected`] when Google refuses the token,
    /// [`DecodeError::Unavailable`] when it cannot be asked.
    async fn decode(&self, token: &str) -> Result<TokenPayload, DecodeError>;
}

/// The production decoder: Google's `decodeIntegrityToken`, as the runtime
/// service account.
pub struct GooglePlayIntegrityDecoder {
    /// The API's base URL, without a trailing slash
    api_url: String,
    /// The app whose tokens this decoder decodes
    package_name: String,
    /// Mints the service account's `playintegrity` access token; an `Arc`
    /// because the trait object is handed in by the caller, which may keep
    /// its own handle (a test reads what its provider was asked)
    token_provider: Arc<dyn TokenProvider>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DecodeResponse {
    token_payload_external: TokenPayload,
}

impl GooglePlayIntegrityDecoder {
    /// A decoder for `package_name`'s tokens against Google's API,
    /// authenticated by `token_provider`.
    #[must_use]
    pub fn new(package_name: &str, token_provider: Arc<dyn TokenProvider>) -> Self {
        Self::with_api_url(PLAY_INTEGRITY_API_URL, package_name, token_provider)
    }

    /// A decoder authenticated as the Cloud Run service account through the
    /// metadata server, with a token for [`PLAY_INTEGRITY_SCOPE`].
    #[must_use]
    pub fn from_metadata_server(package_name: &str) -> Self {
        Self::new(
            package_name,
            Arc::new(MetadataTokenProvider::default().with_scopes(&[PLAY_INTEGRITY_SCOPE])),
        )
    }

    /// A decoder against `api_url` instead of Google's. Test seam: an
    /// integration test serves the API from a local listener.
    #[must_use]
    pub fn with_api_url(
        api_url: &str,
        package_name: &str,
        token_provider: Arc<dyn TokenProvider>,
    ) -> Self {
        Self {
            api_url: api_url.trim_end_matches('/').to_owned(),
            package_name: package_name.to_owned(),
            token_provider,
        }
    }
}

#[async_trait]
impl PlayIntegrityDecoder for GooglePlayIntegrityDecoder {
    async fn decode(&self, token: &str) -> Result<TokenPayload, DecodeError> {
        let access_token = self.token_provider.access_token().await.map_err(|e| {
            warn!(error_code = ?e.code, "Play Integrity access token could not be minted");
            DecodeError::Unavailable
        })?;
        let url = format!(
            "{}/v1/{}:decodeIntegrityToken",
            self.api_url, self.package_name
        );
        let response = api_client()
            .post(&url)
            .bearer_auth(access_token)
            .timeout(HTTP_TIMEOUT)
            .json(&json!({ "integrityToken": token }))
            .send()
            .await
            .map_err(|e| {
                warn!(error = %e, "Play Integrity API unreachable");
                DecodeError::Unavailable
            })?;
        let status = response.status();
        if status == StatusCode::BAD_REQUEST {
            warn!(
                status = status.as_u16(),
                "Play Integrity API refused the token"
            );
            return Err(DecodeError::Rejected);
        }
        if !status.is_success() {
            warn!(
                status = status.as_u16(),
                "Play Integrity API did not decode the token"
            );
            return Err(DecodeError::Unavailable);
        }
        // The parse error is not logged: serde quotes the values it chokes on.
        let decoded: DecodeResponse = response.json().await.map_err(|_| {
            warn!("Play Integrity API answered an unreadable verdict");
            DecodeError::Unavailable
        })?;
        Ok(decoded.token_payload_external)
    }
}
