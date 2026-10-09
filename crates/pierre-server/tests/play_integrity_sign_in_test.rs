// ABOUTME: The Android app's Play Integrity token on its code exchange at /oauth/token (carnet#810)
// ABOUTME: Pins that the verdict binds to the code, a refusal leaves the code unspent, Google out of reach is not a refusal

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! The verdict checks themselves are pinned field by field in `pierre-auth`'s
//! unit tests, and the call to Google in `play_integrity_decoder_test`. These
//! tests drive the token endpoint with a decoder standing in for Google,
//! installed where the server installs the real one (the auth slice the
//! routes' context is built from), so they pin what the endpoint does with a
//! token: where it is accepted, what a refusal costs, that Google out of
//! reach does not lock athletes out while enforcement is off, that Google is
//! only asked about a code that would redeem, and what the `app_attest` span
//! field records.

mod common;
mod helpers;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use common::{create_test_server_resources, create_test_user_with_email};
use helpers::axum_test::{AxumTestRequest, AxumTestResponse};
use helpers::first_party_sign_in::{FirstPartyClient, IssuedCode, SignIn};
use helpers::notify_capture::{capture_logs_and_spans, named, recorded};
use pierre_auth::oauth2_server::first_party::MOBILE_PLAY_PACKAGE_NAME;
use pierre_auth::oauth2_server::play_integrity::{request_hash, TokenPayload};
use pierre_auth::oauth2_server::play_integrity_decoder::{DecodeError, PlayIntegrityDecoder};
use pierre_mcp_server::mcp::resources::ServerContext;
use serde_json::{json, Value};
use uuid::Uuid;

/// `create_test_user_with_email`'s password.
const PASSWORD: &str = "password123";

/// The token the fake answers as Google refusing it.
const REJECTED: &str = "token-google-refuses";

/// The token the fake answers as Google out of reach.
const UNREACHABLE: &str = "token-google-cannot-be-asked-about";

/// Google, for these tests: a token is the verdict it decodes to, as JSON,
/// but for the two sentinel tokens. It counts the decodes it was asked for,
/// each of which is a metered call to the real Google.
struct FakeGoogle {
    decodes: Arc<AtomicUsize>,
}

#[async_trait]
impl PlayIntegrityDecoder for FakeGoogle {
    async fn decode(&self, token: &str) -> Result<TokenPayload, DecodeError> {
        self.decodes.fetch_add(1, Ordering::SeqCst);
        match token {
            REJECTED => Err(DecodeError::Rejected),
            UNREACHABLE => Err(DecodeError::Unavailable),
            verdict => Ok(serde_json::from_str(verdict).expect("a verdict as JSON")),
        }
    }
}

/// A server whose Play Integrity decoder is [`FakeGoogle`], with the count
/// of the decodes it asked for.
async fn counting_server() -> (ServerContext, Arc<AtomicUsize>) {
    let decodes = Arc::new(AtomicUsize::new(0));
    let mut resources = (*create_test_server_resources().await.unwrap()).clone();
    resources.auth.play_integrity = Arc::new(FakeGoogle {
        decodes: Arc::clone(&decodes),
    });
    (resources, decodes)
}

/// A server whose Play Integrity decoder is [`FakeGoogle`].
async fn server() -> ServerContext {
    counting_server().await.0
}

/// The verdict Google returns for a genuine install that requested its token
/// over `code` just now, shaped as Google documents it.
fn genuine_verdict(code: &str) -> Value {
    json!({
        "requestDetails": {
            "requestPackageName": MOBILE_PLAY_PACKAGE_NAME,
            "requestHash": request_hash(code),
            "timestampMillis": Utc::now().timestamp_millis().to_string()
        },
        "appIntegrity": {
            "appRecognitionVerdict": "PLAY_RECOGNIZED",
            "packageName": MOBILE_PLAY_PACKAGE_NAME,
            "certificateSha256Digest": ["6a6a1474b5cbbb2b1aa57e0bc3"],
            "versionCode": "42"
        },
        "deviceIntegrity": {
            "deviceRecognitionVerdict": ["MEETS_DEVICE_INTEGRITY"]
        },
        "accountDetails": {
            "appLicensingVerdict": "LICENSED"
        }
    })
}

async fn athlete(resources: &ServerContext) -> String {
    let email = format!("athlete-{}@example.com", Uuid::new_v4());
    create_test_user_with_email(&resources.agent.database, &email)
        .await
        .unwrap();
    email
}

async fn mobile_code(resources: &ServerContext, email: &str) -> IssuedCode {
    SignIn::new(email, PASSWORD)
        .client(FirstPartyClient::Mobile)
        .issue_code(resources)
        .await
}

/// Redeem `issued` with `evidence` added to the app's form.
async fn redeem_with(issued: &IssuedCode, evidence: &[(&str, &str)]) -> AxumTestResponse {
    let mut form = vec![
        ("grant_type", "authorization_code"),
        ("client_id", issued.client_id),
        ("code", issued.code.as_str()),
        ("redirect_uri", issued.redirect_uri.as_str()),
        ("code_verifier", issued.verifier.as_str()),
    ];
    form.extend_from_slice(evidence);
    AxumTestRequest::post("/oauth/token")
        .form(&form)
        .send(issued.router.clone())
        .await
}

/// Redeem `issued` with `verdict` as its Play Integrity token.
async fn redeem_with_verdict(issued: &IssuedCode, verdict: &Value) -> AxumTestResponse {
    redeem_with(issued, &[("play_integrity_token", &verdict.to_string())]).await
}

fn error_of(response: AxumTestResponse) -> (u16, String) {
    let status = response.status();
    let body: Value = response.json();
    (
        status,
        body["error"].as_str().unwrap_or_default().to_owned(),
    )
}

/// A genuine install's verdict over its own code signs the athlete in, after
/// one decode, and the span records `play_integrity`.
#[tokio::test]
async fn a_genuine_install_signs_in_with_a_verdict_over_its_code() {
    let (resources, decodes) = counting_server().await;
    let email = athlete(&resources).await;
    let issued = mobile_code(&resources, &email).await;

    let (logs, spans, guard) = capture_logs_and_spans();
    let response = redeem_with_verdict(&issued, &genuine_verdict(&issued.code)).await;
    drop(guard);

    assert_eq!(response.status(), 200, "{}", response.body_text());
    let body: Value = response.json();
    assert_eq!(body["user"]["email"], email.as_str(), "{body}");
    let verified = named(&logs, "Play Integrity verdict verified");
    assert_eq!(verified.len(), 1);
    assert_eq!(verified[0].field("strong_integrity"), "false");
    assert_eq!(recorded(&spans, "app_attest"), ["play_integrity"]);
    assert_eq!(decodes.load(Ordering::SeqCst), 1);
}

/// The verdict binds to the code: a token requested over another code is
/// refused, the span records `refused`, and the refusal leaves this code
/// unspent for a retry.
#[tokio::test]
async fn a_verdict_over_another_code_is_refused_and_the_code_survives() {
    let resources = server().await;
    let email = athlete(&resources).await;
    let issued = mobile_code(&resources, &email).await;

    let elsewhere = genuine_verdict("a-code-from-another-sign-in");
    let (_logs, spans, guard) = capture_logs_and_spans();
    let refused = redeem_with_verdict(&issued, &elsewhere).await;
    drop(guard);
    assert_eq!(error_of(refused), (400, "invalid_client".to_owned()));
    assert_eq!(recorded(&spans, "app_attest"), ["refused"]);

    let retried = redeem_with_verdict(&issued, &genuine_verdict(&issued.code)).await;
    assert_eq!(retried.status(), 200, "{}", retried.body_text());
}

/// A binary Google Play does not recognise is refused.
#[tokio::test]
async fn an_app_play_does_not_recognize_is_refused() {
    let resources = server().await;
    let email = athlete(&resources).await;
    let issued = mobile_code(&resources, &email).await;

    let mut sideloaded = genuine_verdict(&issued.code);
    sideloaded["appIntegrity"]["appRecognitionVerdict"] = json!("UNRECOGNIZED_VERSION");
    let refused = redeem_with_verdict(&issued, &sideloaded).await;
    assert_eq!(error_of(refused), (400, "invalid_client".to_owned()));
}

/// A device that does not meet Play's device integrity is refused.
#[tokio::test]
async fn a_device_without_device_integrity_is_refused() {
    let resources = server().await;
    let email = athlete(&resources).await;
    let issued = mobile_code(&resources, &email).await;

    let mut rooted = genuine_verdict(&issued.code);
    rooted["deviceIntegrity"]["deviceRecognitionVerdict"] = json!(["MEETS_BASIC_INTEGRITY"]);
    let (_logs, spans, guard) = capture_logs_and_spans();
    let refused = redeem_with_verdict(&issued, &rooted).await;
    drop(guard);
    assert_eq!(error_of(refused), (400, "invalid_client".to_owned()));
    assert_eq!(recorded(&spans, "app_attest"), ["refused"]);
}

/// A token Google refuses is refused, the span records `refused`, and the
/// code survives it.
#[tokio::test]
async fn a_token_google_refuses_is_refused_and_the_code_survives() {
    let resources = server().await;
    let email = athlete(&resources).await;
    let issued = mobile_code(&resources, &email).await;

    let (_logs, spans, guard) = capture_logs_and_spans();
    let refused = redeem_with(&issued, &[("play_integrity_token", REJECTED)]).await;
    drop(guard);
    assert_eq!(error_of(refused), (400, "invalid_client".to_owned()));
    assert_eq!(recorded(&spans, "app_attest"), ["refused"]);

    let retried = redeem_with_verdict(&issued, &genuine_verdict(&issued.code)).await;
    assert_eq!(retried.status(), 200, "{}", retried.body_text());
}

/// Google out of reach says nothing about the app: the token counts as
/// absent evidence, which is accepted while enforcement is off, the span
/// records `unavailable`, and the token itself is never written to the logs.
#[tokio::test]
async fn google_out_of_reach_signs_in_as_if_no_evidence_was_sent() {
    let resources = server().await;
    let email = athlete(&resources).await;
    let issued = mobile_code(&resources, &email).await;

    let (logs, spans, guard) = capture_logs_and_spans();
    let response = redeem_with(&issued, &[("play_integrity_token", UNREACHABLE)]).await;
    drop(guard);

    assert_eq!(response.status(), 200, "{}", response.body_text());
    assert_eq!(recorded(&spans, "app_attest"), ["unavailable"]);
    assert_eq!(
        named(
            &logs,
            "Play Integrity token not checked: Google could not be asked"
        )
        .len(),
        1
    );
    let leaked: Vec<_> = logs
        .lock()
        .unwrap()
        .iter()
        .filter(|line| {
            line.fields
                .values()
                .any(|value| value.contains(UNREACHABLE))
        })
        .map(|line| line.event.clone())
        .collect();
    assert!(leaked.is_empty(), "the token reached the logs: {leaked:?}");
}

/// A Play Integrity token comes alone: beside any App Attest field it is a
/// malformed request.
#[tokio::test]
async fn a_token_beside_app_attest_evidence_is_a_malformed_request() {
    let resources = server().await;
    let email = athlete(&resources).await;
    let issued = mobile_code(&resources, &email).await;
    let token = genuine_verdict(&issued.code).to_string();

    for evidence in [
        vec![
            ("play_integrity_token", token.as_str()),
            ("app_attest_key_id", "a-key"),
            ("app_attest_assertion", "an-assertion"),
        ],
        vec![
            ("play_integrity_token", token.as_str()),
            ("app_attest_key_id", "a-key"),
            ("app_attest_attestation", "an-attestation"),
        ],
    ] {
        let refused = redeem_with(&issued, &evidence).await;
        assert_eq!(error_of(refused), (400, "invalid_request".to_owned()));
    }

    let unspent = redeem_with_verdict(&issued, &genuine_verdict(&issued.code)).await;
    assert_eq!(unspent.status(), 200, "{}", unspent.body_text());
}

/// Only the mobile app attests; a Play Integrity token on the web app's
/// exchange is a malformed request.
#[tokio::test]
async fn the_web_app_never_sends_a_play_integrity_token() {
    let resources = server().await;
    let email = athlete(&resources).await;

    let issued = SignIn::new(&email, PASSWORD).issue_code(&resources).await;
    let refused = redeem_with_verdict(&issued, &genuine_verdict(&issued.code)).await;
    assert_eq!(error_of(refused), (400, "invalid_request".to_owned()));
}

/// Each decode is a metered call to Google: an invented code never costs
/// one. The exchange is refused as it would be without a token, and the span
/// records no evidence outcome.
#[tokio::test]
async fn an_unknown_code_never_reaches_google() {
    let (resources, decodes) = counting_server().await;
    let email = athlete(&resources).await;
    let issued = mobile_code(&resources, &email).await;
    let invented = IssuedCode {
        code: format!("invented-{}", Uuid::new_v4()),
        ..issued
    };

    let (_logs, spans, guard) = capture_logs_and_spans();
    let refused = redeem_with_verdict(&invented, &genuine_verdict(&invented.code)).await;
    drop(guard);

    assert_eq!(error_of(refused), (400, "invalid_grant".to_owned()));
    assert_eq!(decodes.load(Ordering::SeqCst), 0);
    assert!(recorded(&spans, "app_attest").is_empty());
}

/// A real code with a verifier that fails PKCE never reaches Google either.
#[tokio::test]
async fn a_code_whose_verifier_fails_pkce_never_reaches_google() {
    let (resources, decodes) = counting_server().await;
    let email = athlete(&resources).await;
    let issued = mobile_code(&resources, &email).await;
    let token = genuine_verdict(&issued.code).to_string();
    let wrong_verifier = "a".repeat(64);

    let response = AxumTestRequest::post("/oauth/token")
        .form(&[
            ("grant_type", "authorization_code"),
            ("client_id", issued.client_id),
            ("code", issued.code.as_str()),
            ("redirect_uri", issued.redirect_uri.as_str()),
            ("code_verifier", wrong_verifier.as_str()),
            ("play_integrity_token", token.as_str()),
        ])
        .send(issued.router.clone())
        .await;

    assert_eq!(error_of(response), (400, "invalid_grant".to_owned()));
    assert_eq!(decodes.load(Ordering::SeqCst), 0);
}
