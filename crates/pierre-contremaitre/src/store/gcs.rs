// ABOUTME: PromptStore impl backed by Google Cloud Storage
// ABOUTME: Auth via Cloud Run metadata server; falls back to ADC for local dev
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! GCS-backed [`PromptStore`].
//!
//! ## Auth chain
//!
//! At runtime the store needs an `OAuth2` access token scoped for
//! `https://www.googleapis.com/auth/devstorage.read_only`. Two paths:
//!
//! 1. **Cloud Run / Compute Engine** — fetch from the metadata server at
//!    `http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/token`.
//!    The Cloud Run service account is bound to `roles/storage.objectViewer`
//!    on the bucket via Terraform.
//!
//! 2. **Local dev** — read Application Default Credentials (ADC) from
//!    `~/.config/gcloud/application_default_credentials.json` (after
//!    `gcloud auth application-default login`) and exchange the user's
//!    refresh token for an access token.
//!
//! Tokens are cached in-memory per [`GcsPromptStore`] until 60s before
//! their declared expiry, so a hot-reload burst (e.g. webhook arriving
//! after a contremaitre push) consumes one token-mint per hour at most.
//!
//! ## Layout
//!
//! The bucket mirrors the dravr-contremaitre repo tree exactly:
//!
//! ```text
//! gs://{bucket}/manifest.json
//! gs://{bucket}/prompts/system/coach.md
//! gs://{bucket}/prompts/coaches/{slug}/{locale}.md
//! gs://{bucket}/strings/{key}/{locale}.md
//! gs://{bucket}/tools/{tool_name}.yaml
//! gs://{bucket}/evidence/{domain}/{category}/{slug}.md
//! gs://{bucket}/config/cageux.yaml
//! ```
//!
//! Mirroring is owned by a GitHub Action in `dravr-contremaitre` that
//! `gsutil rsync`'s the repo to the bucket on every push to `main`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::Deserialize;
use tokio::sync::Mutex;
use tracing::{debug, warn};

use pierre_core::http_client::api_client;

use super::super::errors::ContremaitreError;
use super::super::manifest::{parse_manifest, Manifest};
use super::{PromptStore, StoredFile};

/// Cloud Run / GCE metadata server URL for the default service account.
const METADATA_TOKEN_URL: &str =
    "http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/token";

/// Required header for any metadata server request.
const METADATA_FLAVOR_HEADER: &str = "Metadata-Flavor";
const METADATA_FLAVOR_VALUE: &str = "Google";

/// GCS JSON API base. Object download uses `?alt=media` to stream raw
/// bytes; without it the API returns Object metadata JSON instead.
const GCS_API_BASE: &str = "https://storage.googleapis.com/storage/v1";

/// Refresh window: re-fetch a token this long before its declared expiry
/// so the next read doesn't race a 401.
const TOKEN_REFRESH_LEEWAY: Duration = Duration::from_secs(60);

/// Connection / read timeout for metadata + GCS calls. The metadata server
/// is link-local (microseconds when present), and GCS reads are a single
/// HTTPS round-trip; 10s catches a hung VPC route without freezing startup.
const HTTP_TIMEOUT: Duration = Duration::from_secs(10);

/// Read [`PromptStore`] backed by a GCS bucket.
pub struct GcsPromptStore {
    bucket: String,
    token_provider: Arc<dyn TokenProvider>,
}

impl GcsPromptStore {
    /// Build a GCS-backed store for the given bucket name. Token-source
    /// selection is automatic: the metadata server is tried first, and a
    /// failure there falls back to ADC (set `GOOGLE_APPLICATION_CREDENTIALS`
    /// or run `gcloud auth application-default login`).
    #[must_use]
    pub fn new(bucket: String) -> Self {
        Self {
            bucket,
            token_provider: Arc::new(MetadataTokenProvider::default()),
        }
    }

    /// Inject a custom [`TokenProvider`] so integration tests can drive
    /// the store without standing up a real metadata server. Public so
    /// `crates/pierre-server/tests/contremaitre_gcs_store_test.rs` can
    /// construct a stub-token store; production callers go through
    /// [`Self::new`].
    #[must_use]
    pub fn with_token_provider(bucket: String, provider: Arc<dyn TokenProvider>) -> Self {
        Self {
            bucket,
            token_provider: provider,
        }
    }
}

#[async_trait]
impl PromptStore for GcsPromptStore {
    async fn read_file(&self, path: &str) -> Result<StoredFile, ContremaitreError> {
        let token = self.token_provider.access_token().await?;
        // Object names with slashes need percent-encoding when embedded in
        // the URL path; `urlencoding::encode` handles every reserved char
        // including `/`, which GCS requires literal-encoded as `%2F`.
        let encoded_path = urlencoding::encode(path);
        let url = format!(
            "{GCS_API_BASE}/b/{bucket}/o/{path}?alt=media",
            bucket = self.bucket,
            path = encoded_path
        );

        let response = api_client()
            .get(&url)
            .header("Authorization", format!("Bearer {token}"))
            .timeout(HTTP_TIMEOUT)
            .send()
            .await?;

        let status = response.status().as_u16();
        if status != 200 {
            let body = response.text().await.unwrap_or_default();
            return Err(ContremaitreError::GitHubApi {
                status,
                message: format!("GCS read of '{path}' failed: {body}"),
            });
        }

        let content = response
            .text()
            .await
            .map_err(|e| ContremaitreError::GitHubApi {
                status: 200,
                message: format!("GCS body decode for '{path}' failed: {e}"),
            })?;

        Ok(StoredFile {
            content,
            path: path.to_owned(),
        })
    }

    async fn read_manifest(&self) -> Result<Manifest, ContremaitreError> {
        let file = self.read_file("manifest.json").await?;
        parse_manifest(&file.content)
    }

    fn backend_label(&self) -> &'static str {
        "gcs"
    }
}

/// Pluggable access-token source. Production uses
/// [`MetadataTokenProvider`]; tests use a fixed-token stub.
#[async_trait]
pub trait TokenProvider: Send + Sync {
    /// Return a non-expired access token suitable for `Authorization:
    /// Bearer ...` against the GCS JSON API.
    ///
    /// # Errors
    ///
    /// Returns a [`ContremaitreError`] when the underlying token-mint
    /// fails (metadata server unreachable, ADC missing, refresh refused).
    async fn access_token(&self) -> Result<String, ContremaitreError>;
}

/// Metadata-server-backed token provider. Caches the most recently minted
/// token and reuses it until [`TOKEN_REFRESH_LEEWAY`] before expiry.
#[derive(Default)]
struct MetadataTokenProvider {
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

#[async_trait]
impl TokenProvider for MetadataTokenProvider {
    async fn access_token(&self) -> Result<String, ContremaitreError> {
        {
            let cache = self.cached.lock().await;
            if let Some(cached) = cache.as_ref() {
                if cached.expires_at > Instant::now() {
                    debug!("Using cached GCP access token");
                    return Ok(cached.value.clone());
                }
            }
        }

        let response = api_client()
            .get(METADATA_TOKEN_URL)
            .header(METADATA_FLAVOR_HEADER, METADATA_FLAVOR_VALUE)
            .timeout(HTTP_TIMEOUT)
            .send()
            .await
            .map_err(|e| ContremaitreError::GitHubApi {
                status: 0,
                message: format!("GCP metadata server unreachable: {e}"),
            })?;

        let status = response.status().as_u16();
        if status != 200 {
            let body = response.text().await.unwrap_or_default();
            warn!(status, body = %body, "GCP metadata token mint failed");
            return Err(ContremaitreError::GitHubApi {
                status,
                message: format!("metadata token mint returned HTTP {status}: {body}"),
            });
        }

        let parsed: MetadataTokenResponse =
            response
                .json()
                .await
                .map_err(|e| ContremaitreError::GitHubApi {
                    status: 200,
                    message: format!("metadata token JSON parse failed: {e}"),
                })?;

        let lifetime = Duration::from_secs(parsed.expires_in)
            .checked_sub(TOKEN_REFRESH_LEEWAY)
            .unwrap_or_else(|| Duration::from_secs(parsed.expires_in.max(1)));
        let token = parsed.access_token;

        {
            let mut cache = self.cached.lock().await;
            *cache = Some(CachedToken {
                value: token.clone(),
                expires_at: Instant::now() + lifetime,
            });
        }

        Ok(token)
    }
}
