// ABOUTME: The compiled JSON-schema validators the inline visual blocks are checked against
// ABOUTME: Schema texts keyed by id, compiled once per process from the contremaitre constants
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde_json::Value;
use tracing::warn;

/// Schema id for an inline visual block (`chart` or `table`).
///
/// The one schema a reply's blocks are validated against. A block is
/// embedded in prose and a reply may carry several; the plan card is not a
/// schema at all but a projection of the saved plan
/// (`pierre_services::plan_card`).
pub const DRAVR_VIZ: &str = "dravr-viz";

/// Schema texts keyed by schema id.
pub type SchemaTexts = BTreeMap<String, String>;

/// Compiled validators keyed by schema id, built once on first use from the
/// schema texts threaded through the pipeline context.
///
/// Keyed rather than singular so a second block schema slots in beside the
/// first. The set is fixed at startup — every schema is a compiled-in
/// contremaitre constant — so one build covers every later turn.
static VALIDATORS: OnceLock<BTreeMap<String, jsonschema::Validator>> = OnceLock::new();

/// Compile one schema, or log why it could not be compiled.
fn compile(id: &str, schema_json: &str) -> Option<jsonschema::Validator> {
    let mut schema: Value = match serde_json::from_str(schema_json) {
        Ok(value) => value,
        Err(e) => {
            warn!(schema = id, error = %e, "viz-schema: schema text is not valid JSON");
            return None;
        }
    };
    // Drop the `$schema`/`$id` URIs before compiling. Their presence makes the
    // validator resolve the draft meta-schema (and the `$id` base) over the
    // network, which would block the first block-bearing turn on
    // json-schema.org reachability. Stripped, the embedded draft is used and
    // the schema's relative `#/$defs/...` refs still resolve against the
    // document root.
    if let Some(obj) = schema.as_object_mut() {
        obj.remove("$schema");
        obj.remove("$id");
    }
    match jsonschema::validator_for(&schema) {
        Ok(validator) => Some(validator),
        Err(e) => {
            warn!(schema = id, error = %e, "viz-schema: schema failed to compile");
            None
        }
    }
}

/// Look up the compiled validator for `id`, building the registry on first use.
///
/// A schema that fails to compile is simply absent, so its blocks never
/// validate — scoped to the one broken schema rather than disabling every
/// block.
pub(super) fn validator_for(
    schemas: &SchemaTexts,
    id: &str,
) -> Option<&'static jsonschema::Validator> {
    VALIDATORS
        .get_or_init(|| {
            schemas
                .iter()
                .filter_map(|(key, text)| compile(key, text).map(|v| (key.clone(), v)))
                .collect()
        })
        .get(id)
}
