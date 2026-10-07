// ABOUTME: The iOS app's App Attest evidence on its code exchange at /oauth/token (carnet#810)
// ABOUTME: Pins that evidence binds to the code, a refusal leaves the code unspent, the counter only moves forward, web never attests

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! The verification itself — Apple's chain, the nonce, the key id — is
//! pinned against a real iPhone attestation in `pierre-auth`'s unit tests.
//! These tests drive the token endpoint with assertions signed by a P-256
//! key standing in for the Secure Enclave, registered the way a verified
//! attestation registers one, so they pin what the endpoint does with
//! evidence: where it is accepted, what a refusal costs, and the counter.

mod common;
mod helpers;

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use chrono::Utc;
use common::{create_test_server_resources, create_test_user_with_email};
use helpers::axum_test::{AxumTestRequest, AxumTestResponse};
use helpers::first_party_sign_in::{FirstPartyClient, IssuedCode, SignIn};
use pierre_auth::oauth2_server::first_party::MOBILE_APP_ATTEST_APP_ID;
use pierre_core::models::{AppAttestEnvironment, AppAttestKey};
use pierre_mcp_server::mcp::resources::ServerContext;
use ring::rand::SystemRandom;
use ring::signature::{EcdsaKeyPair, KeyPair as _, ECDSA_P256_SHA256_ASN1_SIGNING};
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// `create_test_user_with_email`'s password.
const PASSWORD: &str = "password123";

/// An install of the iOS app: a P-256 key standing in for its Secure Enclave
/// key, signing assertions as Apple specifies them.
struct Install {
    key: EcdsaKeyPair,
    rng: SystemRandom,
}

impl Install {
    fn new() -> Self {
        let rng = SystemRandom::new();
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &rng).unwrap();
        let key = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, pkcs8.as_ref(), &rng)
            .unwrap();
        Self { key, rng }
    }

    /// The key id the app names its key by: base64 of the SHA-256 of the
    /// public key.
    fn key_id(&self) -> String {
        STANDARD.encode(Sha256::digest(self.key.public_key().as_ref()))
    }

    /// Register the key as a verified attestation does.
    async fn register(&self, resources: &ServerContext) {
        let now = Utc::now();
        let registered = resources
            .common
            .repos
            .app_attest_keys
            .register_key(&AppAttestKey {
                key_id: self.key_id(),
                public_key: self.key.public_key().as_ref().to_vec(),
                environment: AppAttestEnvironment::Production,
                sign_count: 0,
                created_at: now,
                last_used_at: now,
            })
            .await
            .unwrap();
        assert!(registered);
    }

    /// An assertion with `counter` over `client_data`, for `app_id`:
    /// `ECDSA-SHA256(SHA-256(authenticatorData ‖ SHA-256(clientData)))`.
    fn assert_for(&self, app_id: &str, counter: u32, client_data: &[u8]) -> String {
        let mut authenticator_data = Sha256::digest(app_id.as_bytes()).to_vec();
        authenticator_data.push(0x40);
        authenticator_data.extend_from_slice(&counter.to_be_bytes());
        let mut nonce = Sha256::new();
        nonce.update(&authenticator_data);
        nonce.update(Sha256::digest(client_data));
        let signature = self.key.sign(&self.rng, &nonce.finalize()).unwrap();
        STANDARD.encode(assertion_cbor(signature.as_ref(), &authenticator_data))
    }

    /// An assertion by this install over `code`, as the app signs one.
    fn assert(&self, counter: u32, code: &str) -> String {
        self.assert_for(MOBILE_APP_ATTEST_APP_ID, counter, code.as_bytes())
    }
}

/// `{"signature": h'…', "authenticatorData": h'…'}` in CBOR. Both byte
/// strings are under 256 bytes (a DER P-256 signature is at most 72, the
/// authenticator data 37), so each takes the one-byte length form `0x58 n`.
fn assertion_cbor(signature: &[u8], authenticator_data: &[u8]) -> Vec<u8> {
    let mut cbor = vec![0xa2];
    for (name, bytes) in [
        ("signature", signature),
        ("authenticatorData", authenticator_data),
    ] {
        cbor.push(0x60 | u8::try_from(name.len()).unwrap());
        cbor.extend_from_slice(name.as_bytes());
        cbor.push(0x58);
        cbor.push(u8::try_from(bytes.len()).unwrap());
        cbor.extend_from_slice(bytes);
    }
    cbor
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

fn error_of(response: AxumTestResponse) -> (u16, String) {
    let status = response.status();
    let body: Value = response.json();
    (
        status,
        body["error"].as_str().unwrap_or_default().to_owned(),
    )
}

async fn stored_counter(resources: &ServerContext, install: &Install) -> u32 {
    resources
        .common
        .repos
        .app_attest_keys
        .find_key(&install.key_id())
        .await
        .unwrap()
        .expect("the key is registered")
        .sign_count
}

/// A registered install signs in with an assertion over its code, and the
/// server stores the assertion's counter.
#[tokio::test]
async fn an_install_signs_in_with_an_assertion_over_its_code() {
    let resources = create_test_server_resources().await.unwrap();
    let email = athlete(&resources).await;
    let install = Install::new();
    install.register(&resources).await;

    let issued = mobile_code(&resources, &email).await;
    let assertion = install.assert(1, &issued.code);
    let response = redeem_with(
        &issued,
        &[
            ("app_attest_key_id", &install.key_id()),
            ("app_attest_assertion", &assertion),
        ],
    )
    .await;

    assert_eq!(response.status(), 200, "{}", response.body_text());
    let body: Value = response.json();
    assert_eq!(body["user"]["email"], email.as_str(), "{body}");
    assert_eq!(stored_counter(&resources, &install).await, 1);
}

/// The evidence binds to the code: an assertion signed over another code is
/// refused, and the refusal leaves this code unspent for a retry.
#[tokio::test]
async fn an_assertion_over_another_code_is_refused_and_the_code_survives() {
    let resources = create_test_server_resources().await.unwrap();
    let email = athlete(&resources).await;
    let install = Install::new();
    install.register(&resources).await;

    let issued = mobile_code(&resources, &email).await;
    let elsewhere = install.assert(1, "a-code-from-another-sign-in");
    let refused = redeem_with(
        &issued,
        &[
            ("app_attest_key_id", &install.key_id()),
            ("app_attest_assertion", &elsewhere),
        ],
    )
    .await;
    assert_eq!(error_of(refused), (400, "invalid_client".to_owned()));
    assert_eq!(stored_counter(&resources, &install).await, 0);

    let retried = redeem_with(
        &issued,
        &[
            ("app_attest_key_id", &install.key_id()),
            ("app_attest_assertion", &install.assert(1, &issued.code)),
        ],
    )
    .await;
    assert_eq!(retried.status(), 200, "{}", retried.body_text());
}

/// The counter only moves forward: an assertion that does not pass the
/// stored counter is refused, even over a fresh code.
#[tokio::test]
async fn an_assertion_that_does_not_advance_the_counter_is_refused() {
    let resources = create_test_server_resources().await.unwrap();
    let email = athlete(&resources).await;
    let install = Install::new();
    install.register(&resources).await;

    let first = mobile_code(&resources, &email).await;
    let signed_in = redeem_with(
        &first,
        &[
            ("app_attest_key_id", &install.key_id()),
            ("app_attest_assertion", &install.assert(5, &first.code)),
        ],
    )
    .await;
    assert_eq!(signed_in.status(), 200, "{}", signed_in.body_text());

    for counter in [5, 4] {
        let next = mobile_code(&resources, &email).await;
        let refused = redeem_with(
            &next,
            &[
                ("app_attest_key_id", &install.key_id()),
                ("app_attest_assertion", &install.assert(counter, &next.code)),
            ],
        )
        .await;
        assert_eq!(error_of(refused), (400, "invalid_client".to_owned()));
    }
    assert_eq!(stored_counter(&resources, &install).await, 5);
}

/// An assertion by a key this server never registered, or by another app,
/// is refused.
#[tokio::test]
async fn an_unregistered_key_or_another_app_is_refused() {
    let resources = create_test_server_resources().await.unwrap();
    let email = athlete(&resources).await;

    let stranger = Install::new();
    let issued = mobile_code(&resources, &email).await;
    let unregistered = redeem_with(
        &issued,
        &[
            ("app_attest_key_id", &stranger.key_id()),
            ("app_attest_assertion", &stranger.assert(1, &issued.code)),
        ],
    )
    .await;
    assert_eq!(error_of(unregistered), (400, "invalid_client".to_owned()));

    let install = Install::new();
    install.register(&resources).await;
    let other_app = install.assert_for("ABCDE12345.com.example.other", 1, issued.code.as_bytes());
    let refused = redeem_with(
        &issued,
        &[
            ("app_attest_key_id", &install.key_id()),
            ("app_attest_assertion", &other_app),
        ],
    )
    .await;
    assert_eq!(error_of(refused), (400, "invalid_client".to_owned()));
}

/// An attestation that is not Apple's is refused before the code is spent.
#[tokio::test]
async fn a_forged_attestation_is_refused_and_the_code_survives() {
    let resources = create_test_server_resources().await.unwrap();
    let email = athlete(&resources).await;
    let install = Install::new();

    let issued = mobile_code(&resources, &email).await;
    let forged = STANDARD.encode(b"not an attestation");
    let refused = redeem_with(
        &issued,
        &[
            ("app_attest_key_id", &install.key_id()),
            ("app_attest_attestation", &forged),
        ],
    )
    .await;
    assert_eq!(error_of(refused), (400, "invalid_client".to_owned()));
    assert!(resources
        .common
        .repos
        .app_attest_keys
        .find_key(&install.key_id())
        .await
        .unwrap()
        .is_none());

    let without_evidence = issued.redeem(None).await;
    assert_eq!(
        without_evidence.status(),
        200,
        "{}",
        without_evidence.body_text()
    );
}

/// Evidence comes as a key id with exactly one of attestation and assertion.
#[tokio::test]
async fn incomplete_evidence_is_a_malformed_request() {
    let resources = create_test_server_resources().await.unwrap();
    let email = athlete(&resources).await;
    let install = Install::new();
    let issued = mobile_code(&resources, &email).await;
    let assertion = install.assert(1, &issued.code);
    let key_id = install.key_id();

    for evidence in [
        vec![("app_attest_key_id", key_id.as_str())],
        vec![("app_attest_assertion", assertion.as_str())],
        vec![
            ("app_attest_key_id", key_id.as_str()),
            ("app_attest_assertion", assertion.as_str()),
            ("app_attest_attestation", assertion.as_str()),
        ],
    ] {
        let refused = redeem_with(&issued, &evidence).await;
        assert_eq!(error_of(refused), (400, "invalid_request".to_owned()));
    }
}

/// Only the mobile app attests; evidence on the web app's exchange is a
/// malformed request.
#[tokio::test]
async fn the_web_app_never_sends_evidence() {
    let resources = create_test_server_resources().await.unwrap();
    let email = athlete(&resources).await;
    let install = Install::new();
    install.register(&resources).await;

    let issued = SignIn::new(&email, PASSWORD).issue_code(&resources).await;
    let refused = redeem_with(
        &issued,
        &[
            ("app_attest_key_id", &install.key_id()),
            ("app_attest_assertion", &install.assert(1, &issued.code)),
        ],
    )
    .await;
    assert_eq!(error_of(refused), (400, "invalid_request".to_owned()));
}

/// A key registers once: a second attestation of the same key id is a replay,
/// refused, and the first record is kept, environment included.
#[tokio::test]
async fn a_key_registers_once_and_keeps_its_first_record() {
    let resources = create_test_server_resources().await.unwrap();
    let install = Install::new();
    install.register(&resources).await;

    let keys = &resources.common.repos.app_attest_keys;
    let now = Utc::now();
    let replayed = keys
        .register_key(&AppAttestKey {
            key_id: install.key_id(),
            public_key: vec![4; 65],
            environment: AppAttestEnvironment::Development,
            sign_count: 0,
            created_at: now,
            last_used_at: now,
        })
        .await
        .unwrap();
    assert!(!replayed);

    let stored = keys.find_key(&install.key_id()).await.unwrap().unwrap();
    assert_eq!(stored.public_key, install.key.public_key().as_ref());
    assert_eq!(stored.environment, AppAttestEnvironment::Production);
}
