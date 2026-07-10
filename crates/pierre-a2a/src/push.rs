// ABOUTME: A2A 1.0 push notification delivery — POSTs task updates to registered webhooks
// ABOUTME: SSRF-validates webhook URLs (rejects loopback/private/link-local hosts, DNS included)
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

use std::net::IpAddr;
use std::sync::LazyLock;
use std::time::Duration;

use pierre_core::models::a2a::A2APushNotificationConfig;
use reqwest::header::AUTHORIZATION;
use reqwest::redirect::Policy;
use tokio::net::lookup_host;
use tracing::{debug, warn};
use url::Host;

use crate::protocol_types::StreamResponse;

/// Timeout for a single webhook delivery attempt.
const DELIVERY_TIMEOUT: Duration = Duration::from_secs(10);

/// Shared webhook HTTP client (bounded timeout, no redirects — a redirect
/// could bounce the request to an unvalidated host).
static WEBHOOK_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .timeout(DELIVERY_TIMEOUT)
        .redirect(Policy::none())
        .build()
        .unwrap_or_default()
});

/// Whether an IP address is unsafe as a webhook target (SSRF guard).
fn is_forbidden_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
        }
        IpAddr::V6(v6) => {
            // A v4-mapped address inherits its v4 classification.
            v6.to_ipv4_mapped().map_or_else(
                || {
                    v6.is_loopback()
                        || v6.is_unspecified()
                        || v6.is_unique_local()
                        || v6.is_unicast_link_local()
                },
                |mapped| is_forbidden_ip(IpAddr::V4(mapped)),
            )
        }
    }
}

/// Validate a client-supplied webhook URL against SSRF (spec §13.4).
///
/// Accepts http(s) only, rejects credentials in the URL, and rejects any
/// URL whose literal host or DNS resolution points at loopback/private/
/// link-local address space.
///
/// # Errors
///
/// Returns a human-readable rejection reason suitable for an invalid-params
/// response.
pub async fn validate_webhook_url(url: &str) -> Result<(), String> {
    let parsed = reqwest::Url::parse(url).map_err(|e| format!("invalid URL: {e}"))?;

    if parsed.scheme() != "https" && parsed.scheme() != "http" {
        return Err(format!("unsupported URL scheme '{}'", parsed.scheme()));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("URL must not embed credentials".into());
    }

    let Some(host) = parsed.host() else {
        return Err("URL has no host".into());
    };

    match host {
        Host::Ipv4(ip) => {
            if is_forbidden_ip(IpAddr::V4(ip)) {
                return Err("URL host resolves to a forbidden address range".into());
            }
        }
        Host::Ipv6(ip) => {
            if is_forbidden_ip(IpAddr::V6(ip)) {
                return Err("URL host resolves to a forbidden address range".into());
            }
        }
        Host::Domain(domain) => {
            validate_webhook_domain(&parsed, domain).await?;
        }
    }

    Ok(())
}

/// Domain-host arm of the SSRF check: reject non-public name suffixes and
/// every address the name currently resolves to.
async fn validate_webhook_domain(parsed: &reqwest::Url, domain: &str) -> Result<(), String> {
    let lowered = domain.to_ascii_lowercase();
    // The last DNS label decides the non-public suffixes (.localhost, .local,
    // .internal — and the bare "localhost" name itself).
    let last_label = lowered.rsplit('.').next().unwrap_or(&lowered);
    if matches!(last_label, "localhost" | "local" | "internal") {
        return Err("URL host is a forbidden local name".into());
    }

    // Resolve and check every address the name currently points at.
    let port = parsed.port_or_known_default().unwrap_or(443);
    let addrs = lookup_host((lowered.as_str(), port))
        .await
        .map_err(|e| format!("URL host does not resolve: {e}"))?;
    for addr in addrs {
        if is_forbidden_ip(addr.ip()) {
            return Err("URL host resolves to a forbidden address range".into());
        }
    }
    Ok(())
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
async fn deliver_to_config(config: &A2APushNotificationConfig, event: &StreamResponse) {
    // Re-validate at delivery time: DNS may have changed since creation.
    if let Err(reason) = validate_webhook_url(&config.url).await {
        warn!(
            config_id = %config.config_id,
            task_id = %config.task_id,
            reason = %reason,
            "Skipping A2A push notification: webhook URL failed validation"
        );
        return;
    }

    let mut request = WEBHOOK_CLIENT.post(&config.url).json(event);
    if let Some(header) = authorization_header(config) {
        request = request.header(AUTHORIZATION, header);
    }

    log_delivery_result(config, request.send().await);
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
        .or_else(|| config.token.as_ref().map(|token| format!("Bearer {token}")))
}

/// Log the outcome of one webhook delivery attempt.
fn log_delivery_result(
    config: &A2APushNotificationConfig,
    result: Result<reqwest::Response, reqwest::Error>,
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
        Err(e) => {
            warn!(
                config_id = %config.config_id,
                task_id = %config.task_id,
                error = %e,
                "A2A push notification delivery failed"
            );
        }
    }
}
