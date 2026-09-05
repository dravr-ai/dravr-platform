// ABOUTME: Pins the gate on the turn-run route — which Google ID tokens the verifier accepts and which it refuses
// ABOUTME: A test signer plays Google: openssl key and certificate, tokens minted for the matrix of refusals
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The turn-run route has no other gate: the backend's invoker IAM is off.
//! So the verifier's refusals are the whole security of the route, and each
//! one is asserted here against a token that is right in every way but one.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod helpers;

use std::collections::HashMap;

use pierre_auth::google_id_token::GoogleIdTokenVerifier;
use pierre_core::errors::ErrorCode;

use crate::helpers::google_token::{now_secs, serve_cert_map, GoogleClaims, TestSigner};

const AUDIENCE: &str = "https://dravr-mcp-server-api-123456.northamerica-northeast1.run.app";
const SA: &str = "dravr-app@dravr-dev.iam.gserviceaccount.com";

async fn verifier_for(signer: &TestSigner) -> GoogleIdTokenVerifier {
    GoogleIdTokenVerifier::with_certs_url(AUDIENCE, SA, signer.serve_certs().await)
}

#[tokio::test]
async fn a_token_minted_for_this_audience_and_runner_is_accepted() {
    let signer = TestSigner::generate();
    let verifier = verifier_for(&signer).await;
    let claims = GoogleClaims::cloud_tasks(AUDIENCE, SA);

    let accepted = verifier.verify(&signer.mint(&claims)).await.unwrap();
    assert_eq!(accepted.aud, AUDIENCE);
    assert_eq!(accepted.email.as_deref(), Some(SA));
    assert_eq!(accepted.sub, claims.sub);

    // The bare-host issuer spelling Google also uses is the same identity.
    let mut bare = claims.clone();
    bare.iss = "accounts.google.com".to_owned();
    assert!(verifier.verify(&signer.mint(&bare)).await.is_ok());
}

#[tokio::test]
async fn every_way_a_token_can_be_wrong_is_a_401() {
    let signer = TestSigner::generate();
    let verifier = verifier_for(&signer).await;
    let good = GoogleClaims::cloud_tasks(AUDIENCE, SA);

    let mut other_audience = good.clone();
    other_audience.aud = "https://someone-else.run.app".to_owned();
    let mut other_issuer = good.clone();
    other_issuer.iss = "https://accounts.example.com".to_owned();
    let mut other_account = good.clone();
    other_account.email = Some("intruder@dravr-dev.iam.gserviceaccount.com".to_owned());
    let mut unverified = good.clone();
    unverified.email_verified = Some(false);
    let mut no_email = good.clone();
    no_email.email = None;
    let mut expired = good.clone();
    expired.exp = now_secs() - 120;
    expired.iat = now_secs() - 3720;

    let cases: Vec<(&str, String, &str)> = vec![
        (
            "another audience",
            signer.mint(&other_audience),
            "Invalid token audience",
        ),
        (
            "another issuer",
            signer.mint(&other_issuer),
            "Invalid token issuer",
        ),
        (
            "another service account",
            signer.mint(&other_account),
            "Token subject is not the turn runner",
        ),
        (
            "an unverified email",
            signer.mint(&unverified),
            "Token subject is not the turn runner",
        ),
        (
            "no email at all",
            signer.mint(&no_email),
            "Token subject is not the turn runner",
        ),
        (
            "a key Google does not publish",
            signer.mint_with_kid(&good, "rotated-away"),
            "Unknown token signing key",
        ),
        ("garbage", "not.a.token".to_owned(), "Invalid token format"),
    ];
    for (what, token, expected) in cases {
        let err = verifier
            .verify(&token)
            .await
            .expect_err(&format!("{what} must be refused"));
        assert_eq!(
            err.code,
            ErrorCode::AuthInvalid,
            "{what}: 401, got {:?}",
            err.code
        );
        assert_eq!(err.message, expected, "{what}");
    }

    let err = verifier
        .verify(&signer.mint(&expired))
        .await
        .expect_err("an expired token must be refused");
    assert_eq!(err.code, ErrorCode::AuthExpired, "expired: {:?}", err.code);
}

#[tokio::test]
async fn a_token_signed_by_another_key_under_a_published_kid_is_refused() {
    // The map publishes one signer's certificate; a token signed by a second
    // key but claiming the first's kid must fail the signature check.
    let published = TestSigner::generate();
    let impostor = TestSigner::generate();
    let verifier = verifier_for(&published).await;

    let err = verifier
        .verify(&impostor.mint_with_kid(&GoogleClaims::cloud_tasks(AUDIENCE, SA), &published.kid))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::AuthInvalid);
    assert_eq!(err.message, "Invalid token");
}

#[tokio::test]
async fn a_token_without_a_key_id_is_refused_before_any_fetch() {
    // A map that would fail to parse if it were ever fetched: the refusal
    // has to come from the header, not from the certificate cache.
    let certs = serve_cert_map(HashMap::from([("kid".to_owned(), "garbage".to_owned())])).await;
    let verifier = GoogleIdTokenVerifier::with_certs_url(AUDIENCE, SA, certs);
    let signer = TestSigner::generate();

    let err = verifier
        .verify(&signer.mint_without_kid(&GoogleClaims::cloud_tasks(AUDIENCE, SA)))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::AuthInvalid);
    assert_eq!(err.message, "Token missing key ID");
}
