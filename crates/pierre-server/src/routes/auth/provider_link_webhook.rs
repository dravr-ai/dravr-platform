// ABOUTME: Outbound webhook emitter for channel-initiated provider connection success
// ABOUTME: Signs payloads with HMAC-SHA256 and posts to a configured Canot URL
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Outbound webhook for channel-initiated provider connections.
//!
//! When a user completes a Sciotte login via the hosted-page flow (authenticated
//! with a link-token minted for a specific channel), Pierre fires a fire-and-forget
//! webhook at a configured URL so the originating channel bot (e.g. Canot) can
//! post a confirmation back in the original thread.
//!
//! Configuration (env vars):
//! - `PROVIDER_LINK_WEBHOOK_URL` — URL to POST events to. If unset, no webhook
//!   is emitted (feature is opt-in).
//! - `PROVIDER_LINK_WEBHOOK_SECRET` — optional HMAC-SHA256 signing key. When set,
//!   the request carries an `X-Pierre-Signature: sha256=<hex>` header so the
//!   receiver can verify the payload.
//!
//! The POST is fire-and-forget in a `tokio::spawn` task; failures are logged but
//! never block the user-facing login response.

use std::env;
use std::sync::LazyLock;
use std::time::Duration;

use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use serde::Serialize;
use sha2::Sha256;
use tracing::{info, warn};
use uuid::Uuid;

/// Environment variable name for the webhook URL
const WEBHOOK_URL_ENV: &str = "PROVIDER_LINK_WEBHOOK_URL";

/// Environment variable name for the HMAC signing secret
const WEBHOOK_SECRET_ENV: &str = "PROVIDER_LINK_WEBHOOK_SECRET";

/// HTTP header carrying the HMAC signature
const SIGNATURE_HEADER: &str = "X-Pierre-Signature";

/// HTTP timeout for the outbound webhook POST (short — this is best-effort).
const WEBHOOK_TIMEOUT: Duration = Duration::from_secs(8);

type HmacSha256 = Hmac<Sha256>;

/// Shared HTTP client for outbound webhook calls (connection pool reuse).
/// Uses the default client; timeout is set per-request in [`post_event`].
static WEBHOOK_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(reqwest::Client::new);

/// Payload posted to the channel bot after a successful provider connection.
#[derive(Debug, Clone, Serialize)]
pub struct ProviderLinkedEvent {
    /// Event type discriminator
    pub event: &'static str,
    /// The Pierre user that completed the connection
    pub user_id: Uuid,
    /// Active tenant for the session
    pub tenant_id: Uuid,
    /// Provider name (e.g. "sciotte", `sciotte_garmin`)
    pub provider: String,
    /// Target platform label ("strava" or "garmin")
    pub target: String,
    /// Channel slug the link-token was minted for (e.g. "slack")
    pub channel: String,
    /// Optional channel thread/DM id for the bot to reply in
    pub channel_thread: Option<String>,
    /// Completion timestamp
    pub connected_at: DateTime<Utc>,
}

impl ProviderLinkedEvent {
    /// Event name for the `event` discriminator.
    pub const NAME: &'static str = "provider.linked";

    /// Build a new event with `connected_at` = now.
    #[must_use]
    pub fn new(
        user_id: Uuid,
        tenant_id: Uuid,
        provider: String,
        target: String,
        channel: String,
        channel_thread: Option<String>,
    ) -> Self {
        Self {
            event: Self::NAME,
            user_id,
            tenant_id,
            provider,
            target,
            channel,
            channel_thread,
            connected_at: Utc::now(),
        }
    }
}

/// Compute an HMAC-SHA256 signature over the payload bytes, returning the
/// canonical header value `sha256=<hex>`.
///
/// # Errors
///
/// Returns an error if the HMAC cannot be initialized from the secret (e.g.
/// empty secret).
pub fn sign_payload(secret: &str, payload: &[u8]) -> Result<String, String> {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .map_err(|e| format!("Invalid HMAC key: {e}"))?;
    mac.update(payload);
    let digest = mac.finalize().into_bytes();
    Ok(format!("sha256={}", hex::encode(digest)))
}

/// Fire-and-forget: spawn a background task that POSTs the event to the
/// configured webhook URL. Silently returns if no URL is configured.
/// All failures are logged at WARN level — they never block the caller.
pub fn spawn_emit(event: ProviderLinkedEvent) {
    let Ok(url) = env::var(WEBHOOK_URL_ENV) else {
        return; // Webhook disabled — feature is opt-in.
    };
    if url.is_empty() {
        return;
    }
    let secret = env::var(WEBHOOK_SECRET_ENV).ok();

    tokio::spawn(async move {
        if let Err(e) = post_event(&url, secret.as_deref(), &event).await {
            warn!(error = %e, url = %url, "Provider-linked webhook POST failed");
        } else {
            info!(
                user_id = %event.user_id,
                channel = %event.channel,
                provider = %event.provider,
                "Emitted provider-linked webhook"
            );
        }
    });
}

async fn post_event(
    url: &str,
    secret: Option<&str>,
    event: &ProviderLinkedEvent,
) -> Result<(), String> {
    let payload =
        serde_json::to_vec(event).map_err(|e| format!("Failed to serialize event: {e}"))?;

    let mut builder = WEBHOOK_CLIENT
        .post(url)
        .timeout(WEBHOOK_TIMEOUT)
        .header("content-type", "application/json")
        .body(payload.clone());

    if let Some(s) = secret {
        if !s.is_empty() {
            let signature = sign_payload(s, &payload)?;
            builder = builder.header(SIGNATURE_HEADER, signature);
        }
    }

    let resp = builder
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!(
            "Webhook returned non-success status: {}",
            resp.status()
        ));
    }
    Ok(())
}
