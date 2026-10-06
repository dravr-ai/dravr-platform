// ABOUTME: Refused passwords are counted per client address and per account named; full windows refuse before the check
// ABOUTME: Past either window the password grant answers 429 with Retry-After; the hosted form shares them; successes never count
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::connect_info::MockConnectInfo;
use axum::Router;
use common::{create_test_server_resources, create_test_user_with_email};
use helpers::axum_test::{AxumTestRequest, AxumTestResponse};
use pierre_core::constants::oauth_rate_limiting::{PASSWORD_LOGIN_ACCOUNT_RPM, PASSWORD_LOGIN_RPM};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_routes_auth::AuthRoutes;
use pierre_routes_identity::oauth2::{OAuth2Context, OAuth2Routes};
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

    /// The app's sign-in routes, every request arriving from `peer`.
    fn auth_routes(&self, peer: [u8; 4]) -> Router {
        AuthRoutes::routes(self.resources.auth_routes_context())
            .layer(MockConnectInfo(SocketAddr::from((peer, 40_804))))
    }

    /// The authorization server's routes, sharing the server's limiter as
    /// they do when mounted, every request arriving from `peer`.
    fn oauth2_routes(&self, peer: [u8; 4]) -> Router {
        let resources = &self.resources;
        let context = OAuth2Context {
            database: resources.agent.database.clone(),
            oauth2_server: resources.common.repos.oauth2_server.clone(),
            tenants: resources.common.repos.tenants.clone(),
            users: resources.common.repos.users.clone(),
            auth_manager: resources.auth.auth_manager.clone(),
            jwks_manager: resources.auth.jwks_manager.clone(),
            config: Arc::new(resources.common.config.oauth2_server.clone()),
            rate_limiter: resources.auth.oauth2_rate_limiter.clone(),
            refresh_token_expiry_days: 30,
            csrf_manager: resources.auth.csrf_manager.clone(),
            accounts: resources.oauth2_accounts(),
            google_sign_in: resources.oauth2_google_sign_in(),
        };
        OAuth2Routes::routes(context).layer(MockConnectInfo(SocketAddr::from((peer, 40_805))))
    }
}

async fn password_grant(routes: Router, email: &str, password: &str) -> AxumTestResponse {
    AxumTestRequest::post("/oauth/token")
        .form(&[
            ("grant_type", "password"),
            ("client_id", "dravr-web"),
            ("username", email),
            ("password", password),
        ])
        .send(routes)
        .await
}

fn assert_throttled(response: AxumTestResponse) {
    assert_eq!(response.status(), 429, "{}", response.body_text());
    let retry_after: u64 = response
        .header("retry-after")
        .expect("a Retry-After")
        .parse()
        .unwrap();
    assert!(retry_after >= 1);
    assert_eq!(response.json::<Value>()["error"], "too_many_requests");
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
        let routes = setup.auth_routes([203, 0, 113, u8::try_from(attempt).unwrap()]);
        let wrong = password_grant(routes, &email, &format!("guess-{attempt}")).await;
        assert_eq!(wrong.status(), 400, "guess {attempt} is checked");
    }

    let right = password_grant(setup.auth_routes([198, 51, 100, 1]), &email, PASSWORD).await;
    assert_throttled(right);
}

/// The account window is keyed by the email named, not by an account found:
/// an address nobody signed up with is metered exactly like one in use, so
/// the 429 cannot be used to tell them apart.
#[tokio::test]
async fn an_unknown_email_is_metered_like_a_known_one() {
    let setup = Setup::new().await;
    let unknown = format!("nobody-{}@example.com", Uuid::new_v4());

    for attempt in 0..PASSWORD_LOGIN_ACCOUNT_RPM {
        let routes = setup.auth_routes([203, 0, 113, u8::try_from(attempt).unwrap()]);
        let refused = password_grant(routes, &unknown, "guess").await;
        assert_eq!(refused.status(), 400, "attempt {attempt} is checked");
    }
    // A spelling variant of the same address shares the window.
    let variant = format!("  {}  ", unknown.to_uppercase());
    assert_throttled(password_grant(setup.auth_routes([198, 51, 100, 2]), &variant, "guess").await);
}

/// One address trying many accounts (credential stuffing) is refused past
/// the address's window, though no single account reached its own.
#[tokio::test]
async fn one_address_trying_many_accounts_is_refused_with_429() {
    let setup = Setup::new().await;
    let peer = [192, 0, 2, 7];

    for attempt in 0..PASSWORD_LOGIN_RPM {
        let email = format!("stuffed-{attempt}-{}@example.com", Uuid::new_v4());
        let refused = password_grant(setup.auth_routes(peer), &email, "leaked").await;
        assert_eq!(refused.status(), 400, "attempt {attempt} is checked");
    }

    let email = setup.athlete().await;
    assert_throttled(password_grant(setup.auth_routes(peer), &email, PASSWORD).await);

    // Another address is untouched by that window.
    let elsewhere = password_grant(setup.auth_routes([192, 0, 2, 8]), &email, PASSWORD).await;
    assert_eq!(elsewhere.status(), 200, "{}", elsewhere.body_text());
}

/// The hosted login form spends the same account window as the app's
/// sign-in: guesses made through one are refused on the other.
#[tokio::test]
async fn the_hosted_login_form_shares_the_account_window() {
    let setup = Setup::new().await;
    let email = setup.athlete().await;

    for attempt in 0..PASSWORD_LOGIN_ACCOUNT_RPM {
        let routes = setup.auth_routes([203, 0, 113, u8::try_from(attempt).unwrap()]);
        let wrong = password_grant(routes, &email, &format!("guess-{attempt}")).await;
        assert_eq!(wrong.status(), 400, "guess {attempt} is checked");
    }

    let form = AxumTestRequest::post("/oauth2/login")
        .form(&[
            ("email", email.as_str()),
            ("password", PASSWORD),
            ("client_id", "some-client"),
            ("redirect_uri", "https://client.example/cb"),
            ("response_type", "code"),
        ])
        .send(setup.oauth2_routes([198, 51, 100, 3]))
        .await;
    assert_eq!(form.status(), 429, "{}", form.body_text());
    assert!(form.header("retry-after").is_some());
    assert!(
        form.header("set-cookie").is_none(),
        "a refused sign-in never sets a session"
    );
}

/// A sign-in whose password is right never spends a window: athletes behind
/// one shared address (a gym, a household) sign in as often as they need.
#[tokio::test]
async fn successful_sign_ins_never_fill_a_window() {
    let setup = Setup::new().await;
    let email = setup.athlete().await;
    let peer = [192, 0, 2, 9];

    for attempt in 0..=PASSWORD_LOGIN_RPM.max(PASSWORD_LOGIN_ACCOUNT_RPM) {
        let signed_in = password_grant(setup.auth_routes(peer), &email, PASSWORD).await;
        assert_eq!(
            signed_in.status(),
            200,
            "sign-in {attempt}: {}",
            signed_in.body_text()
        );
    }
}
