// ABOUTME: Platform NotifyLayer seams — AnalyticsProvider (PostHog capture) + NotifyEnricher (user_email/emoji)
// ABOUTME: Maps event tier + consent to a hashed distinct_id and PII-stripped properties; enriches every event
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashMap;
use std::sync::LazyLock;

use dravr_contremaitre::schemas::NOTIFY_EVENTS_YAML;
use dravr_tronc::notify::{AnalyticsCapture, AnalyticsProvider, NotifyEnricher};
use pierre_services::analytics::{hash_id, is_consented, resolve_user_email, EventTier};
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
    // `user_email` is forwarded only as a PostHog person property ($set) for
    // consented product events — never as a flat event property. `emoji` is a
    // Slack-display field with no analytics meaning.
    "user_email",
    "emoji",
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

        let mut properties = build_properties(fields, tier, raw_tenant);

        // Attach the user's email as a PostHog person property ($set) — the
        // idiomatic way to make a person profile identifiable — but only for
        // consented product-tier events. Operational events key on the hashed
        // tenant and are PII-free by design, so they never carry email.
        if tier == EventTier::Product {
            if let Some(email) = fields.get("user_email") {
                set_person_email(&mut properties, email);
            }
        }

        Some(AnalyticsCapture {
            distinct_id,
            properties,
        })
    }
}

/// Set `email` as a `PostHog` person property under the reserved `$set` key,
/// creating the `$set` object when absent. No-op if `properties` is not a JSON
/// object (it always is, from [`build_properties`]).
///
/// ```
/// use pierre_mcp_server::analytics_sink::set_person_email;
/// use serde_json::json;
///
/// let mut props = json!({ "tier": "product" });
/// set_person_email(&mut props, "jane@acme.com");
/// assert_eq!(props["$set"]["email"], "jane@acme.com");
/// assert_eq!(props["tier"], "product"); // existing properties untouched
/// ```
pub fn set_person_email(properties: &mut Value, email: &str) {
    let Value::Object(map) = properties else {
        return;
    };
    let set = map
        .entry("$set")
        .or_insert_with(|| Value::Object(Map::new()));
    if let Value::Object(set_map) = set {
        set_map.insert("email".to_owned(), Value::String(email.to_owned()));
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

/// Enriches every notify event with host-derived fields for both sinks.
///
/// Injects a `user_email` resolved from the in-process identity cache (so the
/// private monitoring channel shows who, not an opaque UUID) and a per-event
/// display `emoji` for the Slack headline.
///
/// Registered once at startup via [`pierre_logging::set_notify_enricher`]. Runs
/// synchronously on the tracing thread, so it only touches in-memory state —
/// never a database. Events with no `user_id` in scope (pure system events) or
/// a user not yet seen on this pod simply carry no `user_email`.
pub struct PierreNotifyEnricher;

impl NotifyEnricher for PierreNotifyEnricher {
    fn enrich(&self, event: &str, fields: &mut HashMap<String, String>) {
        // Per-event display emoji for the Slack headline. `or_insert` so a
        // call site that set its own emoji wins.
        fields
            .entry("emoji".to_owned())
            .or_insert_with(|| event_emoji(event).to_owned());

        // Resolve the user's email from the identity cache. Clone the id first
        // to release the immutable borrow before the insert.
        if let Some(user_id) = fields.get("user_id").cloned() {
            if let Some(email) = resolve_user_email(&user_id) {
                fields.insert("user_email".to_owned(), email);
            }
        }
    }
}

/// Map a notify event to a Slack-headline emoji by its category (the segment
/// before the first `.`). Every event gets an icon; the `🔔` fallback covers
/// categories without a dedicated glyph.
fn event_emoji(event: &str) -> &'static str {
    match event.split('.').next().unwrap_or_default() {
        "user" => "🔑",
        "chat" => "💬",
        "provider" => "📥",
        "embacle" | "llm" => "🤖",
        "coach" => "🏃",
        "billing" | "checkout" | "subscription" => "💳",
        "group" => "👥",
        "messaging" => "📨",
        "oauth" => "🔗",
        _ => "🔔",
    }
}
