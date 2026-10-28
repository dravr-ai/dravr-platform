// ABOUTME: A2A 1.0 push notification delivery — POSTs task updates to registered webhooks
// ABOUTME: Sends through the tronc webhook guard: SSRF deny list checked at registration and at connect
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Push notification (webhook) delivery for A2A 1.0.
//!
//! When a task reaches a significant state (terminal or interrupted), every
//! `TaskPushNotificationConfig` registered on the task receives an HTTP POST
//! whose body is the wire [`StreamResponse`] status-update frame. Delivery
//! is at-least-once and best-effort per event: failures are logged, not
//! retried — clients are required by the spec to acknowledge with a 2xx and
//! deduplicate.

use std::sync::LazyLock;

use dravr_tronc::http_client::guard::{FetchPolicy, GuardError, GuardedClient, GuardedResponse};
use pierre_core::models::a2a::A2APushNotificationConfig;
use reqwest::header::{HeaderValue, AUTHORIZATION};
use secrecy::ExposeSecret;
use tracing::{debug, warn};

use crate::protocol_types::StreamResponse;

/// Shared webhook client under the tronc `webhook` policy: http(s) only, no
/// credentials in the URL, no local-only name, every resolved address
/// checked against the SSRF deny list at connect time, redirects never
/// followed, and a 10-second total cap. An `Err` only when the TLS backend
/// could not be initialised; every validation and delivery then fails with it.
static WEBHOOK_CLIENT: LazyLock<Result<GuardedClient, GuardError>> =
    LazyLock::new(|| GuardedClient::new(FetchPolicy::webhook()));

/// Validate a client-supplied webhook URL against SSRF (spec §13.4).
///
/// Applies the tronc `webhook` policy: http(s) only, no credentials, no
/// local-only name, and no literal or resolved address in the deny list
/// (loopback, private, link-local and metadata, CGNAT, NAT64, 6to4, …).
/// Delivery checks the addresses again at connect time, since the name may
/// resolve elsewhere by then.
///
/// # Errors
///
/// Returns a human-readable rejection reason suitable for an invalid-params
/// response; it never echoes the URL.
pub async fn validate_webhook_url(url: &str) -> Result<(), String> {
    let client = WEBHOOK_CLIENT.as_ref().map_err(ToString::to_string)?;
    client.validate(url).await.map_err(|e| e.to_string())
}

/// Deliver a task update to every registered webhook of the task.
///
/// The body is the `ProtoJSON` [`StreamResponse`] frame. Authentication uses
/// the stored per-config material: `authentication{scheme, credentials}`
/// wins; a bare `token` is sent as `Authorization: Bearer <token>`.
pub async fn deliver_task_update(configs: &[A2APushNotificationConfig], event: &StreamResponse) {
    for config in configs {
        deliver_to_config(config, event).await;
    }
}

/// Deliver one webhook POST, logging (never propagating) failures.
///
/// The guarded client re-checks the URL and every address its host resolves
/// to at connect time, so a name rebound since registration is refused
/// without a separate lookup.
async fn deliver_to_config(config: &A2APushNotificationConfig, event: &StreamResponse) {
    log_delivery_result(config, send_to_config(config, event).await);
}

/// Send one webhook POST through the guarded client.
async fn send_to_config(
    config: &A2APushNotificationConfig,
    event: &StreamResponse,
) -> Result<GuardedResponse, String> {
    let client = WEBHOOK_CLIENT.as_ref().map_err(ToString::to_string)?;
    let mut request = client
        .post(&config.url)
        .map_err(|e| e.to_string())?
        .json(event);
    if let Some(header) = authorization_header(config) {
        let mut value = HeaderValue::from_str(&header)
            .map_err(|_| "stored webhook credentials are not a valid header value".to_owned())?;
        value.set_sensitive(true);
        request = request.header(AUTHORIZATION, value);
    }
    request.send().await.map_err(|e| e.to_string())
}

/// `Authorization` header value for a webhook request: the stored
/// `authentication{scheme, credentials}` wins; a bare `token` is sent as
/// `Bearer <token>`.
fn authorization_header(config: &A2APushNotificationConfig) -> Option<String> {
    config
        .auth_scheme
        .as_ref()
        .map(|scheme| {
            let credentials = config.auth_credentials.as_deref().unwrap_or_default();
            format!("{scheme} {credentials}")
        })
        .or_else(|| {
            config
                .token
                .as_ref()
                .map(|token| format!("Bearer {}", token.expose_secret()))
        })
}

/// Log the outcome of one webhook delivery attempt.
fn log_delivery_result(
    config: &A2APushNotificationConfig,
    result: Result<GuardedResponse, String>,
) {
    match result {
        Ok(response) if response.status().is_success() => {
            debug!(
                config_id = %config.config_id,
                task_id = %config.task_id,
                "A2A push notification delivered"
            );
        }
        Ok(response) => {
            warn!(
                config_id = %config.config_id,
                task_id = %config.task_id,
                status = %response.status(),
                "A2A push notification rejected by webhook"
            );
        }
        Err(reason) => {
            warn!(
                config_id = %config.config_id,
                task_id = %config.task_id,
                reason = %reason,
                "A2A push notification delivery failed"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use serde_json::json;

    use super::*;

    #[tokio::test]
    async fn internal_and_malformed_webhook_urls_are_refused_without_dns() {
        for (url, expected) in [
            (
                "http://169.254.169.254/latest/meta-data/",
                GuardError::DeniedAddress,
            ),
            ("http://127.0.0.1:8081/admin", GuardError::DeniedAddress),
            ("http://[::1]/", GuardError::DeniedAddress),
            ("http://[::ffff:10.0.0.1]/", GuardError::DeniedAddress),
            ("http://100.100.100.200/", GuardError::DeniedAddress),
            ("http://localhost/", GuardError::LocalName),
            ("http://metadata.google.internal/", GuardError::LocalName),
            (
                "https://user:secret@example.com/hook",
                GuardError::CredentialsInUrl,
            ),
            ("ftp://example.com/hook", GuardError::SchemeNotAllowed),
            ("not a url", GuardError::InvalidUrl),
        ] {
            let reason = validate_webhook_url(url).await.err();
            assert_eq!(
                reason,
                Some(expected.to_string()),
                "bug: the webhook guard must refuse {url} for the documented reason"
            );
        }
    }

    #[tokio::test]
    async fn delivery_to_an_internal_url_never_leaves_the_guard() {
        let config = A2APushNotificationConfig {
            config_id: "config-1".to_owned(),
            task_id: "task-1".to_owned(),
            url: "http://169.254.169.254/latest/meta-data/".to_owned(),
            token: None,
            auth_scheme: Some("Bearer".to_owned()),
            auth_credentials: Some("webhook-secret".to_owned()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let event: StreamResponse = serde_json::from_value(json!({
            "statusUpdate": {
                "taskId": "task-1",
                "contextId": "context-1",
                "status": { "state": "TASK_STATE_COMPLETED" }
            }
        }))
        .expect("a status-update frame deserializes"); // Safe: test assertion
        let outcome = send_to_config(&config, &event).await;
        assert_eq!(
            outcome.err(),
            Some(GuardError::DeniedAddress.to_string()),
            "bug: delivery must refuse the metadata address before any request leaves"
        );
    }
}
