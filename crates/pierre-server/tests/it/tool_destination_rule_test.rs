// ABOUTME: No built-in tool serving an athlete's records takes a URL or destination parameter (carnet#1543)
// ABOUTME: Reads what each registered tool declares (its ToolData and capabilities), never a list of names

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::collections::BTreeSet;
use std::mem;

use dravr_tronc::mcp::tool::ToolCapabilities;
use pierre_tool_runtime::read_subject::DataClass;
use pierre_tool_runtime::tool_data::{ToolData, WriteKind};
use serde_json::{json, Value};

use pierre_mcp_server::tools::registry_builtin::register_builtin_tools;
use pierre_tool_runtime::registry::ToolRegistry;

// No barrage-backed tool takes a URL or destination parameter
// (carnet#1543, design row 38). A tool serving an athlete's records
// answers its caller; one that also took a URL, an address or a callback
// could be asked to send them elsewhere, the exfiltration leg of a prompt
// injection.
//
// Barrage-backed is read from what each tool declares, never from a list:
// its declared ToolData serves at least one barrage stream, or its
// capabilities include READS_DATA or REQUIRES_PROVIDER. The stream arm
// holds the writes that serve records (save_training_plan, set_goal); the
// capability arm holds the reads that declare no stream (compose_route,
// over the OpenStreetMap routes discover_routes found).
//
// connect_provider's `redirect_url` is outside the rule by design
// (ChefFamille, 2026-11-23): the tool reads no athlete data and serves no
// stream, so the URL it takes can carry none of their records.
//
// A URL or destination parameter is an input-schema property, at any
// depth, whose name holds one of DESTINATION_WORDS as a word (split on
// non-alphanumerics and camelCase, so `during` never reads as `uri`), or
// whose `format` is one of DESTINATION_FORMATS.

/// Words that make a property a destination: where a request, a message
/// or a file would be sent.
const DESTINATION_WORDS: &[&str] = &[
    "url",
    "urls",
    "uri",
    "uris",
    "href",
    "endpoint",
    "endpoints",
    "webhook",
    "webhooks",
    "callback",
    "callbacks",
    "redirect",
    "destination",
    "destinations",
    "dest",
    "recipient",
    "recipients",
    "email",
    "emails",
    "host",
    "hostname",
];

/// JSON Schema formats that make a property a destination.
const DESTINATION_FORMATS: &[&str] = &[
    "uri",
    "uri-reference",
    "uri-template",
    "iri",
    "iri-reference",
    "url",
    "email",
    "idn-email",
    "hostname",
    "idn-hostname",
];

/// Keywords whose value is one subschema.
const SUBSCHEMA_KEYS: &[&str] = &["items", "additionalProperties", "not"];

/// Keywords whose value is a list of subschemas.
const SUBSCHEMA_LIST_KEYS: &[&str] = &["items", "prefixItems", "oneOf", "anyOf", "allOf"];

/// Keywords whose value maps names to subschemas a caller does not set by
/// that name.
const SUBSCHEMA_MAP_KEYS: &[&str] = &["patternProperties", "$defs", "definitions"];

/// Whether `data` declares a stream. `ToolData` shows its streams to no
/// one outside its crate, so a declaration serves one when it equals none of the
/// stream-less forms; a class added later is counted as serving one until
/// it is listed here, which only widens the rule.
fn serves_a_stream(data: &ToolData) -> bool {
    let classes = [
        DataClass::Training,
        DataClass::Wellness,
        DataClass::NoSubject,
    ];
    let streamless = classes
        .iter()
        .map(|class| ToolData::of(*class, &[]))
        .chain([ToolData::own(&[])]);
    // What a write stores (carnet#1511 call 16) is no stream.
    !streamless
        .flat_map(|form| [form, form.with_athlete_text()])
        .flat_map(|form| {
            [
                form,
                form.writes(WriteKind::FreeText),
                form.writes(WriteKind::Structured),
            ]
        })
        .any(|form| form == *data)
}

/// The tools the invariant covers in `registry`.
fn barrage_backed(registry: &ToolRegistry) -> BTreeSet<String> {
    let reads = ToolCapabilities::READS_DATA | ToolCapabilities::REQUIRES_PROVIDER;
    let by_capability = registry
        .all_tool_metadata()
        .into_iter()
        .filter(|(_, _, capabilities, _)| capabilities.intersects(reads))
        .map(|(name, _, _, _)| name);
    let by_stream = registry
        .tool_names()
        .into_iter()
        .filter(|name| {
            registry
                .get(name)
                .is_some_and(|tool| serves_a_stream(&tool.tool_data()))
        })
        .map(str::to_owned);
    by_capability.chain(by_stream).collect()
}

/// The words of a property name: `callbackUrl`, `callback_url` and
/// `callback-url` are all `callback` and `url`.
fn words(name: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut previous_lower = false;
    for c in name.chars() {
        if !c.is_alphanumeric() {
            if !current.is_empty() {
                words.push(mem::take(&mut current));
            }
            previous_lower = false;
            continue;
        }
        if c.is_uppercase() && previous_lower && !current.is_empty() {
            words.push(mem::take(&mut current));
        }
        previous_lower = c.is_lowercase() || c.is_ascii_digit();
        current.extend(c.to_lowercase());
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

fn names_a_destination(name: &str) -> bool {
    words(name)
        .iter()
        .any(|word| DESTINATION_WORDS.contains(&word.as_str()))
}

fn formats_a_destination(schema: &Value) -> bool {
    schema
        .get("format")
        .and_then(Value::as_str)
        .is_some_and(|format| DESTINATION_FORMATS.contains(&format))
}

/// Every destination property under `schema`, as a path from `at`.
fn destinations(schema: &Value, at: &str, found: &mut Vec<String>) {
    let Some(object) = schema.as_object() else {
        return;
    };
    if let Some(properties) = object.get("properties").and_then(Value::as_object) {
        for (name, property) in properties {
            let path = if at.is_empty() {
                name.clone()
            } else {
                format!("{at}.{name}")
            };
            if names_a_destination(name) || formats_a_destination(property) {
                found.push(path.clone());
            }
            destinations(property, &path, found);
        }
    }
    for key in SUBSCHEMA_KEYS {
        if let Some(sub) = object.get(*key).filter(|sub| sub.is_object()) {
            destinations(sub, &format!("{at}[{key}]"), found);
        }
    }
    for key in SUBSCHEMA_LIST_KEYS {
        if let Some(list) = object.get(*key).and_then(Value::as_array) {
            for (i, sub) in list.iter().enumerate() {
                destinations(sub, &format!("{at}[{key}/{i}]"), found);
            }
        }
    }
    for key in SUBSCHEMA_MAP_KEYS {
        if let Some(map) = object.get(*key).and_then(Value::as_object) {
            for (name, sub) in map {
                let path = format!("{at}[{key}/{name}]");
                if formats_a_destination(sub) {
                    found.push(path.clone());
                }
                destinations(sub, &path, found);
            }
        }
    }
}

fn builtin_registry() -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    register_builtin_tools(&mut registry);
    registry
}

#[test]
fn no_barrage_backed_tool_takes_a_url_or_destination_parameter() {
    let registry = builtin_registry();
    let covered = barrage_backed(&registry);
    assert!(!covered.is_empty(), "premise: the rule covers no tool");
    let mut offending = Vec::new();
    for schema in registry.all_schemas() {
        if !covered.contains(&schema.name) {
            continue;
        }
        let input = serde_json::to_value(&schema.input_schema).unwrap();
        let mut found = Vec::new();
        destinations(&input, "", &mut found);
        offending.extend(
            found
                .into_iter()
                .map(|path| format!("{}: {path}", schema.name)),
        );
    }
    assert!(
        offending.is_empty(),
        "a barrage-backed tool takes a URL or destination parameter, so a caller could point \
         it somewhere other than back to itself (design row 38): {offending:?}"
    );
}

/// The rule covers what design row 38 reviewed, both arms included, or
/// it proves nothing; only a build registering every family has them.
#[cfg(all(
    feature = "tools-connection",
    feature = "tools-data",
    feature = "tools-analytics",
    feature = "tools-goals",
    feature = "tools-groups",
    feature = "tools-config",
    feature = "tools-nutrition",
    feature = "tools-sleep",
    feature = "tools-recipes",
    feature = "tools-agents",
    feature = "tools-admin",
    feature = "tools-mobility",
    feature = "tools-memory",
    feature = "tools-verification",
))]
#[test]
fn the_destination_rule_covers_the_reviewed_tools() {
    let covered = barrage_backed(&builtin_registry());
    for reviewed in [
        "export_routes",
        "compose_route",
        "get_activities",
        "set_goal",
    ] {
        assert!(
            covered.contains(reviewed),
            "premise: {reviewed} is not barrage-backed by the rule: {covered:?}"
        );
    }
    assert!(
        !covered.contains("connect_provider"),
        "premise: connect_provider reads nothing and serves no stream: {covered:?}"
    );
}

#[test]
fn the_destination_rule_reads_words_and_formats_not_substrings() {
    for destination in [
        "url",
        "callbackUrl",
        "redirect_uri",
        "webhook-endpoint",
        "Email",
    ] {
        assert!(
            names_a_destination(destination),
            "bug: {destination} reads as no destination"
        );
    }
    for other in [
        "during",
        "security",
        "curl_count",
        "hosted_by_me",
        "activity_id",
    ] {
        assert!(
            !names_a_destination(other),
            "bug: {other} reads as a destination"
        );
    }
    let mut found = Vec::new();
    destinations(
        &json!({
            "type": "object",
            "properties": {
                "target": {"type": "string", "format": "uri"},
                "steps": {"type": "array", "items": {"properties": {"push_to": {"format": "email"}}}},
                "either": {"anyOf": [{"properties": {"callback": {"type": "string"}}}]}
            }
        }),
        "",
        &mut found,
    );
    found.sort();
    assert_eq!(
        found,
        ["either[anyOf/0].callback", "steps[items].push_to", "target"],
        "bug: a nested destination is missed"
    );
}

#[test]
fn a_streamless_declaration_serves_no_stream() {
    assert!(
        !serves_a_stream(&ToolData::own(&[])),
        "bug: own(&[]) serves a stream"
    );
    assert!(
        !serves_a_stream(&ToolData::of(DataClass::NoSubject, &[]).with_athlete_text()),
        "bug: a text-only declaration serves a stream"
    );
    assert!(
        serves_a_stream(&ToolData::own(&[dravr_barrage::Stream::Workout])),
        "bug: a Workout declaration serves no stream"
    );
}
