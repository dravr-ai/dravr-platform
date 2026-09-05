// ABOUTME: GCP access-token source shared by every Google API client in the platform
// ABOUTME: TokenProvider trait plus the metadata-server implementation with expiry caching
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Google Cloud access tokens for outbound Google API calls.
//!
//! On Cloud Run and Compute Engine the instance metadata server mints an
//! `OAuth2` access token for the attached service account. Every consumer that
//! calls a Google API with that identity — the GCS prompt store, the Cloud KMS
//! KEK provider, the Cloud Tasks enqueuer — obtains it through the one
//! [`TokenProvider`] trait here, so caching and error mapping live in one place
//! and a test can substitute a fixed-token stub.
//!
//! [`MetadataTokenProvider`] is the production implementation. It caches the
//! most recently minted token until [`TOKEN_REFRESH_LEEWAY`] before the
//! declared expiry, so a burst of Google API calls costs one mint per token
//! lifetime (an hour on Cloud Run) rather than one per call.

use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::Deserialize;
use tracing::{debug, warn};

use crate::errors::{AppError, AppResult, ErrorCode};
use crate::http_client::api_client;

/// Cloud Run / GCE metadata server URL for the default service account's
/// access token.
pub const METADATA_TOKEN_URL: &str =
    "http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/token";

/// Required header for any metadata server request.
const METADATA_FLAVOR_HEADER: &str = "Metadata-Flavor";
const METADATA_FLAVOR_VALUE: &str = "Google";

/// Refresh window: re-mint a token this long before its declared expiry so
/// the next Google API call doesn't race a 401.
pub const TOKEN_REFRESH_LEEWAY: Duration = Duration::from_mins(1);

/// Connection / read timeout for the metadata server. It is link-local
/// (microseconds when present); 10s catches a hung VPC route without
/// freezing the caller.
const HTTP_TIMEOUT: Duration = Duration::from_secs(10);

/// Pluggable access-token source for Google APIs. Production uses
/// [`MetadataTokenProvider`]; tests use a fixed-token stub.
#[async_trait]
pub trait TokenProvider: Send + Sync {
    /// Return a non-expired access token suitable for `Authorization:
    /// Bearer ...` against a Google API.
    ///
    /// # Errors
    ///
    /// Returns an [`AppError`] when the underlying token mint fails:
    /// `ExternalServiceUnavailable` when the metadata server cannot be
    /// reached, `ExternalAuthFailed` when it answers with a non-200 status,
    /// `ExternalServiceError` when its response cannot be decoded.
    async fn access_token(&self) -> AppResult<String>;
}

/// Metadata-server-backed token provider. Caches the most recently minted
/// token and reuses it until [`TOKEN_REFRESH_LEEWAY`] before expiry.
pub struct MetadataTokenProvider {
    token_url: String,
    cached: Mutex<Option<CachedToken>>,
}

#[derive(Clone)]
struct CachedToken {
    value: String,
    expires_at: Instant,
}

#[derive(Deserialize)]
struct MetadataTokenResponse {
    access_token: String,
    expires_in: u64,
}

impl Default for MetadataTokenProvider {
    /// Provider pointed at the real metadata server ([`METADATA_TOKEN_URL`]).
    fn default() -> Self {
        Self::with_token_url(METADATA_TOKEN_URL)
    }
}

impl MetadataTokenProvider {
    /// Provider that mints from `token_url` instead of the metadata server.
    /// Test seam: an integration test serves the token document from a local
    /// listener to exercise caching and error mapping without GCP.
    #[must_use]
    pub fn with_token_url(token_url: impl Into<String>) -> Self {
        Self {
            token_url: token_url.into(),
            cached: Mutex::new(None),
        }
    }

    /// The cached token when one is present and not yet due for refresh.
    /// The lock is never held across an await: the guard lives only inside
    /// this call.
    fn cached_token(&self) -> Option<String> {
        let cache = self.cached.lock().unwrap_or_else(PoisonError::into_inner);
        cache
            .as_ref()
            .filter(|cached| cached.expires_at > Instant::now())
            .map(|cached| cached.value.clone())
    }

    fn store(&self, token: &str, lifetime: Duration) {
        let mut cache = self.cached.lock().unwrap_or_else(PoisonError::into_inner);
        *cache = Some(CachedToken {
            value: token.to_owned(),
            expires_at: Instant::now() + lifetime,
        });
    }
}

#[async_trait]
impl TokenProvider for MetadataTokenProvider {
    async fn access_token(&self) -> AppResult<String> {
        if let Some(token) = self.cached_token() {
            debug!("Using cached GCP access token");
            return Ok(token);
        }

        let response = api_client()
            .get(&self.token_url)
            .header(METADATA_FLAVOR_HEADER, METADATA_FLAVOR_VALUE)
            .timeout(HTTP_TIMEOUT)
            .send()
            .await
            .map_err(|e| {
                AppError::new(
                    ErrorCode::ExternalServiceUnavailable,
                    format!("GCP metadata server unreachable: {e}"),
                )
            })?;

        let status = response.status().as_u16();
        if status != 200 {
            let body = response.text().await.unwrap_or_default();
            warn!(status, body = %body, "GCP metadata token mint failed");
            return Err(AppError::new(
                ErrorCode::ExternalAuthFailed,
                format!("metadata token mint returned HTTP {status}: {body}"),
            ));
        }

        let parsed: MetadataTokenResponse = response.json().await.map_err(|e| {
            AppError::new(
                ErrorCode::ExternalServiceError,
                format!("metadata token JSON parse failed: {e}"),
            )
        })?;

        let lifetime = Duration::from_secs(parsed.expires_in)
            .checked_sub(TOKEN_REFRESH_LEEWAY)
            .unwrap_or_else(|| Duration::from_secs(parsed.expires_in.max(1)));
        self.store(&parsed.access_token, lifetime);

        Ok(parsed.access_token)
    }
}
