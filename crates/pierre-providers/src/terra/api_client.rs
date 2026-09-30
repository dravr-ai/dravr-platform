// ABOUTME: Terra REST API client for authentication, user management, and historical data
// ABOUTME: Handles API key auth, widget sessions, and on-demand data requests
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Terra REST API client
//!
//! This module provides a client for interacting with Terra's REST API endpoints.
//! While Terra primarily uses webhooks for data delivery, the REST API is used for:
//! - Generating authentication widget sessions
//! - Requesting historical data
//! - Managing user connections
//! - Deauthenticating users

use crate::errors::provider::ProviderError;
use crate::http_client::{shared_client, SharedHttpClient};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Terra API configuration
#[derive(Debug, Clone)]
pub struct TerraApiConfig {
    /// Terra API key (from dashboard)
    pub api_key: String,
    /// Terra dev ID (from dashboard)
    pub dev_id: String,
    /// Base URL for Terra API
    pub base_url: String,
    /// Request timeout
    pub timeout: Duration,
}

impl Default for TerraApiConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            dev_id: String::new(),
            base_url: "https://api.tryterra.co/v2".to_owned(),
            timeout: Duration::from_secs(30),
        }
    }
}

/// Terra API client for REST operations
pub struct TerraApiClient {
    config: TerraApiConfig,
    client: &'static SharedHttpClient,
}

/// Response from user info request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserInfoResponse {
    /// Status
    pub status: String,
    /// User info
    pub user: Option<TerraUserInfo>,
}

/// Terra user info
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerraUserInfo {
    /// Terra user ID
    pub user_id: String,
    /// Provider name
    pub provider: String,
    /// Last webhook update
    pub last_webhook_update: Option<String>,
    /// Reference ID
    pub reference_id: Option<String>,
    /// Scopes granted
    pub scopes: Option<String>,
}

impl TerraApiClient {
    /// Create a new Terra API client using the shared HTTP client
    #[must_use]
    pub fn new(config: TerraApiConfig) -> Self {
        Self {
            config,
            client: shared_client(),
        }
    }

    /// Get user info from Terra
    ///
    /// # Errors
    ///
    /// Returns an error if the API request fails
    pub async fn get_user_info(&self, user_id: &str) -> Result<UserInfoResponse, ProviderError> {
        let url = format!("{}/userInfo", self.config.base_url);

        let response = self
            .client
            .get(&url)
            .header("x-api-key", &self.config.api_key)
            .header("dev-id", &self.config.dev_id)
            .query(&[("user_id", user_id)])
            .send()
            .await
            .map_err(|e| ProviderError::NetworkError(e.to_string()))?;

        let status = response.status();
        let text = response.text().await.unwrap_or_default();

        if !status.is_success() {
            return Err(ProviderError::ApiError {
                provider: "terra".to_owned(),
                status_code: status.as_u16(),
                message: text,
                retryable: status.is_server_error(),
            });
        }

        serde_json::from_str(&text).map_err(|e| ProviderError::ParseError {
            provider: "terra".to_owned(),
            field: "user_info_response",
            source: e,
        })
    }
}
