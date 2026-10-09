// ABOUTME: Pins the call to Google's decodeIntegrityToken — URL, body, bearer token — and how each answer maps to an outcome
// ABOUTME: Drives GooglePlayIntegrityDecoder against a local stand-in for the Play Integrity API, never Google

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! The sign-in treats the two ways a decode fails oppositely (carnet#810):
//! Google refusing the token (400) refuses the exchange, while Google out of
//! reach lets it proceed as if no evidence had been sent. Which answer lands
//! on which side is the decoder's whole contract, pinned here against a local
//! listener that plays the Play Integrity API.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::post;
use axum::{Json, Router};
use pierre_auth::oauth2_server::play_integrity_decoder::{
    DecodeError, GooglePlayIntegrityDecoder, PlayIntegrityDecoder,
};
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::gcp_token::TokenProvider;
use serde_json::{json, Value};
use tokio::net::TcpListener;

const PACKAGE: &str = "ai.dravr.app";
const ACCESS_TOKEN: &str = "ya29.play-integrity";
const INTEGRITY_TOKEN: &str = "an-integrity-token";

/// What the stand-in API answers, and what it was sent.
struct ApiStub {
    status: StatusCode,
    body: String,
    seen: Mutex<Vec<(String, Option<String>, Value)>>,
}

async fn serve(stub: Arc<ApiStub>) -> String {
    async fn handler(
        State(stub): State<Arc<ApiStub>>,
        Path(method): Path<String>,
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> impl IntoResponse {
        let bearer = headers
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        stub.seen.lock().unwrap().push((method, bearer, body));
        (stub.status, stub.body.clone())
    }
    let app = Router::new()
        .route("/v1/{method}", post(handler))
        .with_state(stub);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

fn stub(status: StatusCode, body: &str) -> Arc<ApiStub> {
    Arc::new(ApiStub {
        status,
        body: body.to_owned(),
        seen: Mutex::new(Vec::new()),
    })
}

/// The service account's token, or the metadata server failing to mint one.
struct FixedToken(Option<&'static str>);

#[async_trait]
impl TokenProvider for FixedToken {
    async fn access_token(&self) -> AppResult<String> {
        self.0.map(str::to_owned).ok_or_else(|| {
            AppError::new(
                ErrorCode::ExternalServiceUnavailable,
                "metadata server unreachable",
            )
        })
    }
}

async fn decoder_for(stub: &Arc<ApiStub>) -> GooglePlayIntegrityDecoder {
    GooglePlayIntegrityDecoder::with_api_url(
        &serve(Arc::clone(stub)).await,
        PACKAGE,
        Arc::new(FixedToken(Some(ACCESS_TOKEN))),
    )
}

fn decoded_body() -> String {
    json!({
        "tokenPayloadExternal": {
            "requestDetails": {
                "requestPackageName": PACKAGE,
                "requestHash": "aGVsbG8gd29scmQgdGhlcmU",
                "timestampMillis": "1675655009345"
            },
            "appIntegrity": {
                "appRecognitionVerdict": "PLAY_RECOGNIZED",
                "packageName": PACKAGE,
                "certificateSha256Digest": ["6a6a1474b5cbbb2b1aa57e0bc3"],
                "versionCode": "42"
            },
            "deviceIntegrity": {
                "deviceRecognitionVerdict": ["MEETS_DEVICE_INTEGRITY"]
            },
            "accountDetails": { "appLicensingVerdict": "LICENSED" },
            "environmentDetails": { "playProtectVerdict": "NO_ISSUES" }
        }
    })
    .to_string()
}

#[tokio::test]
async fn the_token_is_posted_to_the_package_decode_method_as_the_service_account() {
    let api = stub(StatusCode::OK, &decoded_body());
    let decoder = decoder_for(&api).await;

    let payload = decoder.decode(INTEGRITY_TOKEN).await.unwrap();

    assert_eq!(payload.request_details.request_package_name, PACKAGE);
    assert_eq!(
        payload.device_integrity.device_recognition_verdict,
        vec!["MEETS_DEVICE_INTEGRITY".to_owned()]
    );
    let seen = api.seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    let (method, bearer, body) = &seen[0];
    assert_eq!(method, &format!("{PACKAGE}:decodeIntegrityToken"));
    assert_eq!(bearer.as_deref(), Some(&*format!("Bearer {ACCESS_TOKEN}")));
    assert_eq!(body, &json!({ "integrityToken": INTEGRITY_TOKEN }));
}

#[tokio::test]
async fn a_token_google_refuses_is_rejected() {
    let api = stub(
        StatusCode::BAD_REQUEST,
        r#"{"error":{"code":400,"status":"INVALID_ARGUMENT"}}"#,
    );
    let decoder = decoder_for(&api).await;
    assert_eq!(
        decoder.decode(INTEGRITY_TOKEN).await,
        Err(DecodeError::Rejected)
    );
}

#[tokio::test]
async fn an_answer_about_this_server_is_unavailable_not_a_refusal() {
    for status in [
        StatusCode::UNAUTHORIZED,
        StatusCode::FORBIDDEN,
        StatusCode::TOO_MANY_REQUESTS,
        StatusCode::INTERNAL_SERVER_ERROR,
        StatusCode::SERVICE_UNAVAILABLE,
    ] {
        let api = stub(status, r#"{"error":{}}"#);
        let decoder = decoder_for(&api).await;
        assert_eq!(
            decoder.decode(INTEGRITY_TOKEN).await,
            Err(DecodeError::Unavailable),
            "{status}"
        );
    }
}

#[tokio::test]
async fn an_unreadable_verdict_is_unavailable() {
    let api = stub(StatusCode::OK, r#"{"somethingElse":{}}"#);
    let decoder = decoder_for(&api).await;
    assert_eq!(
        decoder.decode(INTEGRITY_TOKEN).await,
        Err(DecodeError::Unavailable)
    );
}

#[tokio::test]
async fn no_access_token_is_unavailable_and_google_is_never_asked() {
    let api = stub(StatusCode::OK, &decoded_body());
    let decoder = GooglePlayIntegrityDecoder::with_api_url(
        &serve(Arc::clone(&api)).await,
        PACKAGE,
        Arc::new(FixedToken(None)),
    );
    assert_eq!(
        decoder.decode(INTEGRITY_TOKEN).await,
        Err(DecodeError::Unavailable)
    );
    assert!(api.seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn google_unreachable_is_unavailable() {
    // Bind a port, then release it, so nothing answers there.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    let decoder = GooglePlayIntegrityDecoder::with_api_url(
        &format!("http://{addr}"),
        PACKAGE,
        Arc::new(FixedToken(Some(ACCESS_TOKEN))),
    );
    assert_eq!(
        decoder.decode(INTEGRITY_TOKEN).await,
        Err(DecodeError::Unavailable)
    );
}
