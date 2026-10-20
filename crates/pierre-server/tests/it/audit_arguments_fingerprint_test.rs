// ABOUTME: Tests the tool-call audit fingerprint identifies arguments without revealing them
// ABOUTME: Covers stability, canonical ordering independence, and that no argument text survives
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! The 2026-07-28 revision asks for an audit event per tool call naming the
//! arguments a call was made with. Naming them literally is not an option —
//! tool arguments carry athlete data throughout and provider credentials on the
//! connect-style tools — so the trail carries a fingerprint instead. These
//! tests pin the two properties that makes it useful and safe.

use pierre_mcp_server::mcp::audit::arguments_fingerprint;
use serde_json::json;

/// The fingerprint identifies a call: same arguments agree, different arguments
/// disagree, and key order is not part of the identity. That last one is not
/// hypothetical — `serde_json` runs with `preserve_order`, so an unsorted render
/// would give one call a different digest on every request and make the field
/// useless for correlating a replay.
#[test]
fn fingerprint_identifies_arguments_and_ignores_key_order() {
    let a = json!({"provider": "strava", "limit": 50});
    let b = json!({"limit": 50, "provider": "strava"});
    assert_eq!(
        arguments_fingerprint(&a),
        arguments_fingerprint(&b),
        "key order is not part of a call's identity"
    );

    let different = json!({"provider": "strava", "limit": 51});
    assert_ne!(
        arguments_fingerprint(&a),
        arguments_fingerprint(&different),
        "a different argument value must be distinguishable in the trail"
    );

    // Stable across calls, or it cannot correlate anything.
    assert_eq!(arguments_fingerprint(&a), arguments_fingerprint(&a));

    // Nested objects are canonicalized too, not just the top level.
    let nested_a = json!({"filter": {"sport": "run", "after": 1}});
    let nested_b = json!({"filter": {"after": 1, "sport": "run"}});
    assert_eq!(
        arguments_fingerprint(&nested_a),
        arguments_fingerprint(&nested_b)
    );

    // A no-argument call still fingerprints, and agrees with the empty object.
    assert_eq!(
        arguments_fingerprint(&json!(null)),
        arguments_fingerprint(&json!({})),
        "an absent argument set must still produce a comparable fingerprint"
    );
}

/// The fingerprint must not carry its input back out. This is the property that
/// makes it safe to put in a log line at all: an operator reading the audit
/// trail learns that two calls matched, never what was in them.
#[test]
fn fingerprint_reveals_nothing_about_the_arguments() {
    let secret = "AKIA0000EXAMPLESECRET";
    let fingerprint = arguments_fingerprint(&json!({
        "provider": "strava",
        "client_secret": secret,
        "athlete_email": "someone@example.com",
    }));

    for leaked in [secret, "strava", "client_secret", "someone@example.com"] {
        assert!(
            !fingerprint.contains(leaked),
            "the fingerprint leaked {leaked:?}: {fingerprint}"
        );
    }
    assert_eq!(fingerprint.len(), 64, "expected a hex sha256 digest");
    assert!(
        fingerprint.chars().all(|c| c.is_ascii_hexdigit()),
        "a digest must be opaque hex, got {fingerprint}"
    );
}
