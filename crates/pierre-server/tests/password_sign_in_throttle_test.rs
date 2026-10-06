// ABOUTME: Refused passwords are counted per client address and per account named; full windows refuse before the check
// ABOUTME: Past either window the hosted login form answers 429 with Retry-After; successes never count; the password grant is gone
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! Since carnet#787 Dravr's apps sign in on the hosted login page
//! (`POST /oauth2/login`), so that is where a password is checked and where
//! the sign-in windows of carnet#804 are read and filled. `/oauth/token` no
//! longer takes a password at all.

mod common;
mod helpers;

use std::sync::Arc;

use common::{create_test_server_resources, create_test_user_with_email};
use helpers::axum_test::{AxumTestRequest, AxumTestResponse};
use helpers::first_party_sign_in::{
    first_party_router, fresh_state, is_redirect, submit_login, FirstPartyClient, Pkce, SignIn,
};
use pierre_core::constants::oauth_rate_limiting::{PASSWORD_LOGIN_ACCOUNT_RPM, PASSWORD_LOGIN_RPM};
use pierre_mcp_server::mcp::resources::ServerContext;
use serde_json::Value;
use uuid::Uuid;

/// The fixture user's password (`create_test_user_with_email`).
const PASSWORD: &str = "password123";

struct Setup {
    resources: Arc<ServerContext>,
}

impl Setup {
    async fn new() -> Self {
        Self {
            resources: create_test_server_resources().await.unwrap(),
        }
    }

    async fn athlete(&self) -> String {
        let email = format!("athlete-{}@example.com", Uuid::new_v4());
        create_test_user_with_email(&self.resources.agent.database, &email)
            .await
            .unwrap();
        email
    }

    /// The hosted login form, as Dravr's web app posts it, from `peer`.
    async fn sign_in(&self, peer: [u8; 4], email: &str, password: &str) -> AxumTestResponse {
        let client = FirstPartyClient::Web;
        let pkce = Pkce::generate();
        submit_login(
            first_party_router(&self.resources, peer),
            client.client_id(),
            &client.redirect_uri(&self.resources.common.config.oauth2_server),
            &pkce.challenge,
            &fresh_state(),
            email,
            password,
        )
        .await
    }
}

/// A password the form checked and refused: the generic page, no session.
fn assert_checked_and_refused(response: &AxumTestResponse, attempt: u32) {
    assert_eq!(
        response.status(),
        401,
        "attempt {attempt} is checked: {}",
        response.body_text()
    );
    assert!(response.header("set-cookie").is_none());
}

/// A sign-in the form admitted: the redirect back to `/oauth2/authorize`
/// with the authorization server's session.
fn assert_signed_in(response: &AxumTestResponse, attempt: u32) {
    assert!(
        is_redirect(response),
        "sign-in {attempt}: {} {}",
        response.status(),
        response.body_text()
    );
    assert!(response.header("set-cookie").is_some(), "sign-in {attempt}");
}

/// Refused unchecked: 429, a Retry-After, no session.
fn assert_throttled(response: &AxumTestResponse) {
    assert_eq!(response.status(), 429, "{}", response.body_text());
    let retry_after: u64 = response
        .header("retry-after")
        .expect("a Retry-After")
        .parse()
        .unwrap();
    assert!(retry_after >= 1);
    assert!(
        response.header("set-cookie").is_none(),
        "a refused sign-in never sets a session"
    );
}

/// Every refused password naming an account spends its window, from any
/// address: past it even the right password is refused with 429, so a
/// refusal never says whether a guess was right.
#[tokio::test]
async fn guesses_at_one_account_past_its_window_are_refused_with_429() {
    let setup = Setup::new().await;
    let email = setup.athlete().await;

    for attempt in 0..PASSWORD_LOGIN_ACCOUNT_RPM {
        // A fresh address per guess: the account's window, not the address's.
        let peer = [203, 0, 113, u8::try_from(attempt).unwrap()];
        let wrong = setup
            .sign_in(peer, &email, &format!("guess-{attempt}"))
            .await;
        assert_checked_and_refused(&wrong, attempt);
    }

    assert_throttled(&setup.sign_in([198, 51, 100, 1], &email, PASSWORD).await);
}

/// The account window is keyed by the email named, not by an account found:
/// an address nobody signed up with is metered exactly like one in use, so
/// the 429 cannot be used to tell them apart.
#[tokio::test]
async fn an_unknown_email_is_metered_like_a_known_one() {
    let setup = Setup::new().await;
    let unknown = format!("nobody-{}@example.com", Uuid::new_v4());

    for attempt in 0..PASSWORD_LOGIN_ACCOUNT_RPM {
        let peer = [203, 0, 113, u8::try_from(attempt).unwrap()];
        let refused = setup.sign_in(peer, &unknown, "guess").await;
        assert_checked_and_refused(&refused, attempt);
    }
    // A spelling variant of the same address shares the window.
    let variant = format!("  {}  ", unknown.to_uppercase());
    assert_throttled(&setup.sign_in([198, 51, 100, 2], &variant, "guess").await);
}

/// One address trying many accounts (credential stuffing) is refused past
/// the address's window, though no single account reached its own.
#[tokio::test]
async fn one_address_trying_many_accounts_is_refused_with_429() {
    let setup = Setup::new().await;
    let peer = [192, 0, 2, 7];

    for attempt in 0..PASSWORD_LOGIN_RPM {
        let email = format!("stuffed-{attempt}-{}@example.com", Uuid::new_v4());
        let refused = setup.sign_in(peer, &email, "leaked").await;
        assert_checked_and_refused(&refused, attempt);
    }

    let email = setup.athlete().await;
    assert_throttled(&setup.sign_in(peer, &email, PASSWORD).await);

    // Another address is untouched by that window.
    let elsewhere = setup.sign_in([192, 0, 2, 8], &email, PASSWORD).await;
    assert_signed_in(&elsewhere, 0);
}

/// The windows guard the whole sign-in, not just the form: an athlete whose
/// account window is full cannot finish the apps' flow either, and no code
/// reaches the app.
#[tokio::test]
async fn a_full_window_stops_the_apps_sign_in_before_a_code_is_issued() {
    let setup = Setup::new().await;
    let email = setup.athlete().await;

    for attempt in 0..PASSWORD_LOGIN_ACCOUNT_RPM {
        let peer = [203, 0, 113, u8::try_from(attempt).unwrap()];
        let wrong = setup
            .sign_in(peer, &email, &format!("guess-{attempt}"))
            .await;
        assert_checked_and_refused(&wrong, attempt);
    }

    let refused = SignIn::new(&email, PASSWORD)
        .client(FirstPartyClient::Mobile)
        .arriving_from([198, 51, 100, 3])
        .run(&setup.resources)
        .await
        .login_refused();
    assert_throttled(&refused);
}

/// A sign-in whose password is right never spends a window: athletes behind
/// one shared address (a gym, a household) sign in as often as they need.
#[tokio::test]
async fn successful_sign_ins_never_fill_a_window() {
    let setup = Setup::new().await;
    let email = setup.athlete().await;
    let peer = [192, 0, 2, 9];

    for attempt in 0..=PASSWORD_LOGIN_RPM.max(PASSWORD_LOGIN_ACCOUNT_RPM) {
        assert_signed_in(&setup.sign_in(peer, &email, PASSWORD).await, attempt);
    }
}

/// `/oauth/token` no longer checks a password, so it has nothing to meter:
/// the password grant is refused as unsupported, never with a 429, and
/// never fills a window the hosted form reads.
#[tokio::test]
async fn the_password_grant_is_gone_and_spends_no_window() {
    let setup = Setup::new().await;
    let email = setup.athlete().await;
    let peer = [192, 0, 2, 10];

    for _ in 0..=PASSWORD_LOGIN_RPM.max(PASSWORD_LOGIN_ACCOUNT_RPM) {
        let response = AxumTestRequest::post("/oauth/token")
            .form(&[
                ("grant_type", "password"),
                ("client_id", "dravr-web"),
                ("username", email.as_str()),
                ("password", "a-wrong-guess"),
            ])
            .send(first_party_router(&setup.resources, peer))
            .await;
        assert_eq!(response.status(), 400, "{}", response.body_text());
        assert_eq!(response.json::<Value>()["error"], "unsupported_grant_type");
    }

    assert_signed_in(&setup.sign_in(peer, &email, PASSWORD).await, 0);
}
