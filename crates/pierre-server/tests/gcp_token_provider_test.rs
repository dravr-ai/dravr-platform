// ABOUTME: Pins the platform-wide GCP token provider — one mint per lifetime, re-mint at expiry, typed failures
// ABOUTME: Drives MetadataTokenProvider against a local stand-in for the metadata server, never GCP
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Every Google API client in the platform — the GCS prompt store, the Cloud
//! KMS KEK provider, the Cloud Tasks enqueuer — takes its bearer token from
//! `pierre_core::gcp_token`. What that module promises is what a burst of
//! calls costs (one mint per token lifetime), what happens when a token is
//! about to expire (a fresh mint, not a 401 downstream), and which typed
//! error each failure of the metadata server turns into. Each is pinned here
//! against a local listener that plays the metadata server.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use pierre_core::errors::ErrorCode;
use pierre_core::gcp_token::{MetadataTokenProvider, TokenProvider};
use tokio::net::TcpListener;
use tokio::time::sleep;

/// What the stand-in metadata server answers, and how often it was asked.
struct MetadataStub {
    status: StatusCode,
    body: String,
    hits: AtomicUsize,
}

async fn serve(stub: Arc<MetadataStub>) -> String {
    async fn handler(State(stub): State<Arc<MetadataStub>>) -> impl IntoResponse {
        stub.hits.fetch_add(1, Ordering::SeqCst);
        (stub.status, stub.body.clone())
    }
    let app = Router::new().route("/token", get(handler)).with_state(stub);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}/token")
}

fn stub(status: StatusCode, body: &str) -> Arc<MetadataStub> {
    Arc::new(MetadataStub {
        status,
        body: body.to_owned(),
        hits: AtomicUsize::new(0),
    })
}

#[tokio::test]
async fn a_minted_token_is_cached_for_its_lifetime() {
    let server = stub(
        StatusCode::OK,
        r#"{"access_token":"ya29.first","expires_in":3600,"token_type":"Bearer"}"#,
    );
    let provider = MetadataTokenProvider::with_token_url(serve(Arc::clone(&server)).await);

    let first = provider.access_token().await.unwrap();
    let second = provider.access_token().await.unwrap();
    let third = provider.access_token().await.unwrap();

    assert_eq!(first, "ya29.first");
    assert_eq!(second, "ya29.first");
    assert_eq!(third, "ya29.first");
    assert_eq!(
        server.hits.load(Ordering::SeqCst),
        1,
        "three calls inside one token lifetime cost one mint"
    );
}

#[tokio::test]
async fn a_token_past_its_lifetime_is_minted_again() {
    // `expires_in: 0` is shorter than the refresh leeway, so the provider
    // keeps it for the one-second floor and then mints again.
    let server = stub(
        StatusCode::OK,
        r#"{"access_token":"ya29.short","expires_in":0,"token_type":"Bearer"}"#,
    );
    let provider = MetadataTokenProvider::with_token_url(serve(Arc::clone(&server)).await);

    provider.access_token().await.unwrap();
    provider.access_token().await.unwrap();
    assert_eq!(
        server.hits.load(Ordering::SeqCst),
        1,
        "still inside the floor"
    );

    sleep(Duration::from_millis(1_100)).await;
    provider.access_token().await.unwrap();
    assert_eq!(
        server.hits.load(Ordering::SeqCst),
        2,
        "the expired token is not served; a fresh one is minted"
    );
}

#[tokio::test]
async fn a_refused_mint_is_an_external_auth_failure() {
    let server = stub(StatusCode::FORBIDDEN, "no service account attached");
    let provider = MetadataTokenProvider::with_token_url(serve(server).await);

    let err = provider.access_token().await.unwrap_err();
    assert_eq!(err.code, ErrorCode::ExternalAuthFailed);
    assert!(
        err.message.contains("HTTP 403"),
        "the status reaches the operator: {}",
        err.message
    );
}

#[tokio::test]
async fn an_undecodable_mint_is_an_external_service_error() {
    let server = stub(StatusCode::OK, "<html>not json</html>");
    let provider = MetadataTokenProvider::with_token_url(serve(server).await);

    let err = provider.access_token().await.unwrap_err();
    assert_eq!(err.code, ErrorCode::ExternalServiceError);
}

#[tokio::test]
async fn an_unreachable_metadata_server_is_unavailable() {
    // Bind then drop: the port is known and nothing listens on it.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    let provider = MetadataTokenProvider::with_token_url(format!("http://{addr}/token"));

    let err = provider.access_token().await.unwrap_err();
    assert_eq!(err.code, ErrorCode::ExternalServiceUnavailable);
}
