// ABOUTME: Pins the gate on the turn-run route — which Google ID tokens a Cloud Tasks runner accepts and which it refuses
// ABOUTME: A test signer plays Google: an RSA key served as a JWK set, tokens minted for the matrix of refusals
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The turn-run route has no other gate: the backend's invoker IAM is off.
//! So the runner's refusals are the whole security of the route, and each
//! one is asserted here against a token that is right in every way but one.
//! The verifier is dravr-tronc's, configured by the runner with this
//! service's audience and its one allowed service account; what is pinned
//! here is that configuration and the answer each refusal becomes.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod helpers;

use std::time::Duration;

use pierre_core::errors::ErrorCode;
use pierre_mcp_server::services::turn_runner::CloudTasksRunner;

use crate::helpers::cloud_tasks_stub::{cloud_tasks_runner, SERVICE_ACCOUNT, TARGET};
use crate::helpers::google_token::{
    now_secs, serve_key_set, unreachable_key_set_url, GoogleClaims, TestSigner,
};

/// The verifier never reaches the queue, so no Cloud Tasks stand-in is served.
const UNUSED_API_BASE: &str = "http://127.0.0.1:1";

fn runner_reading(jwks_url: &str) -> CloudTasksRunner {
    cloud_tasks_runner(UNUSED_API_BASE, jwks_url, Duration::from_secs(1))
}

#[tokio::test]
async fn a_jwk_served_token_minted_for_this_audience_and_runner_is_accepted() {
    let signer = TestSigner::generate();
    let jwks = signer.serve_jwks().await;
    let runner = runner_reading(&jwks.url);
    let claims = GoogleClaims::cloud_tasks(TARGET, SERVICE_ACCOUNT);

    runner
        .verify_delivery(&signer.mint(&claims))
        .await
        .expect("a token for this service from the turn runner is accepted");

    // The bare-host issuer spelling Google also uses is the same identity.
    let mut bare = claims.clone();
    bare.iss = "accounts.google.com".to_owned();
    runner
        .verify_delivery(&signer.mint(&bare))
        .await
        .expect("the bare-host issuer is accepted");

    // Both came from one fetch of the key set.
    assert_eq!(jwks.fetches(), 1);
}

#[tokio::test]
async fn every_way_a_token_can_be_wrong_is_an_opaque_401() {
    let signer = TestSigner::generate();
    let jwks = signer.serve_jwks().await;
    let runner = runner_reading(&jwks.url);
    let good = GoogleClaims::cloud_tasks(TARGET, SERVICE_ACCOUNT);

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

    let cases: Vec<(&str, String)> = vec![
        ("another audience", signer.mint(&other_audience)),
        ("another issuer", signer.mint(&other_issuer)),
        ("another service account", signer.mint(&other_account)),
        ("an unverified email", signer.mint(&unverified)),
        ("no email at all", signer.mint(&no_email)),
        ("an expired token", signer.mint(&expired)),
        (
            "a key Google does not publish",
            signer.mint_with_kid(&good, "rotated-away"),
        ),
        ("garbage", "not.a.token".to_owned()),
    ];
    for (what, token) in cases {
        let err = runner
            .verify_delivery(&token)
            .await
            .expect_err(&format!("{what} must be refused"));
        assert_eq!(
            err.code,
            ErrorCode::AuthInvalid,
            "{what}: 401, got {:?}",
            err.code
        );
        // The caller learns that it failed, never which claim to adjust.
        assert_eq!(err.message, "Invalid identity token", "{what}");
    }
}

#[tokio::test]
async fn a_token_signed_by_another_key_under_a_published_kid_is_refused() {
    // The set publishes one signer's key; a token signed by a second key but
    // claiming the first's kid must fail the signature check.
    let published = TestSigner::generate();
    let impostor = TestSigner::generate();
    let jwks = published.serve_jwks().await;
    let runner = runner_reading(&jwks.url);

    let err = runner
        .verify_delivery(&impostor.mint_with_kid(
            &GoogleClaims::cloud_tasks(TARGET, SERVICE_ACCOUNT),
            &published.kid,
        ))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::AuthInvalid);
}

#[tokio::test]
async fn a_token_without_a_key_id_is_refused_before_any_fetch() {
    let signer = TestSigner::generate();
    let jwks = signer.serve_jwks().await;
    let runner = runner_reading(&jwks.url);

    let err = runner
        .verify_delivery(
            &signer.mint_without_kid(&GoogleClaims::cloud_tasks(TARGET, SERVICE_ACCOUNT)),
        )
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::AuthInvalid);
    assert_eq!(
        jwks.fetches(),
        0,
        "the refusal comes from the header, not from Google"
    );
}

#[tokio::test]
async fn unknown_key_ids_inside_the_refetch_interval_cost_one_fetch() {
    // An unauthenticated caller can send any kid it likes before anything
    // about the token is verified. The first unknown kid may refetch — that
    // is how a rotation shows up — but every later one inside the interval
    // is refused from memory, so made-up kids cannot turn each request into
    // an outbound fetch from Google.
    let signer = TestSigner::generate();
    let jwks = signer.serve_jwks().await;
    let runner = runner_reading(&jwks.url);
    let claims = GoogleClaims::cloud_tasks(TARGET, SERVICE_ACCOUNT);

    for attempt in 0..5 {
        let err = runner
            .verify_delivery(&signer.mint_with_kid(&claims, &format!("made-up-{attempt}")))
            .await
            .expect_err("an unknown kid is refused");
        assert_eq!(err.code, ErrorCode::AuthInvalid, "attempt {attempt}");
    }
    assert_eq!(jwks.fetches(), 1, "five unknown kids, one fetch");

    // The set that one fetch brought still serves the real key.
    runner
        .verify_delivery(&signer.mint(&claims))
        .await
        .expect("the published kid is served from the cached set");
    assert_eq!(jwks.fetches(), 1);
}

#[tokio::test]
async fn an_unreachable_key_set_is_an_internal_error_not_a_401() {
    // An outage fetching Google's keys is not somebody presenting a bad
    // token, and must not be recorded as one.
    let runner = runner_reading(&unreachable_key_set_url());
    let signer = TestSigner::generate();

    let err = runner
        .verify_delivery(&signer.mint(&GoogleClaims::cloud_tasks(TARGET, SERVICE_ACCOUNT)))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::InternalError, "{err:?}");
}

#[tokio::test]
async fn a_key_set_without_the_signers_key_refuses_its_tokens() {
    // Google publishes someone else's key: the signer's token names a kid
    // the set does not carry.
    let other = TestSigner::generate();
    let jwks = serve_key_set(vec![other.jwk()]).await;
    let runner = runner_reading(&jwks.url);
    let signer = TestSigner::generate();

    let err = runner
        .verify_delivery(&signer.mint(&GoogleClaims::cloud_tasks(TARGET, SERVICE_ACCOUNT)))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::AuthInvalid);
    assert_eq!(jwks.fetches(), 1);
}
