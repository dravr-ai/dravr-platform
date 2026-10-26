// ABOUTME: A success body through the shared provider request shell decodes into the caller's type
// ABOUTME: and a body that is not the expected JSON fails as an ExternalService error quoting no data
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The success branch of `api_request_with_retry`, end to end over a socket.
//!
//! The retry loop hands back the raw body and the generic outer function
//! decodes it into the caller's type, so these pin both halves: a valid body
//! arrives as the caller's typed value, field for field, and a malformed one
//! is an `ExternalService` error naming the provider — never a panic and
//! never a different code a caller would route as a reconnect or a retry.

use pierre_providers::errors::ErrorCode;
use pierre_providers::shared_client;
use pierre_providers::utils::{api_request_with_retry, no_vendor_error, RetryConfig};
use serde::Deserialize;

use super::api_request_401_precedes_vendor_error_test::stub_once;

/// A slice of a vendor athlete payload, typed the way a provider decodes it.
#[derive(Debug, Deserialize, PartialEq, Eq)]
struct Athlete {
    id: u64,
    firstname: String,
}

#[tokio::test]
async fn a_success_body_decodes_into_the_callers_type() {
    let (url, stub) = stub_once("200 OK", r#"{"id":4242,"firstname":"Ada","extra":true}"#).await;

    let athlete = api_request_with_retry::<Athlete, _>(
        shared_client(),
        &url,
        "valid-token",
        "strava",
        &RetryConfig::default(),
        None,
        no_vendor_error,
    )
    .await
    .expect("a well-formed 200 decodes");
    stub.await.expect("stub served the request");

    assert_eq!(
        athlete,
        Athlete {
            id: 4242,
            firstname: "Ada".to_owned(),
        },
        "the body must reach the caller field for field"
    );
}

#[tokio::test]
async fn a_malformed_success_body_is_an_external_service_parse_error() {
    let (url, stub) = stub_once("200 OK", "<html>maintenance</html>").await;

    let err = api_request_with_retry::<Athlete, _>(
        shared_client(),
        &url,
        "valid-token",
        "strava",
        &RetryConfig::default(),
        None,
        no_vendor_error,
    )
    .await
    .expect_err("a body that is not JSON cannot decode");
    stub.await.expect("stub served the request");

    assert_eq!(
        err.code,
        ErrorCode::ExternalServiceError,
        "a provider returning an unreadable body is an external-service failure; got {err}"
    );
    assert!(
        err.to_string().contains("Failed to parse API response"),
        "the error must say the response did not parse; got {err}"
    );
}

#[tokio::test]
async fn a_mistyped_field_fails_without_quoting_the_athletes_data() {
    // `id` arrives as a string: serde's own message would quote it.
    let (url, stub) = stub_once(
        "200 OK",
        r#"{"id":"sentinel-athlete-7f3a","firstname":"Ada"}"#,
    )
    .await;

    let err = api_request_with_retry::<Athlete, _>(
        shared_client(),
        &url,
        "valid-token",
        "strava",
        &RetryConfig::default(),
        None,
        no_vendor_error,
    )
    .await
    .expect_err("a string id cannot decode into u64");
    stub.await.expect("stub served the request");

    let message = err.to_string();
    assert!(
        !message.contains("sentinel-athlete-7f3a"),
        "the parse error quoted provider data: {message}"
    );
    assert!(
        message.contains("Data at line 1"),
        "the error still names the failure's category and position: {message}"
    );
}
