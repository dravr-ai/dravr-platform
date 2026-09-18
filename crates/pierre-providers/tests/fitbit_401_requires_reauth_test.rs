// ABOUTME: A Fitbit 401 carrying the vendor's errors[] body must surface as ProviderAuthRequired,
// ABOUTME: driven through the provider's real request path against a loopback fixture
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Fitbit 401 → re-authentication, end to end through the request shell.
//!
//! Fitbit answers a rejected token with HTTP 401 and a body naming the cause
//! (`expired_token`, `invalid_token`). The provider's vendor hook reads that
//! body, and a hook that claimed it before the shared 401 mapping ran turned
//! the reconnect signal into a generic external-service error — a revoked
//! credential then failed every turn with no way back to re-auth. This pins
//! the order: the 401 wins, whatever the body says.
//
// This `//!` must precede the crate-level `#![cfg]`: when the feature is off the
// cfg empties the crate (dropping any inner `#![allow(missing_docs)]`), so without
// a surviving crate doc the command-line `-D warnings` trips `missing_docs`.
#![cfg(feature = "provider-fitbit")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pierre_providers::core::{FitnessProvider, OAuth2Credentials, ProviderConfig};
use pierre_providers::errors::ErrorCode;
use pierre_providers::fitbit_provider::FitbitProvider;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

/// A loopback HTTP fixture answering one request with the given status and
/// body, standing in for api.fitbit.com. A real socket rather than a mocked
/// client so the provider's own request shell — retry decision, vendor hook,
/// generic mapping — runs exactly as it does in production.
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
    (format!("http://{addr}"), handle)
}

/// A provider pointed at the fixture, holding a credential whose stored expiry
/// is unset — so nothing refreshes and the request goes out as-is, the shape
/// a revoked-but-unexpired token takes.
async fn provider_against(base_url: String) -> FitbitProvider {
    let provider = FitbitProvider::with_config(ProviderConfig {
        name: "fitbit".to_owned(),
        auth_url: "https://www.fitbit.com/oauth2/authorize".to_owned(),
        token_url: format!("{base_url}/oauth2/token"),
        api_base_url: base_url,
        revoke_url: None,
        default_scopes: Vec::new(),
    });
    provider
        .set_credentials(OAuth2Credentials {
            client_id: "client".to_owned(),
            client_secret: "secret".to_owned(),
            access_token: Some("revoked-token".to_owned()),
            refresh_token: Some("refresh".to_owned()),
            expires_at: None,
            scopes: Vec::new(),
        })
        .await
        .expect("set credentials");
    provider
}

/// Fitbit's standard rejected-token body. Its `errors[]` entry is exactly
/// what the vendor hook decodes; the 401 must still win.
const EXPIRED_TOKEN_BODY: &str = r#"{"errors":[{"errorType":"expired_token","message":"Access token expired: revoked-token"}],"success":false}"#;

#[tokio::test]
async fn fitbit_401_with_errors_body_requires_reauth() {
    let (base_url, stub) = stub_once("401 Unauthorized", EXPIRED_TOKEN_BODY).await;
    let provider = provider_against(base_url).await;

    let err = provider
        .get_athlete()
        .await
        .expect_err("a 401 is a failure");
    stub.await.expect("stub served the request");

    assert_eq!(
        err.code,
        ErrorCode::ProviderAuthRequired,
        "a Fitbit 401 must carry ProviderAuthRequired so the chat pipeline mints a reconnect link; got {err}"
    );
    assert_eq!(
        err.provider_auth_required_provider().as_deref(),
        Some("fitbit"),
        "the provider slug must survive in details"
    );
}

/// The vendor hook still owns every other status: a 403 with a scope gap is
/// Fitbit's own vocabulary, not a reconnect prompt.
#[tokio::test]
async fn fitbit_403_scope_gap_keeps_vendor_message() {
    let (base_url, stub) = stub_once(
        "403 Forbidden",
        r#"{"errors":[{"errorType":"insufficient_scope","message":"This application does not have permission to access sleep data."}],"success":false}"#,
    )
    .await;
    let provider = provider_against(base_url).await;

    let err = provider
        .get_athlete()
        .await
        .expect_err("a 403 is a failure");
    stub.await.expect("stub served the request");

    assert_ne!(err.code, ErrorCode::ProviderAuthRequired);
    assert!(
        err.to_string().contains("Insufficient permissions"),
        "the vendor's scope message must reach the caller; got {err}"
    );
    assert!(
        err.to_string().contains("sleep data"),
        "the vendor's own words must survive; got {err}"
    );
}
