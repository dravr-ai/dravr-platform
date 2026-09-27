// ABOUTME: Drives the Slack ops-actions route through dravr-canot's Slack signature verifier and body parser
// ABOUTME: Pins the signature and replay gate plus the form decoding: + is a space, %2B stays a plus

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Slack ops-action route tests.
//
// This `//!` must precede the crate-level `#![cfg]`: when the feature is off the
// cfg empties the crate, so without a surviving crate doc the command-line
// `-D warnings` trips `missing_docs`.
#![cfg(feature = "client-messaging")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;
mod helpers;

use std::env;
use std::sync::Arc;

use axum::http::StatusCode;
use chrono::Utc;
use hmac::{Hmac, Mac};
use pierre_mcp_server::routes::messaging::MessagingRoutes;
use serde_json::Value;
use serial_test::serial;
use sha2::Sha256;

use crate::common::create_test_server_resources;
use crate::helpers::axum_test::AxumTestRequest;

/// The signing secret the route reads from `SLACK_SIGNING_SECRET`.
const SIGNING_SECRET: &str = "slack-ops-route-test-secret";

/// Slack's v0 signature over `v0:{timestamp}:{body}`.
fn sign(timestamp: &str, body: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(SIGNING_SECRET.as_bytes()).unwrap();
    mac.update(format!("v0:{timestamp}:{body}").as_bytes());
    format!("v0={}", hex::encode(mac.finalize().into_bytes()))
}

/// A form body carrying `encoded_payload_json` (already form-encoded) under
/// `payload`, the shape Slack posts for an interactive action.
fn form_body(encoded_payload_json: &str) -> String {
    format!("payload={encoded_payload_json}")
}

/// Post `body` to the ops-actions route with the given signature headers and
/// return the status plus the ephemeral text Slack would show the clicker.
///
/// Every test sets the same secret and runs serially, so the environment
/// write never races a reader in another test.
async fn post_action(body: &str, timestamp: &str, signature: &str) -> (StatusCode, String) {
    env::set_var("SLACK_SIGNING_SECRET", SIGNING_SECRET);
    let resources = create_test_server_resources().await.unwrap();
    let router = MessagingRoutes::routes(Arc::clone(&resources));
    let response = AxumTestRequest::post("/api/ops/slack/actions")
        .header("content-type", "application/x-www-form-urlencoded")
        .header("x-slack-request-timestamp", timestamp)
        .header("x-slack-signature", signature)
        .text(body)
        .send(router)
        .await;
    let status = response.status_code();
    let json: Value = response.json();
    let text = json["text"].as_str().unwrap_or_default().to_owned();
    (status, text)
}

#[tokio::test]
#[serial]
async fn a_forged_signature_is_refused_before_the_payload_is_read() {
    let body = form_body("%7B%22type%22%3A%22block_actions%22%7D");
    let timestamp = Utc::now().timestamp().to_string();
    let forged = format!("v0={}", "0".repeat(64));

    let (status, text) = post_action(&body, &timestamp, &forged).await;

    // 200 keeps Slack from retrying; the text is what refuses.
    assert_eq!(status, StatusCode::OK);
    assert_eq!(text, "Signature verification failed");
}

#[tokio::test]
#[serial]
async fn a_correctly_signed_but_stale_request_is_refused_as_a_replay() {
    let body = form_body("%7B%22type%22%3A%22block_actions%22%7D");
    let stale = (Utc::now().timestamp() - 600).to_string();
    let signature = sign(&stale, &body);

    let (status, text) = post_action(&body, &stale, &signature).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(text, "Signature verification failed");
}

#[tokio::test]
#[serial]
async fn a_signed_payload_decodes_a_form_plus_as_a_space() {
    // Slack form-encodes a space as `+`. The payload type is the first field
    // the handler reads back, so it carries the decoded value to the reply.
    let body = form_body("%7B%22type%22%3A%22block+actions%22%7D");
    let timestamp = Utc::now().timestamp().to_string();
    let signature = sign(&timestamp, &body);

    let (status, text) = post_action(&body, &timestamp, &signature).await;

    assert_eq!(status, StatusCode::OK);
    assert!(
        text.contains("Unexpected payload type: block actions"),
        "the signature passed and `+` decoded to a space: {text}"
    );
}

#[tokio::test]
#[serial]
async fn a_signed_payload_keeps_an_encoded_plus_as_a_literal_plus() {
    // `%2B` is how Slack sends a literal `+`; decoding it to a space would
    // corrupt any value that holds one.
    let body = form_body("%7B%22type%22%3A%22a%2Bb%22%7D");
    let timestamp = Utc::now().timestamp().to_string();
    let signature = sign(&timestamp, &body);

    let (status, text) = post_action(&body, &timestamp, &signature).await;

    assert_eq!(status, StatusCode::OK);
    assert!(
        text.contains("Unexpected payload type: a+b"),
        "an encoded plus must survive decoding as a plus: {text}"
    );
}
