// ABOUTME: Every declared outputSchema must be the same document on every derivation
// ABOUTME: A schema carrying a wall-clock default differs between two tools/list replies, so a client that caches one never gets a hit
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Every declared `outputSchema` must be the same document on every
//! derivation.
//!
//! `tools/list` re-derives the whole catalogue per reply, so a schema that is
//! not a pure function of its type gives two callers — or one caller twice —
//! different documents for the same tool.

use pierre_mcp_server::tools::registry_builtin::get_tools;

/// `tools/list` is answered by re-deriving every schema, so a schema that is
/// not a pure function of its type changes between two replies to the same
/// caller.
///
/// The way that happens is a `#[serde(default = "…")]` whose path is not
/// constant: schemars calls it while generating and serializes the result
/// into `default` (`schemars_derive-1.2.1/src/schema_exprs.rs:785`).
/// `Utc::now` is the one that bites — it writes the generation timestamp
/// into the schema, so no two derivations agree, and the advertised default
/// ("this field defaults to 2026-09-07T18:04:56Z") is meaningless to the
/// client on top of that.
#[test]
fn a_declared_output_schema_is_the_same_document_every_time() {
    let first = get_tools();
    let second = get_tools();

    assert_eq!(
        first.len(),
        second.len(),
        "the tool list itself must not vary between calls"
    );

    let unstable: Vec<&str> = first
        .iter()
        .zip(second.iter())
        .filter(|(a, b)| a.output_schema != b.output_schema)
        .map(|(a, _)| a.name.as_str())
        .collect();

    assert!(
        unstable.is_empty(),
        "these tools derive a different outputSchema each time they are asked \
         for one, so a client cannot cache it: {unstable:?}"
    );

    // Comparing two listings that both carry nothing would pass forever, which
    // is what this test did while the conversions dropped every schema.
    let carried = first
        .iter()
        .filter(|tool| tool.output_schema.is_some())
        .count();
    assert!(
        carried > 100,
        "this compares what the listing carries, so it says nothing unless the \
         listing carries the schemas: found {carried}"
    );
}
