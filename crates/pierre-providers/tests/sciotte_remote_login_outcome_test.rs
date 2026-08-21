// ABOUTME: Pins the platform's parsing of every sciotte login status against real payloads
// ABOUTME: The bodies are verbatim captures from a running dravr-sciotte server, not invented
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Cross-repo contract tests for `RemoteSciotteClient` login outcomes.
//
// This `//!` must precede the crate-level `#![cfg]`: when the feature is off the
// cfg empties the crate (dropping any inner `#![allow(missing_docs)]`), so without
// a surviving crate doc the command-line `-D warnings` trips `missing_docs`.
#![cfg(feature = "provider-sciotte")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! `sciotte_remote.rs` turns dravr-sciotte's JSON into `RemoteLoginOutcome`, and every
//! 2FA path a user can hit depends on that mapping: `otp_required` drives the code
//! prompt, `two_factor_choice` the method picker, `number_match` the phone-tap screen.
//! Nothing tested it — the strings appeared only in the implementation — so a rename on
//! either side of the repo boundary would have surfaced as a login that silently fails.
//!
//! The bodies below are verbatim captures from a real `dravr-sciotte-server` driven over
//! HTTP on 2026-08-14, so this test fails if either side of the contract drifts.

use std::env;
use std::sync::Arc;

use pierre_providers::sciotte_remote::{RemoteLoginOutcome, RemoteSciotteClient, ENV_REMOTE_URL};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Mutex;

/// Serve one canned `(status, body)` per connection, in order.
///
/// A queue rather than a router: each assertion below drives exactly one request, so
/// order is the simplest thing that cannot silently mismatch. An exhausted queue keeps
/// answering — with a 500 that no assertion accepts — rather than dropping the
/// listener, so an unexpected extra request fails as a wrong *outcome* instead of a
/// connection error that says nothing about what went wrong.
async fn stub_server(responses: Vec<(u16, String)>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let queue = Arc::new(Mutex::new(responses.into_iter().collect::<Vec<_>>()));

    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let (status, body) = {
                let mut q = queue.lock().await;
                if q.is_empty() {
                    (500, r#"{"error":"stub queue exhausted"}"#.to_owned())
                } else {
                    q.remove(0)
                }
            };

            // Drain what the client sent; the assertions are about the response.
            let mut buf = [0_u8; 4096];
            let _ = stream.read(&mut buf).await;

            let response = format!(
                "HTTP/1.1 {status} OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes()).await;
            let _ = stream.flush().await;
        }
    });

    format!("http://{addr}")
}

/// Every case lives in ONE test on purpose.
///
/// `RemoteSciotteClient` is only constructible from the environment, and an env var is
/// process-global: two `#[test]`s setting `DRAVR_SCIOTTE_REMOTE_URL` race under the
/// default parallel harness, and the loser's client talks to the winner's stub. That
/// surfaced as `error sending request` — a connection failure that looks like a broken
/// client rather than a broken test. One test, one server, no shared mutable state.
#[tokio::test]
async fn every_sciotte_login_status_maps_to_its_outcome() {
    // Verbatim from a running dravr-sciotte-server (fake-login fixtures), 2026-08-14.
    let otp = r#"{"status":"otp_required","reason":"Provider requires a one-time password or 2FA verification","flow_id":"6a7f46b9-f274d90","provider":"strava"}"#;
    let choice = r#"{"status":"two_factor_choice","options":[{"id":"app","label":"Tap Yes on your phone or tablet"},{"id":"otp","label":"Get a verification code from the Google Authenticator app"}],"flow_id":"6a7f4723-a59a880","provider":"strava"}"#;
    let number = r#"{"status":"number_match","number":"78","flow_id":"6a7f4723-a59a880","provider":"strava"}"#;
    let failed = r#"{"status":"failed","reason":"Wrong password. Try again."}"#;

    // A 401 from the service's identity-token gate carries no login status.
    let no_status = r#"{"error":"unauthorized","message":"Missing bearer identity token"}"#;

    let base = stub_server(vec![
        (200, otp.to_owned()),
        (200, choice.to_owned()),
        (200, number.to_owned()),
        (401, failed.to_owned()),
        (401, no_status.to_owned()),
    ])
    .await;

    env::set_var(ENV_REMOTE_URL, &base);
    let client = RemoteSciotteClient::require_from_env().expect("client builds from env");

    // 1. A TOTP page after the password submit — the path the scraper fix restored.
    let outcome = client
        .login_with_credentials("test@example.com", "totp-password", "google", "strava")
        .await
        .expect("otp_required is a login outcome, not a transport error");
    match outcome {
        RemoteLoginOutcome::OtpRequired { flow_id } => {
            assert_eq!(flow_id.as_deref(), Some("6a7f46b9-f274d90"));
        }
        other => panic!("expected OtpRequired, got {other:?}"),
    }

    // 2. The 2FA method chooser.
    let outcome = client
        .login_with_credentials("test@example.com", "2fa-password", "google", "strava")
        .await
        .expect("two_factor_choice is a login outcome");
    match outcome {
        RemoteLoginOutcome::TwoFactorChoice { options, flow_id } => {
            assert_eq!(flow_id.as_deref(), Some("6a7f4723-a59a880"));
            let ids: Vec<&str> = options
                .as_array()
                .expect("options is an array")
                .iter()
                .filter_map(|o| o.get("id").and_then(serde_json::Value::as_str))
                .collect();
            assert_eq!(
                ids,
                vec!["app", "otp"],
                "options must pass through verbatim so the UI renders the provider's own choice"
            );
        }
        other => panic!("expected TwoFactorChoice, got {other:?}"),
    }

    // 3. Phone-tap number matching.
    let outcome = client
        .select_2fa("app", Some("6a7f4723-a59a880"))
        .await
        .expect("number_match is a login outcome");
    match outcome {
        RemoteLoginOutcome::NumberMatch { number, flow_id } => {
            assert_eq!(number, "78");
            assert_eq!(flow_id.as_deref(), Some("6a7f4723-a59a880"));
        }
        other => panic!("expected NumberMatch, got {other:?}"),
    }

    // 4. A rejected credential is Failed, carrying the provider's own reason.
    let outcome = client
        .submit_otp("000000", Some("6a7f4723-a59a880"))
        .await
        .expect("an explicit failed status is an outcome, not an error");
    match outcome {
        RemoteLoginOutcome::Failed(reason) => {
            assert_eq!(reason, "Wrong password. Try again.");
        }
        other => panic!("expected Failed, got {other:?}"),
    }

    // 5. A response with no login status must NOT collapse to Failed, or a deployment
    //    fault reads to the user as a bad password — the mistake the implementation
    //    comment records from the first e2e run.
    let result = client
        .login_with_credentials("test@example.com", "whatever", "google", "strava")
        .await;
    assert!(
        result.is_err(),
        "a response with no login status is a transport/auth fault, got {result:?}"
    );

    env::remove_var(ENV_REMOTE_URL);
}
