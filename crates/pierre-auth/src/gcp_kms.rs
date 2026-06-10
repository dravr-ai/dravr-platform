// ABOUTME: GCP Cloud KMS KekProvider — wraps/unwraps the DEK via the KMS symmetric encrypt/decrypt API
// ABOUTME: Key material never leaves KMS; auth uses the Cloud Run metadata-server access token (ADR-017 Phase 5)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! GCP Cloud KMS-backed [`KekProvider`].
//!
//! Enabled with the `gcp-kms` feature and selected at bootstrap when
//! `PIERRE_KMS_KEY_RESOURCE` is set. The Data Encryption Key (DEK) is wrapped by
//! a Cloud KMS symmetric key (the KEK); the KEK itself never leaves KMS. Because
//! KMS retains prior key versions, KEK rotation is a KMS-side configuration and
//! existing wrapped DEKs keep decrypting (see ADR-017 Phase 6).

use std::env;

use async_trait::async_trait;
use base64::engine::general_purpose::STANDARD as Base64Standard;
use base64::Engine as _;
use pierre_core::errors::{AppError, AppResult};
use serde::Deserialize;

use crate::key_management::KekProvider;

/// Cloud Run / GCE metadata server endpoint for the service-account access token.
const METADATA_TOKEN_URL: &str =
    "http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/token";
/// Cloud KMS REST base URL.
const KMS_BASE_URL: &str = "https://cloudkms.googleapis.com/v1";

/// KEK provider backed by GCP Cloud KMS.
pub struct GcpKmsKekProvider {
    /// Full KMS crypto-key resource:
    /// `projects/{p}/locations/{l}/keyRings/{r}/cryptoKeys/{k}`.
    key_resource: String,
    http: reqwest::Client,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
}

#[derive(Deserialize)]
struct EncryptResponse {
    ciphertext: String,
}

#[derive(Deserialize)]
struct DecryptResponse {
    plaintext: String,
}

impl GcpKmsKekProvider {
    /// Build a provider for the given KMS crypto-key resource name.
    #[must_use]
    pub fn new(key_resource: impl Into<String>) -> Self {
        Self {
            key_resource: key_resource.into(),
            http: reqwest::Client::new(),
        }
    }

    /// Build from the `PIERRE_KMS_KEY_RESOURCE` environment variable.
    ///
    /// # Errors
    ///
    /// Returns an error if the variable is not set.
    pub fn from_env() -> AppResult<Self> {
        let key_resource = env::var("PIERRE_KMS_KEY_RESOURCE").map_err(|_| {
            AppError::config(
                "PIERRE_KMS_KEY_RESOURCE is required for the GCP KMS KEK provider \
                 (projects/.../locations/.../keyRings/.../cryptoKeys/...)",
            )
        })?;
        Ok(Self::new(key_resource))
    }

    /// Fetch a short-lived access token from the instance metadata server.
    async fn access_token(&self) -> AppResult<String> {
        let response = self
            .http
            .get(METADATA_TOKEN_URL)
            .header("Metadata-Flavor", "Google")
            .send()
            .await
            .map_err(|e| AppError::internal(format!("KMS metadata token request failed: {e}")))?;
        if !response.status().is_success() {
            return Err(AppError::internal(format!(
                "KMS metadata token request returned status {}",
                response.status()
            )));
        }
        let token: TokenResponse = response
            .json()
            .await
            .map_err(|e| AppError::internal(format!("Invalid metadata token response: {e}")))?;
        Ok(token.access_token)
    }
}

#[async_trait]
impl KekProvider for GcpKmsKekProvider {
    async fn wrap(&self, plaintext: &[u8]) -> AppResult<Vec<u8>> {
        let token = self.access_token().await?;
        let url = format!("{KMS_BASE_URL}/{}:encrypt", self.key_resource);
        let body = serde_json::json!({ "plaintext": Base64Standard.encode(plaintext) });
        let response = self
            .http
            .post(&url)
            .bearer_auth(token)
            .json(&body)
            .send()
            .await
            .map_err(|e| AppError::internal(format!("KMS encrypt request failed: {e}")))?;
        if !response.status().is_success() {
            return Err(AppError::internal(format!(
                "KMS encrypt returned status {}",
                response.status()
            )));
        }
        let parsed: EncryptResponse = response
            .json()
            .await
            .map_err(|e| AppError::internal(format!("Invalid KMS encrypt response: {e}")))?;
        Base64Standard
            .decode(parsed.ciphertext)
            .map_err(|e| AppError::internal(format!("Invalid KMS ciphertext base64: {e}")))
    }

    async fn unwrap(&self, ciphertext: &[u8]) -> AppResult<Vec<u8>> {
        let token = self.access_token().await?;
        let url = format!("{KMS_BASE_URL}/{}:decrypt", self.key_resource);
        let body = serde_json::json!({ "ciphertext": Base64Standard.encode(ciphertext) });
        let response = self
            .http
            .post(&url)
            .bearer_auth(token)
            .json(&body)
            .send()
            .await
            .map_err(|e| AppError::internal(format!("KMS decrypt request failed: {e}")))?;
        if !response.status().is_success() {
            return Err(AppError::internal(format!(
                "KMS decrypt returned status {}",
                response.status()
            )));
        }
        let parsed: DecryptResponse = response
            .json()
            .await
            .map_err(|e| AppError::internal(format!("Invalid KMS decrypt response: {e}")))?;
        Base64Standard
            .decode(parsed.plaintext)
            .map_err(|e| AppError::internal(format!("Invalid KMS plaintext base64: {e}")))
    }
}
