// ABOUTME: A 401 through the shared provider request shell surfaces as ProviderAuthRequired
// ABOUTME: even when the provider's vendor-error hook would claim that body; other statuses stay the hook's
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! 401 → re-authentication, end to end through `api_request_with_retry`.
//!
//! A vendor answers a rejected token with HTTP 401 and often a body naming the
//! cause. Each provider hands the request shell a hook that decodes its own
//! error bodies, and a hook that claimed a 401 before the shared mapping ran
//! would turn the reconnect signal into a generic external-service error — a
//! revoked credential then fails every turn with no way back to re-auth. This
//! pins the order with a hook that claims every status it is shown: the 401
//! wins anyway, and every other status is still the hook's.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pierre_providers::errors::{AppError, ErrorCode};
use pierre_providers::shared_client;
use pierre_providers::utils::{api_request_with_retry, RetryConfig};
use reqwest::StatusCode;
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

/// A loopback HTTP fixture answering one request with the given status and
/// body. A real socket rather than a mocked client, so the request shell —
/// retry decision, vendor hook, generic mapping — runs exactly as it does in
/// production.
async fn stub_once(status_line: &'static str, body: &'static str) -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind stub");
    let addr = listener.local_addr().expect("stub addr");
    let handle = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("accept");
        let mut head = Vec::new();
        let mut buf = [0_u8; 1024];
        loop {
            let n = socket.read(&mut buf).await.expect("read request");
            if n == 0 {
                break;
            }
            head.extend_from_slice(&buf[..n]);
            if head.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }
        let response = format!(
            "HTTP/1.1 {status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        socket
            .write_all(response.as_bytes())
            .await
            .expect("write response");
        socket.flush().await.expect("flush response");
    });
    (format!("http://{addr}/athlete"), handle)
}

/// The error a vendor hook that claims every non-success status returns,
/// quoting the body it was shown — the shape that would shadow a 401 if it
/// ran first.
fn claimed(status: StatusCode, body: &str) -> AppError {
    AppError::external_service("strava", format!("vendor hook claimed {status}: {body}"))
}

/// A rejected-token body of the kind a vendor sends with its 401.
const EXPIRED_TOKEN_BODY: &str =
    r#"{"errors":[{"errorType":"expired_token","message":"Access token expired"}]}"#;

#[tokio::test]
async fn a_401_requires_reauth_before_the_vendor_hook_reads_it() {
    let (url, stub) = stub_once("401 Unauthorized", EXPIRED_TOKEN_BODY).await;

    let err = api_request_with_retry::<Value, _>(
        shared_client(),
        &url,
        "revoked-token",
        "strava",
        &RetryConfig::default(),
        |status, body| Some(claimed(status, body)),
    )
    .await
    .expect_err("a 401 is a failure");
    stub.await.expect("stub served the request");

    assert_eq!(
        err.code,
        ErrorCode::ProviderAuthRequired,
        "a 401 must carry ProviderAuthRequired so the chat pipeline mints a reconnect link; got {err}"
    );
    assert_eq!(
        err.provider_auth_required_provider().as_deref(),
        Some("strava"),
        "the provider slug must survive in details"
    );
    assert!(
        !err.to_string().contains("vendor hook claimed"),
        "the hook must never see a 401; got {err}"
    );
}

#[tokio::test]
async fn any_other_status_keeps_the_vendor_hook_reading() {
    let (url, stub) = stub_once(
        "403 Forbidden",
        r#"{"errors":[{"errorType":"insufficient_scope","message":"no access to sleep data"}]}"#,
    )
    .await;

    let err = api_request_with_retry::<Value, _>(
        shared_client(),
        &url,
        "valid-token",
        "strava",
        &RetryConfig::default(),
        |status, body| Some(claimed(status, body)),
    )
    .await
    .expect_err("a 403 is a failure");
    stub.await.expect("stub served the request");

    assert_ne!(err.code, ErrorCode::ProviderAuthRequired);
    assert!(
        err.to_string()
            .contains("vendor hook claimed 403 Forbidden"),
        "the hook owns every status but 401; got {err}"
    );
    assert!(
        err.to_string().contains("no access to sleep data"),
        "the vendor's own words must survive; got {err}"
    );
}
