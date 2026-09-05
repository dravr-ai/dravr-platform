// ABOUTME: PromptStore impl backed by Google Cloud Storage
// ABOUTME: Auth via the platform-wide GCP TokenProvider (Cloud Run metadata server)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! GCS-backed [`PromptStore`].
//!
//! ## Auth
//!
//! At runtime the store needs an `OAuth2` access token scoped for
//! `https://www.googleapis.com/auth/devstorage.read_only`. It comes from
//! [`pierre_core::gcp_token::TokenProvider`] — in production the Cloud Run
//! metadata server ([`MetadataTokenProvider`]), whose service account is
//! bound to `roles/storage.objectViewer` on the bucket via Terraform.
//!
//! The provider caches each minted token until 60s before its declared
//! expiry, so a hot-reload burst (e.g. webhook arriving after a
//! contremaitre push) consumes one token-mint per hour at most.
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
use std::time::Duration;

use async_trait::async_trait;

use pierre_core::gcp_token::{MetadataTokenProvider, TokenProvider};
use pierre_core::http_client::api_client;

use super::super::errors::ContremaitreError;
use super::super::manifest::{parse_manifest, Manifest};
use super::{PromptStore, StoredFile};

/// GCS JSON API base. Object download uses `?alt=media` to stream raw
/// bytes; without it the API returns Object metadata JSON instead.
const GCS_API_BASE: &str = "https://storage.googleapis.com/storage/v1";

/// Connection / read timeout for GCS calls. A read is a single HTTPS
/// round-trip; 10s catches a hung VPC route without freezing startup.
const HTTP_TIMEOUT: Duration = Duration::from_secs(10);

/// Read [`PromptStore`] backed by a GCS bucket.
pub struct GcsPromptStore {
    bucket: String,
    token_provider: Arc<dyn TokenProvider>,
}

impl GcsPromptStore {
    /// Build a GCS-backed store for the given bucket name, minting tokens
    /// from the Cloud Run metadata server.
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
        let token = self
            .token_provider
            .access_token()
            .await
            .map_err(ContremaitreError::TokenProvider)?;
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
