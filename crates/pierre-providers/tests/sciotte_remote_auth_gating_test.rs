// ABOUTME: Pins the RemoteSciotteClient construction gate: audience-or-loopback, never a shared key
// ABOUTME: Asserts a loopback scrape carries no Authorization header even with the retired key env set
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Construction-gate contract for `RemoteSciotteClient` (registre#36 cutover).
//
// This `//!` must precede the crate-level `#![cfg]`: when the feature is off the
// cfg empties the crate (dropping any inner `#![allow(missing_docs)]`), so without
// a surviving crate doc the command-line `-D warnings` trips `missing_docs`.
#![cfg(feature = "provider-sciotte")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! The scraper service gates on Google identity tokens addressed to
//! `DRAVR_SCIOTTE_AUDIENCE`; the previous shared `DRAVR_SCIOTTE_API_KEY` bearer
//! is retired. The client's rules are load-bearing on both sides:
//!
//! - a **non-loopback** URL without an audience is *disabled* (`Ok(None)`) —
//!   unsigned requests would be refused by the service, and a client that sends
//!   them anyway surfaces as scrapes failing in ways that read as session
//!   problems;
//! - a **loopback** URL needs no audience and sends **no** `Authorization`
//!   header at all — a developer's own scraper serves unauthenticated, and the
//!   retired key variable must stay ignored even when it is still exported.
//!
//! All scenarios live in one test because they mutate process environment;
//! separate `#[test]`s in this binary would race on the same variables.

use std::env;

use pierre_providers::sciotte_remote::{
    RemoteLoginOutcome, RemoteSciotteClient, ENV_AUDIENCE, ENV_REMOTE_URL,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::oneshot;

/// Serve one canned 200 and hand back the raw request bytes, so assertions can
/// look at what the client actually put on the wire.
async fn capture_one_request() -> (String, oneshot::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = oneshot::channel();

    tokio::spawn(async move {
        let Ok((mut stream, _)) = listener.accept().await else {
            return;
        };
        let mut buf = vec![0_u8; 8192];
        let n = stream.read(&mut buf).await.unwrap_or(0);
        let request = String::from_utf8_lossy(&buf[..n]).into_owned();

        let body = r#"{"status":"failed","reason":"capture stub"}"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = stream.write_all(response.as_bytes()).await;
        let _ = tx.send(request);
    });

    (format!("http://{addr}"), rx)
}

#[tokio::test]
async fn construction_gate_is_audience_or_loopback_never_a_shared_key() {
    // 1. Unset URL → not configured.
    env::remove_var(ENV_REMOTE_URL);
    env::remove_var(ENV_AUDIENCE);
    assert!(
        RemoteSciotteClient::from_env()
            .expect("construction itself succeeds")
            .is_none(),
        "no URL means no client"
    );

    // 2. Non-loopback URL without an audience → disabled, not a client that
    //    would send unsigned requests the service refuses.
    env::set_var(ENV_REMOTE_URL, "https://sciotte.example.internal");
    env::remove_var(ENV_AUDIENCE);
    assert!(
        RemoteSciotteClient::from_env()
            .expect("construction itself succeeds")
            .is_none(),
        "a remote URL with no audience must disable the client (both-or-neither)"
    );

    // 3. Non-loopback URL with an audience → enabled. (Token minting itself
    //    needs the metadata server, so this pins only the construction gate.)
    env::set_var(ENV_AUDIENCE, "dravr-sciotte-test");
    assert!(
        RemoteSciotteClient::from_env()
            .expect("construction itself succeeds")
            .is_some(),
        "URL + audience is the configured remote shape"
    );

    // 4. Loopback URL, no audience, and the RETIRED key variable still
    //    exported: the client is enabled and the request carries no
    //    Authorization header at all — the key must be ignored, not sent.
    let (base, captured) = capture_one_request().await;
    env::set_var(ENV_REMOTE_URL, &base);
    env::remove_var(ENV_AUDIENCE);
    env::set_var(
        "DRAVR_SCIOTTE_API_KEY",
        "retired-key-still-in-someones-envrc",
    );

    let client = RemoteSciotteClient::require_from_env().expect("loopback needs no audience");
    let outcome = client
        .login_with_credentials("t@example.com", "pw", "google", "strava")
        .await
        .expect("the capture stub answers a parseable login outcome");
    assert_eq!(
        outcome,
        RemoteLoginOutcome::Failed("capture stub".to_owned()),
        "the canned body must round-trip, proving the request reached the stub"
    );

    let request = captured.await.expect("stub captured the request");
    assert!(
        !request.to_ascii_lowercase().contains("authorization:"),
        "a loopback scrape must be unauthenticated; found an Authorization \
         header in:\n{request}"
    );

    env::remove_var("DRAVR_SCIOTTE_API_KEY");
    env::remove_var(ENV_REMOTE_URL);
}
