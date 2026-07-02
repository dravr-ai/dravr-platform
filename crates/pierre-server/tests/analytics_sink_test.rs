// ABOUTME: Tests the platform AnalyticsProvider — tier lookup, distinct_id strategy, PII stripping
// ABOUTME: Operational events capture consent-free at tenant level; product events gate on consent
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use dravr_tronc::notify::{AnalyticsProvider, NotifyEnricher};
use pierre_mcp_server::analytics_sink::{
    event_tier, PierreAnalyticsProvider, PierreNotifyEnricher,
};
use pierre_services::analytics::{cache_user_email, EventTier};
use serde_json::Value;
use std::collections::HashMap;

fn fields(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

#[test]
fn event_tier_resolves_catalogue_entries() {
    assert_eq!(event_tier("user.login"), Some(EventTier::Product));
    assert_eq!(
        event_tier("embacle.call_completed"),
        Some(EventTier::Operational)
    );
    assert_eq!(event_tier("provider.connected"), Some(EventTier::Product));
    assert_eq!(event_tier("not.a.real.event"), None);
}

#[test]
fn operational_event_captures_without_consent_at_tenant_level() {
    let provider = PierreAnalyticsProvider;
    let f = fields(&[
        ("user_id", "user-123"),
        ("tenant_id", "tenant-abc"),
        ("model", "claude-opus"),
        ("latency_ms", "1500"),
        ("ok", "true"),
        ("uri", "/chat"),
    ]);
    let capture = provider
        .capture_for("embacle.call_completed", &f)
        .expect("operational event captures");

    // distinct_id is the hashed tenant, never the raw value.
    assert_ne!(capture.distinct_id, "tenant-abc");
    assert!(!capture.distinct_id.is_empty());

    let props = capture.properties.as_object().expect("object properties");
    // Identity + request plumbing stripped; no user dimension on operational.
    assert!(!props.contains_key("user_id"));
    assert!(!props.contains_key("tenant_id"));
    assert!(!props.contains_key("uri"));
    // Event-specific fields kept and coerced for aggregation.
    assert_eq!(
        props.get("model").and_then(Value::as_str),
        Some("claude-opus")
    );
    assert_eq!(props.get("latency_ms").and_then(Value::as_i64), Some(1500));
    assert_eq!(props.get("ok").and_then(Value::as_bool), Some(true));
    assert_eq!(
        props.get("tier").and_then(Value::as_str),
        Some("operational")
    );
}

#[test]
fn operational_event_without_tenant_uses_service_bucket() {
    let provider = PierreAnalyticsProvider;
    let f = fields(&[("remaining", "42")]);
    let capture = provider
        .capture_for("llm.rate_limit_low", &f)
        .expect("operational event captures");
    assert_eq!(capture.distinct_id, "service");
}

#[test]
fn product_event_skips_without_consent() {
    // The default process tracker is the noop (analytics-posthog disabled /
    // uninitialised in tests), so is_consented() is always false → product
    // events are dropped. Proves the opt-in gate holds.
    let provider = PierreAnalyticsProvider;
    let f = fields(&[("user_id", "user-123"), ("tenant_id", "tenant-abc")]);
    assert!(provider.capture_for("user.login", &f).is_none());
}

#[test]
fn uncatalogued_event_is_skipped() {
    let provider = PierreAnalyticsProvider;
    let f = fields(&[("user_id", "user-123")]);
    assert!(provider.capture_for("bogus.event", &f).is_none());
}

#[test]
fn enricher_injects_emoji_and_cached_email() {
    // Unique user id keeps the process-wide identity cache test-isolated.
    cache_user_email("enrich-user-1", "alice@acme.com");
    let mut f = fields(&[("user_id", "enrich-user-1"), ("persona", "casual")]);
    PierreNotifyEnricher.enrich("chat.question_asked", &mut f);
    assert_eq!(
        f.get("user_email").map(String::as_str),
        Some("alice@acme.com")
    );
    assert_eq!(f.get("emoji").map(String::as_str), Some("💬"));
    // Pre-existing fields are preserved.
    assert_eq!(f.get("persona").map(String::as_str), Some("casual"));
}

#[test]
fn enricher_emoji_by_category_with_fallback() {
    let cases = [
        ("user.login", "🔑"),
        ("provider.fetch_started", "📥"),
        ("embacle.call_completed", "🤖"),
        ("billing.checkout_completed", "💳"),
        ("something.uncatalogued", "🔔"),
    ];
    for (event, emoji) in cases {
        let mut f = HashMap::new();
        PierreNotifyEnricher.enrich(event, &mut f);
        assert_eq!(
            f.get("emoji").map(String::as_str),
            Some(emoji),
            "event {event}"
        );
    }
}

#[test]
fn enricher_no_email_when_user_uncached() {
    let mut f = fields(&[("user_id", "never-cached-xyz")]);
    PierreNotifyEnricher.enrich("user.login", &mut f);
    assert!(!f.contains_key("user_email"));
    // Emoji is still attached regardless of cache state.
    assert_eq!(f.get("emoji").map(String::as_str), Some("🔑"));
}

#[test]
fn enricher_no_email_without_user_id() {
    let mut f = fields(&[("provider", "strava")]);
    PierreNotifyEnricher.enrich("provider.fetch_started", &mut f);
    assert!(!f.contains_key("user_email"));
}

#[test]
fn enricher_injects_sender_host_without_clobbering_explicit_value() {
    // host_identity() resolves K_REVISION or the machine hostname; either
    // way a test process always has one.
    let resolved = pierre_logging::host_identity().expect("host identity resolves on test hosts");
    assert!(!resolved.is_empty());

    let mut f = HashMap::new();
    PierreNotifyEnricher.enrich("provider.fetch_started", &mut f);
    assert_eq!(f.get("sender_host").map(String::as_str), Some(resolved));

    // A call site that set its own sender_host wins (entry().or_insert).
    let mut preset = fields(&[("sender_host", "explicit-sender")]);
    PierreNotifyEnricher.enrich("provider.fetch_started", &mut preset);
    assert_eq!(
        preset.get("sender_host").map(String::as_str),
        Some("explicit-sender")
    );
}

#[test]
fn sender_host_survives_posthog_property_stripping() {
    // `host` is denylisted as HTTP-header noise; `sender_host` must not be.
    let provider = PierreAnalyticsProvider;
    let f = fields(&[
        ("tenant_id", "tenant-1"),
        ("sender_host", "dev-laptop.local"),
        ("host", "api.dravr.ai"),
    ]);
    let capture = provider
        .capture_for("provider.fetch_started", &f)
        .expect("operational event captures");
    assert_eq!(capture.properties["sender_host"], "dev-laptop.local");
    assert!(capture.properties.get("host").is_none());
}

#[test]
fn enricher_keeps_call_site_emoji() {
    let mut f = fields(&[("emoji", "🎯")]);
    PierreNotifyEnricher.enrich("user.login", &mut f);
    // or_insert: a call-site-provided emoji is not overwritten.
    assert_eq!(f.get("emoji").map(String::as_str), Some("🎯"));
}

#[test]
fn operational_event_strips_email_and_emoji() {
    // Even when the enricher has attached email/emoji, an operational event
    // (hashed-tenant distinct_id, PII-free) forwards neither to PostHog.
    let provider = PierreAnalyticsProvider;
    let f = fields(&[
        ("user_id", "user-789"),
        ("tenant_id", "tenant-xyz"),
        ("provider", "sciotte"),
        ("trigger", "user_initiated"),
        ("user_email", "bob@acme.com"),
        ("emoji", "📥"),
    ]);
    let capture = provider
        .capture_for("provider.fetch_started", &f)
        .expect("operational event captures");
    let props = capture.properties.as_object().expect("object properties");
    assert!(!props.contains_key("user_email"));
    assert!(!props.contains_key("emoji"));
    assert!(!props.contains_key("$set"));
    // distinct_id is the hashed tenant, never the user.
    assert_ne!(capture.distinct_id, "user-789");
}

#[test]
fn product_event_with_email_still_gated_by_consent() {
    // Email present, but without consent (noop tracker in tests) the product
    // event is dropped before any $set promotion — the opt-in gate holds.
    let provider = PierreAnalyticsProvider;
    let f = fields(&[
        ("user_id", "user-123"),
        ("tenant_id", "tenant-abc"),
        ("user_email", "carol@acme.com"),
    ]);
    assert!(provider.capture_for("user.login", &f).is_none());
}
