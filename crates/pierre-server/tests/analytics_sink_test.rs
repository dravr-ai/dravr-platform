// ABOUTME: Tests the platform AnalyticsProvider — tier lookup, distinct_id strategy, PII stripping
// ABOUTME: Operational events capture consent-free at tenant level; product events gate on consent
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use dravr_tronc::notify::AnalyticsProvider;
use pierre_mcp_server::analytics_sink::{event_tier, PierreAnalyticsProvider};
use pierre_services::analytics::EventTier;
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
