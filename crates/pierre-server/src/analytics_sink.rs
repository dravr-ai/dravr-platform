// ABOUTME: Platform AnalyticsProvider — resolves per-notify-event PostHog capture decisions
// ABOUTME: Maps event tier + consent to a hashed distinct_id and PII-stripped properties
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashMap;
use std::sync::LazyLock;

use dravr_contremaitre::schemas::NOTIFY_EVENTS_YAML;
use dravr_tronc::notify::{AnalyticsCapture, AnalyticsProvider};
use pierre_services::analytics::{hash_id, is_consented, EventTier};
use serde::Deserialize;
use serde_json::{Map, Value};
use tracing::error;

/// Field keys never forwarded to `PostHog`: identity, the tracing message body,
/// and the HTTP/OAuth plumbing inherited from the enclosing request span
/// (mirrors tronc's Slack denylist so neither sink leaks request internals —
/// notably `username` from the OAuth password grant).
const STRIPPED_KEYS: &[&str] = &[
    "user_id",
    "tenant_id",
    "message",
    "uri",
    "method",
    "version",
    "host",
    "grant_type",
    "username",
    "route",
    "id",
    "x-request-id",
];

#[derive(Deserialize)]
struct RawEvent {
    name: String,
    tier: EventTier,
}

#[derive(Deserialize)]
struct RawCatalogue {
    #[serde(default)]
    events: Vec<RawEvent>,
}

/// Event-name → consent tier, parsed once from the embedded notify catalogue.
///
/// A malformed catalogue would already fail `notify_catalogue_test` at CI; if
/// it somehow fails to parse here the map is empty and every event resolves to
/// `None` — the privacy-safe default, where the sink captures nothing.
static EVENT_TIERS: LazyLock<HashMap<String, EventTier>> =
    LazyLock::new(
        || match serde_yaml::from_str::<RawCatalogue>(NOTIFY_EVENTS_YAML) {
            Ok(raw) => raw.events.into_iter().map(|e| (e.name, e.tier)).collect(),
            Err(e) => {
                error!(error = %e, "notify-events.yaml failed to parse; PostHog capture disabled");
                HashMap::new()
            }
        },
    );

/// Look up the consent tier for a catalogued notify event, or `None` when the
/// event name is not in the catalogue.
#[must_use]
pub fn event_tier(event: &str) -> Option<EventTier> {
    EVENT_TIERS.get(event).copied()
}

/// Resolves `PostHog` capture decisions for the tronc `NotifyLayer`.
///
/// Holds no state: it reads the static tier catalogue and the process-wide
/// analytics consent cache at call time. Registered once at startup via
/// [`pierre_logging::set_analytics_provider`].
pub struct PierreAnalyticsProvider;

impl AnalyticsProvider for PierreAnalyticsProvider {
    fn capture_for(
        &self,
        event: &str,
        fields: &HashMap<String, String>,
    ) -> Option<AnalyticsCapture> {
        let tier = event_tier(event)?;
        let raw_tenant = fields.get("tenant_id");

        let distinct_id = match tier {
            // Per-user behavioural event: identity is the hashed user, gated on
            // the user's opt-in consent.
            EventTier::Product => {
                let hashed_user = hash_id(fields.get("user_id")?);
                if !is_consented(&hashed_user) {
                    return None;
                }
                hashed_user
            }
            // System event: no personal dimension. Tenant-level id, or a
            // service-wide bucket when no tenant is in scope.
            EventTier::Operational => {
                raw_tenant.map_or_else(|| "service".to_owned(), |tenant| hash_id(tenant))
            }
        };

        Some(AnalyticsCapture {
            distinct_id,
            properties: build_properties(fields, tier, raw_tenant),
        })
    }
}

/// Build the `PostHog` property bag: every event field except identity, message,
/// and request plumbing, with values coerced to numbers/bools where possible so
/// `latency_ms` and `cost_usd` are aggregatable. A hashed `tenant` dimension is
/// added for product events (operational events already key on the tenant),
/// plus the `tier` for filtering.
fn build_properties(
    fields: &HashMap<String, String>,
    tier: EventTier,
    raw_tenant: Option<&String>,
) -> Value {
    let mut map = Map::new();
    for (key, value) in fields {
        if STRIPPED_KEYS.contains(&key.as_str()) {
            continue;
        }
        map.insert(key.clone(), coerce(value));
    }
    if tier == EventTier::Product {
        if let Some(tenant) = raw_tenant {
            map.insert("tenant".to_owned(), Value::String(hash_id(tenant)));
        }
    }
    map.insert(
        "tier".to_owned(),
        Value::String(tier_label(tier).to_owned()),
    );
    Value::Object(map)
}

const fn tier_label(tier: EventTier) -> &'static str {
    match tier {
        EventTier::Product => "product",
        EventTier::Operational => "operational",
    }
}

/// Coerce a stringified tracing field to a JSON number or bool when it parses
/// cleanly, so `PostHog` can aggregate `latency_ms`, `cost_usd`, counts, and `ok`.
fn coerce(value: &str) -> Value {
    if value == "true" {
        return Value::Bool(true);
    }
    if value == "false" {
        return Value::Bool(false);
    }
    if let Ok(i) = value.parse::<i64>() {
        return Value::from(i);
    }
    if let Ok(f) = value.parse::<f64>() {
        if f.is_finite() {
            return Value::from(f);
        }
    }
    Value::String(value.to_owned())
}
