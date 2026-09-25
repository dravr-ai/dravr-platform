// ABOUTME: Best-effort bridge notification after a successful OAuth connection
// ABOUTME: Posts the fresh token to the bridge listener that started the flow, presenting its per-flow token

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The post-connect bridge ping, split out of `oauth_flow` so the flow
//! orchestrator stays within its size budget. Strictly best-effort: a bridge
//! that is not running must never fail the OAuth flow that just succeeded.
//!
//! The SDK bridge's local listener stores whatever provider tokens it accepts,
//! and every process on the machine can reach it, so it refuses a POST that
//! does not present the per-flow token it generated. The bridge hands that
//! token over when it starts the flow; it rides the flow's state to the
//! callback and is presented here. A flow no bridge started carries none and
//! notifies nobody.

use std::time::Duration as StdDuration;

use pierre_auth::oauth2_client::OAuth2Token;
use pierre_config::environment::ServerConfig;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::http_client::{api_client, SharedHttpError};
use serde_json::{json, Value as JsonValue};
use tracing::{debug, info, warn};

/// The header carrying the bridge listener's per-flow token: on the request
/// that starts a flow, and on the success notification, the only POST the
/// listener accepts provider tokens from.
pub const BRIDGE_CALLBACK_TOKEN_HEADER: &str = "X-Callback-Token";

/// Shortest per-flow token accepted: 128 bits of a hex-encoded secret. The
/// SDK bridge sends 64 hex characters (256 bits).
const MIN_BRIDGE_CALLBACK_TOKEN_LEN: usize = 32;

/// Longest per-flow token accepted, so a flow start cannot store an
/// arbitrarily large value on the state row.
const MAX_BRIDGE_CALLBACK_TOKEN_LEN: usize = 128;

/// The per-flow token of the bridge listener that starts an OAuth flow, as the
/// flow start carries it in [`BRIDGE_CALLBACK_TOKEN_HEADER`].
///
/// Only URL- and header-safe characters are accepted, because the value is
/// stored with the flow and sent back verbatim on the success notification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeCallbackToken(String);

impl BridgeCallbackToken {
    /// Accept the token a flow start presents.
    ///
    /// # Errors
    /// Returns an invalid-input error unless `raw` is 32 to 128 characters of
    /// `A-Z`, `a-z`, `0-9`, `-` or `_`.
    pub fn parse(raw: &str) -> AppResult<Self> {
        let well_formed = (MIN_BRIDGE_CALLBACK_TOKEN_LEN..=MAX_BRIDGE_CALLBACK_TOKEN_LEN)
            .contains(&raw.len())
            && raw
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
        if well_formed {
            Ok(Self(raw.to_owned()))
        } else {
            Err(AppError::invalid_input(format!(
                "{BRIDGE_CALLBACK_TOKEN_HEADER} must be {MIN_BRIDGE_CALLBACK_TOKEN_LEN} to {MAX_BRIDGE_CALLBACK_TOKEN_LEN} characters of A-Z, a-z, 0-9, '-' or '_'"
            )))
        }
    }

    /// The token as the flow's state stores it.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Notify the bridge that started this flow about its successful OAuth
/// connection (for client-side token storage and focus recovery), presenting
/// the per-flow token its listener demands.
pub async fn notify_bridge_oauth_success(
    config: &ServerConfig,
    provider: &str,
    token: &OAuth2Token,
    callback_token: &str,
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
        .header(BRIDGE_CALLBACK_TOKEN_HEADER, callback_token)
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
