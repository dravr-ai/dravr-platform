// ABOUTME: Best-effort bridge notification after a successful OAuth connection
// ABOUTME: Posts the fresh token to the local bridge callback for client-side storage/focus recovery

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The post-connect bridge ping, split out of `oauth_flow` so the flow
//! orchestrator stays within its size budget. Strictly best-effort: a bridge
//! that is not running must never fail the OAuth flow that just succeeded.

use std::time::Duration as StdDuration;

use pierre_auth::oauth2_client::OAuth2Token;
use pierre_config::environment::ServerConfig;
use pierre_core::http_client::{api_client, SharedHttpError};
use serde_json::{json, Value as JsonValue};
use tracing::{debug, info, warn};

/// Notify the bridge about a successful OAuth connection (for client-side
/// token storage and focus recovery).
pub async fn notify_bridge_oauth_success(
    config: &ServerConfig,
    provider: &str,
    token: &OAuth2Token,
) {
    let oauth_callback_port = config.oauth_callback_port;
    let callback_url =
        format!("http://localhost:{oauth_callback_port}/oauth/provider-callback/{provider}");

    let token_data = build_bridge_token_data(token);

    debug!(
        "Notifying bridge about {} OAuth success at {}",
        provider, callback_url
    );

    // Best-effort notification with configured timeout - don't fail OAuth flow if bridge notification fails
    // Timeout is sourced from ServerConfig.http_client (loaded at startup via HttpClientConfig::from_env)
    let timeout_secs = config.http_client.oauth_callback_notification_timeout_secs;
    let result = api_client()
        .post(&callback_url)
        .json(&token_data)
        .timeout(StdDuration::from_secs(timeout_secs))
        .send()
        .await;

    log_bridge_notification_result(result, provider);
}

/// Build OAuth token data for bridge notification
fn build_bridge_token_data(token: &OAuth2Token) -> JsonValue {
    // Calculate expires_in from expires_at if available
    let expires_in = token.expires_at.map(|expires_at| {
        let duration = expires_at - chrono::Utc::now();
        duration.num_seconds().max(0)
    });

    json!({
        "access_token": token.access_token,
        "refresh_token": token.refresh_token,
        "expires_in": expires_in,
        "token_type": token.token_type,
        "scope": token.scope
    })
}

/// Log bridge notification response
fn log_bridge_notification_result(
    result: Result<reqwest::Response, SharedHttpError>,
    provider: &str,
) {
    match result {
        Ok(response) if response.status().is_success() => {
            info!(
                "Successfully notified bridge about {} OAuth completion",
                provider
            );
        }
        Ok(response) => {
            warn!(
                "Bridge notification responded with status {} for provider {}",
                response.status(),
                provider
            );
        }
        Err(e) => {
            warn!(
                "Failed to notify bridge about {} OAuth (bridge may not be running): {}",
                provider, e
            );
        }
    }
}
