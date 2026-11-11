// ABOUTME: `settings connect-order` through the real binary, against the guardian suite's stand-in server
// ABOUTME: Pins the request each verb sends from its flags, what it prints, and that a refused flip exits non-zero (carnet#1472)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The verbs are thin clients of `/admin/settings/connect-order`, so what they
//! owe is the request their flags make and the exit code a script reads.

use serde_json::{json, Value};

use crate::guardian_settings_cli_test::{stdout_of, CliHome, ServerStandIn};

/// The server's answer to a read or a flip: Garmin on the OAuth first, every
/// other provider on the default sign-in first.
fn orders_reply() -> Value {
    json!({
        "success": true,
        "message": "ok",
        "data": {
            "orders": {
                "coros": "sign_in_first",
                "garmin": "oauth_first",
                "strava": "sign_in_first",
                "trainingpeaks": "sign_in_first",
            },
            "description": "Which way a NEW connection of each provider goes first."
        }
    })
}

#[test]
fn set_sends_the_flags_as_the_wire_forms_and_prints_every_order() {
    let server = ServerStandIn::serve(vec![(200, orders_reply())]);
    let home = CliHome::for_server(&server.base_url);

    let output = home.run(&[
        "settings",
        "connect-order",
        "set",
        "--provider",
        "strava",
        "--first",
        "sign-in",
    ]);

    assert_eq!(
        output.status.code(),
        Some(0),
        "bug: a landed flip exits non-zero: {output:?}"
    );
    let stdout = stdout_of(&output);
    assert!(
        stdout.contains("strava now connects sign-in first"),
        "bug: the flip is not confirmed: {stdout}"
    );
    assert!(
        stdout.contains("garmin") && stdout.contains("OAuth first"),
        "bug: the read-back omits the other provider: {stdout}"
    );
    let sent = server.sent();
    assert_eq!(
        sent[0].request_line, "PUT /admin/settings/connect-order HTTP/1.1",
        "bug: set reached another route"
    );
    // Wire values: the body the server's handler parses.
    assert_eq!(
        sent[0].body,
        Some(json!({ "provider": "strava", "order": "sign_in_first" })),
        "bug: the flags sent another body"
    );
}

#[test]
fn show_reads_the_route_and_a_refused_flip_exits_non_zero() {
    let server = ServerStandIn::serve(vec![
        (200, orders_reply()),
        (
            403,
            json!({ "success": false, "message": "super-admin only" }),
        ),
    ]);
    let home = CliHome::for_server(&server.base_url);

    let shown = home.run(&["settings", "connect-order", "show"]);
    assert_eq!(shown.status.code(), Some(0), "bug: show failed: {shown:?}");
    assert!(
        stdout_of(&shown).contains("sign-in first  (sign_in_first)"),
        "bug: show does not print the stored order: {}",
        stdout_of(&shown)
    );

    let refused = home.run(&[
        "settings",
        "connect-order",
        "set",
        "--provider",
        "garmin",
        "--first",
        "oauth",
    ]);
    assert_ne!(
        refused.status.code(),
        Some(0),
        "bug: a refused flip exits zero, so a script reads it as landed: {refused:?}"
    );

    let sent = server.sent();
    assert_eq!(
        sent[0].request_line, "GET /admin/settings/connect-order HTTP/1.1",
        "bug: show reached another route"
    );
    assert_eq!(
        sent[1].request_line, "PUT /admin/settings/connect-order HTTP/1.1",
        "bug: set reached another route"
    );
}
