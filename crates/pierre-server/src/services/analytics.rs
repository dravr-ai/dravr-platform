// ABOUTME: Product analytics trait and PostHog implementation for messaging funnel tracking
// ABOUTME: OnceLock singleton with noop fallback; SHA-256 hashed user IDs for anonymization
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::env;
use std::sync::OnceLock;

use dashmap::DashMap;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tracing::{info, warn};

/// Global singleton instance, initialized once at server startup
static ANALYTICS: OnceLock<Box<dyn AnalyticsTracker>> = OnceLock::new();

/// SHA-256 hash a value for anonymized analytics identifiers
///
/// `PostHog` never sees real user UUIDs, tenant IDs, or channel user IDs.
/// Only the hash (hex-encoded) is transmitted.
pub fn hash_id(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    hex::encode(hasher.finalize())
}

/// Initialize the global analytics tracker from environment variables
///
/// When `POSTHOG_ENABLED` is `true` (default) and `POSTHOG_API_KEY` is set,
/// a live `PostHog` tracker is created. Otherwise a noop tracker is installed
/// so callers never need null checks.
pub fn init_analytics() {
    let enabled = env::var("POSTHOG_ENABLED")
        .map(|v| v != "false" && v != "0")
        .unwrap_or(true);

    let tracker: Box<dyn AnalyticsTracker> = if enabled {
        match create_posthog_tracker() {
            Some(tracker) => tracker,
            None => {
                warn!("Analytics enabled but POSTHOG_API_KEY not set -- using noop tracker");
                Box::new(NoopAnalyticsTracker)
            }
        }
    } else {
        info!("Analytics disabled (POSTHOG_ENABLED=false)");
        Box::new(NoopAnalyticsTracker)
    };

    let _ = ANALYTICS.set(tracker);
}

/// Fallback noop instance for when `init_analytics()` has not been called
static NOOP_FALLBACK: NoopAnalyticsTracker = NoopAnalyticsTracker;

/// Get a reference to the global analytics tracker
///
/// Always returns a valid tracker, either the live `PostHog` client or a noop.
/// If called before `init_analytics()`, returns a noop silently.
pub fn analytics() -> &'static dyn AnalyticsTracker {
    ANALYTICS
        .get()
        .map_or(&NOOP_FALLBACK as &dyn AnalyticsTracker, AsRef::as_ref)
}

// =============================================================================
// Trait
// =============================================================================

/// Product analytics tracker for messaging funnel events, tool usage, and commands
///
/// Implementations are either a live `PostHog` client or a noop.
/// All methods are fire-and-forget; errors are logged internally, never propagated.
/// All user/tenant identifiers are pre-hashed by callers via `hash_id()`.
pub trait AnalyticsTracker: Send + Sync {
    // -- Messaging Funnel --

    /// New messaging session created (user linked, first message on new channel)
    fn track_session_started(&self, channel: &str, tenant_id: &str, user_id: &str, is_new: bool);

    /// Inbound message received and stored
    fn track_message_received(
        &self,
        channel: &str,
        tenant_id: &str,
        user_id: &str,
        content_type: &str,
    );

    /// User intent recognized (link_code, otp_flow, slash_command:<name>, logout, normal_chat)
    fn track_intent(&self, channel: &str, tenant_id: &str, distinct_id: &str, intent_type: &str);

    /// Bot response sent back to the user
    fn track_bot_response(
        &self,
        channel: &str,
        tenant_id: &str,
        user_id: &str,
        response_type: &str,
        execution_time_ms: u64,
        model: &str,
    );

    /// Account linking completed (deep_link or otp)
    fn track_linking_completed(&self, channel: &str, tenant_id: &str, user_id: &str, method: &str);

    /// Account linking failed
    fn track_linking_failed(&self, channel: &str, tenant_id: &str, reason: &str);

    /// Error occurred (llm_dispatch_failed, delivery_failed, dead_lettered)
    fn track_error(&self, channel: &str, tenant_id: &str, error_type: &str);

    /// Outbound message delivered successfully
    fn track_outbound_delivered(&self, channel: &str, tenant_id: &str, is_retry: bool);

    /// Unlinked user prompted to link their account
    fn track_unlinked_prompted(&self, channel: &str, tenant_id: &str, prompt_type: &str);

    /// Session ended (logout, timeout, unlinked)
    fn track_session_dropped(&self, channel: &str, tenant_id: &str, user_id: &str, reason: &str);

    // -- Tool & Command Usage --

    /// MCP tool executed during LLM dispatch (auto-captured per tool name)
    fn track_tool_executed(
        &self,
        channel: &str,
        tenant_id: &str,
        user_id: &str,
        tool_name: &str,
        success: bool,
        duration_ms: u64,
    );

    /// Slash command executed (auto-captured per command name)
    fn track_command_executed(
        &self,
        channel: &str,
        tenant_id: &str,
        user_id: &str,
        command_name: &str,
        success: bool,
    );

    // -- Identity --

    /// Merge pre-link channel identity with post-link Pierre user ID
    fn alias(&self, channel_identity: &str, pierre_user_id: &str);

    /// Set user properties (platform, tenant, new/returning)
    fn identify(&self, user_id: &str, properties: Value);

    // -- Consent --

    /// Update the consent cache for a user (called on consent change and session start)
    fn set_consent(&self, user_id: &str, enabled: bool);
}

// =============================================================================
// Noop implementation
// =============================================================================

/// No-op tracker used when `PostHog` is disabled or unconfigured
struct NoopAnalyticsTracker;

impl AnalyticsTracker for NoopAnalyticsTracker {
    fn track_session_started(&self, _: &str, _: &str, _: &str, _: bool) {}
    fn track_message_received(&self, _: &str, _: &str, _: &str, _: &str) {}
    fn track_intent(&self, _: &str, _: &str, _: &str, _: &str) {}
    fn track_bot_response(&self, _: &str, _: &str, _: &str, _: &str, _: u64, _: &str) {}
    fn track_linking_completed(&self, _: &str, _: &str, _: &str, _: &str) {}
    fn track_linking_failed(&self, _: &str, _: &str, _: &str) {}
    fn track_error(&self, _: &str, _: &str, _: &str) {}
    fn track_outbound_delivered(&self, _: &str, _: &str, _: bool) {}
    fn track_unlinked_prompted(&self, _: &str, _: &str, _: &str) {}
    fn track_session_dropped(&self, _: &str, _: &str, _: &str, _: &str) {}
    fn track_tool_executed(&self, _: &str, _: &str, _: &str, _: &str, _: bool, _: u64) {}
    fn track_command_executed(&self, _: &str, _: &str, _: &str, _: &str, _: bool) {}
    fn alias(&self, _: &str, _: &str) {}
    fn identify(&self, _: &str, _: Value) {}
    fn set_consent(&self, _: &str, _: bool) {}
}

// =============================================================================
// PostHog implementation
// =============================================================================

/// Attempt to create a `PostHog` tracker from environment variables.
///
/// Returns `None` if `POSTHOG_API_KEY` is not set.
/// The `posthog-rs` client is initialized lazily on first capture (async init).
#[cfg(feature = "analytics-posthog")]
fn create_posthog_tracker() -> Option<Box<dyn AnalyticsTracker>> {
    let api_key = env::var("POSTHOG_API_KEY").ok()?;
    if api_key.is_empty() {
        return None;
    }

    let host = env::var("POSTHOG_HOST").unwrap_or_else(|_| "https://us.i.posthog.com".to_owned());

    info!(host = %host, "PostHog analytics initialized");

    Some(Box::new(PostHogTracker {
        api_key,
        host,
        consent_cache: DashMap::new(),
    }))
}

/// Stub when the `analytics-posthog` feature is disabled.
#[cfg(not(feature = "analytics-posthog"))]
fn create_posthog_tracker() -> Option<Box<dyn AnalyticsTracker>> {
    None
}

#[cfg(feature = "analytics-posthog")]
struct PostHogTracker {
    api_key: String,
    host: String,
    /// Per-user consent cache: hashed_user_id to consented
    /// Missing entry = no consent known, skip events
    consent_cache: DashMap<String, bool>,
}

#[cfg(feature = "analytics-posthog")]
impl PostHogTracker {
    /// Check if a user has given analytics consent.
    /// Returns `false` if user is unknown or has not consented.
    fn has_consent(&self, hashed_user_id: &str) -> bool {
        self.consent_cache.get(hashed_user_id).map_or(false, |v| *v)
    }

    /// Fire-and-forget: spawn a tokio task to capture the event
    fn capture_event(&self, event_name: &str, distinct_id: &str, props: Value) {
        if !self.has_consent(distinct_id) {
            return;
        }

        let api_key = self.api_key.clone();
        let host = self.host.clone();
        let event_name = event_name.to_owned();
        let distinct_id = distinct_id.to_owned();

        tokio::spawn(async move {
            use posthog_rs::{client, Event};

            let client = client((&*api_key, &*host)).await;
            let mut event = Event::new(&event_name, &distinct_id);

            if let Some(obj) = props.as_object() {
                for (k, v) in obj {
                    if let Err(e) = event.insert_prop(k.as_str(), v.clone()) {
                        warn!(key = %k, error = %e, "Failed to set PostHog event property");
                    }
                }
            }

            if let Err(e) = client.capture(event).await {
                warn!(event = %event_name, error = %e, "PostHog capture failed");
            }
        });
    }
}

#[cfg(feature = "analytics-posthog")]
impl AnalyticsTracker for PostHogTracker {
    fn track_session_started(&self, channel: &str, tenant_id: &str, user_id: &str, is_new: bool) {
        self.capture_event(
            "messaging_session_started",
            user_id,
            serde_json::json!({
                "channel": channel,
                "tenant_id": tenant_id,
                "is_new_user": is_new,
            }),
        );
    }

    fn track_message_received(
        &self,
        channel: &str,
        tenant_id: &str,
        user_id: &str,
        content_type: &str,
    ) {
        self.capture_event(
            "messaging_message_received",
            user_id,
            serde_json::json!({
                "channel": channel,
                "tenant_id": tenant_id,
                "content_type": content_type,
            }),
        );
    }

    fn track_intent(&self, channel: &str, tenant_id: &str, distinct_id: &str, intent_type: &str) {
        self.capture_event(
            "messaging_intent_recognized",
            distinct_id,
            serde_json::json!({
                "channel": channel,
                "tenant_id": tenant_id,
                "intent_type": intent_type,
            }),
        );
    }

    fn track_bot_response(
        &self,
        channel: &str,
        tenant_id: &str,
        user_id: &str,
        response_type: &str,
        execution_time_ms: u64,
        model: &str,
    ) {
        self.capture_event(
            "messaging_bot_response_sent",
            user_id,
            serde_json::json!({
                "channel": channel,
                "tenant_id": tenant_id,
                "response_type": response_type,
                "execution_time_ms": execution_time_ms,
                "model": model,
            }),
        );
    }

    fn track_linking_completed(&self, channel: &str, tenant_id: &str, user_id: &str, method: &str) {
        self.capture_event(
            "messaging_linking_completed",
            user_id,
            serde_json::json!({
                "channel": channel,
                "tenant_id": tenant_id,
                "link_method": method,
            }),
        );
    }

    fn track_linking_failed(&self, channel: &str, tenant_id: &str, reason: &str) {
        self.capture_event(
            "messaging_linking_failed",
            tenant_id,
            serde_json::json!({
                "channel": channel,
                "tenant_id": tenant_id,
                "failure_reason": reason,
            }),
        );
    }

    fn track_error(&self, channel: &str, tenant_id: &str, error_type: &str) {
        self.capture_event(
            "messaging_error_occurred",
            tenant_id,
            serde_json::json!({
                "channel": channel,
                "tenant_id": tenant_id,
                "error_type": error_type,
            }),
        );
    }

    fn track_outbound_delivered(&self, channel: &str, tenant_id: &str, is_retry: bool) {
        self.capture_event(
            "messaging_outbound_delivered",
            tenant_id,
            serde_json::json!({
                "channel": channel,
                "tenant_id": tenant_id,
                "is_retry": is_retry,
            }),
        );
    }

    fn track_unlinked_prompted(&self, channel: &str, tenant_id: &str, prompt_type: &str) {
        self.capture_event(
            "messaging_unlinked_user_prompted",
            tenant_id,
            serde_json::json!({
                "channel": channel,
                "tenant_id": tenant_id,
                "prompt_type": prompt_type,
            }),
        );
    }

    fn track_session_dropped(&self, channel: &str, tenant_id: &str, user_id: &str, reason: &str) {
        self.capture_event(
            "messaging_session_dropped",
            user_id,
            serde_json::json!({
                "channel": channel,
                "tenant_id": tenant_id,
                "reason": reason,
            }),
        );
    }

    fn track_tool_executed(
        &self,
        channel: &str,
        tenant_id: &str,
        user_id: &str,
        tool_name: &str,
        success: bool,
        duration_ms: u64,
    ) {
        self.capture_event(
            "messaging_tool_executed",
            user_id,
            serde_json::json!({
                "channel": channel,
                "tenant_id": tenant_id,
                "tool_name": tool_name,
                "success": success,
                "duration_ms": duration_ms,
            }),
        );
    }

    fn track_command_executed(
        &self,
        channel: &str,
        tenant_id: &str,
        user_id: &str,
        command_name: &str,
        success: bool,
    ) {
        self.capture_event(
            "messaging_command_executed",
            user_id,
            serde_json::json!({
                "channel": channel,
                "tenant_id": tenant_id,
                "command_name": command_name,
                "success": success,
            }),
        );
    }

    fn alias(&self, channel_identity: &str, pierre_user_id: &str) {
        let api_key = self.api_key.clone();
        let host = self.host.clone();
        let channel_identity = channel_identity.to_owned();
        let pierre_user_id = pierre_user_id.to_owned();

        tokio::spawn(async move {
            use posthog_rs::{client, Event};

            let client = client((&*api_key, &*host)).await;
            let mut event = Event::new("$create_alias", &pierre_user_id);
            if let Err(e) = event.insert_prop("alias", &channel_identity) {
                warn!(error = %e, "Failed to set alias property");
                return;
            }
            if let Err(e) = client.capture(event).await {
                warn!(error = %e, "PostHog alias capture failed");
            }
        });
    }

    fn identify(&self, user_id: &str, properties: Value) {
        let api_key = self.api_key.clone();
        let host = self.host.clone();
        let user_id = user_id.to_owned();

        tokio::spawn(async move {
            use posthog_rs::{client, Event};

            let client = client((&*api_key, &*host)).await;
            let mut event = Event::new("$identify", &user_id);
            if let Err(e) = event.insert_prop("$set", properties) {
                warn!(error = %e, "Failed to set identify properties");
                return;
            }
            if let Err(e) = client.capture(event).await {
                warn!(error = %e, "PostHog identify capture failed");
            }
        });
    }

    fn set_consent(&self, user_id: &str, enabled: bool) {
        self.consent_cache.insert(user_id.to_owned(), enabled);
    }
}
