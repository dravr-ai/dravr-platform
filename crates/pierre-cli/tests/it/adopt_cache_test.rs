// ABOUTME: `barrage adopt-cache` through the real binary, against the guardian suite's stand-in server (carnet#1482)
// ABOUTME: Pins the dry-run listing, one request per live pair, the table, and a non-zero exit on any difference

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The verb is a thin client of `POST /admin/barrage/adopt-cache`, so what it
//! owes is the requests it makes, the table it prints and the exit code a
//! script reads.

use serde_json::{json, Value};

use crate::guardian_settings_cli_test::{stdout_of, CliHome, ServerStandIn};

const TENANT: &str = "00000000-0000-0000-0000-0000000000aa";
const USER: &str = "00000000-0000-0000-0000-0000000000bb";

/// One report row.
fn row(provider: &str, kind: &str, outcome: &str, held_after: Option<u64>) -> Value {
    json!({
        "tenant_id": TENANT,
        "user_id": USER,
        "provider": provider,
        "kind": kind,
        "cached": 3,
        "held_before": if kind == "live" { json!(0) } else { Value::Null },
        "held_after": held_after,
        "outcome": outcome,
    })
}

fn reply(dry_run: bool, rows: &[Value]) -> Value {
    json!({
        "success": true,
        "message": "ok",
        "data": { "dry_run": dry_run, "failures": 0, "rows": rows },
    })
}

/// The dry-run listing: one live pair, one archive.
fn listing() -> Value {
    reply(
        true,
        &[
            row("strava", "live", "would_adopt", None),
            row("strava_archive", "archive", "skipped", None),
        ],
    )
}

#[test]
fn a_dry_run_lists_once_and_writes_nothing() {
    let server = ServerStandIn::serve(vec![(200, listing())]);
    let home = CliHome::for_server(&server.base_url);

    let output = home.run(&["barrage", "adopt-cache", "--tenant", TENANT, "--dry-run"]);

    assert_eq!(
        output.status.code(),
        Some(0),
        "bug: a dry run failed: {output:?}"
    );
    let stdout = stdout_of(&output);
    assert!(
        stdout.contains("would adopt") && stdout.contains("archive, skipped"),
        "bug: the table omits a pair: {stdout}"
    );
    assert!(stdout.contains("nothing written"), "bug: {stdout}");
    let sent = server.sent();
    assert_eq!(sent.len(), 1, "bug: a dry run made more than its listing");
    assert_eq!(
        sent[0].request_line, "POST /admin/barrage/adopt-cache HTTP/1.1",
        "bug: the listing reached another route"
    );
    assert_eq!(
        sent[0].body,
        Some(json!({ "tenant_id": TENANT, "dry_run": true })),
        "bug: the listing could write"
    );
}

#[test]
fn each_live_pair_is_adopted_in_its_own_request_and_a_match_exits_zero() {
    let server = ServerStandIn::serve(vec![
        (200, listing()),
        (
            200,
            reply(false, &[row("strava", "live", "matched", Some(3))]),
        ),
    ]);
    let home = CliHome::for_server(&server.base_url);

    let output = home.run(&["barrage", "adopt-cache"]);

    assert_eq!(
        output.status.code(),
        Some(0),
        "bug: a matched adoption failed: {output:?}"
    );
    let stdout = stdout_of(&output);
    assert!(stdout.contains("2 pair(s), 0 to act on."), "bug: {stdout}");
    let sent = server.sent();
    assert_eq!(
        sent.len(),
        2,
        "bug: the archive was adopted, or the live pair was not"
    );
    assert_eq!(
        sent[0].body,
        Some(json!({ "tenant_id": null, "dry_run": true })),
        "bug: no --tenant does not list every tenant"
    );
    assert_eq!(
        sent[1].body,
        Some(json!({
            "tenant_id": TENANT,
            "user_id": USER,
            "provider": "strava",
            "dry_run": false,
        })),
        "bug: the pair's adoption names another scope"
    );
}

#[test]
fn a_difference_exits_non_zero_and_is_named() {
    let server = ServerStandIn::serve(vec![
        (200, listing()),
        (
            200,
            reply(false, &[row("strava", "live", "mismatched", Some(2))]),
        ),
    ]);
    let home = CliHome::for_server(&server.base_url);

    let output = home.run(&["barrage", "adopt-cache", "--tenant", TENANT]);

    assert_ne!(
        output.status.code(),
        Some(0),
        "bug: a difference exits zero, so a script reads the adoption as complete: {output:?}"
    );
    assert!(
        stdout_of(&output).contains("DIFFERS"),
        "bug: the differing pair is not named: {}",
        stdout_of(&output)
    );
}
