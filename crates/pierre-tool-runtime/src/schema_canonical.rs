// ABOUTME: Canonical (key-ordered) JSON rendering for tool schemas on the wire
// ABOUTME: Exists because serde_json runs with preserve_order, so a HashMap's random order reaches clients
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Deterministic rendering of a tool's input schema.
//!
//! Two facts combine into a bug that is invisible in any single response:
//!
//! 1. `JsonSchema::properties` is a `HashMap`, and Rust reseeds `RandomState`
//!    per map instance — two maps with identical keys, built in the same
//!    process, iterate in different orders.
//! 2. `serde_json` is compiled with `preserve_order` (pulled in transitively),
//!    so `Value::Object` is an `IndexMap` that keeps insertion order instead of
//!    sorting. The random order therefore survives all the way to the wire.
//!
//! The result is that every tool's parameter block is emitted in a different
//! order on every request. That breaks two things at once: no cache validator
//! over the catalog can ever be stable, and — the expensive one — the LLM
//! prompt prefix diverges inside the first tool's `properties`, so prompt
//! caching misses even after the tool *array* was sorted to fix exactly that.
//!
//! Sorting object keys at the rendering boundary fixes both. The real root
//! cause is the `HashMap` in the engine's schema type; changing it there is a
//! breaking change to a public field, so this normalizes at the seam instead.

use serde::Serialize;
use serde_json::{Map, Value};

/// Serialize `value` to JSON with every object key in sorted order.
///
/// Returns `Value::Null` if the value cannot be serialized, matching the
/// permissive behaviour of the call sites this replaces (a tool with an
/// unrenderable schema is listed without one rather than failing the listing).
pub fn to_canonical_value<T: Serialize>(value: &T) -> Value {
    serde_json::to_value(value).map_or(Value::Null, sort_keys)
}

/// Recursively rewrite every object in `value` with its keys sorted.
///
/// Arrays keep their order — element order is meaningful JSON, unlike key
/// order, which is not.
fn sort_keys(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<(String, Value)> = map.into_iter().collect();
            entries.sort_by(|(a, _), (b, _)| a.cmp(b));
            Value::Object(
                entries
                    .into_iter()
                    .map(|(key, value)| (key, sort_keys(value)))
                    .collect::<Map<String, Value>>(),
            )
        }
        Value::Array(items) => Value::Array(items.into_iter().map(sort_keys).collect()),
        other => other,
    }
}
