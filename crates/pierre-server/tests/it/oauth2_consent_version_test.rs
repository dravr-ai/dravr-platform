// ABOUTME: An approval records the version and language of the consent text it was given under
// ABOUTME: A later authorization skips the screen only while that text is current; a changed or unrecorded text asks again
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! EFVP 2026-11-17, section 5.3 and decision 9.4a. The consent screen was
//! skipped whenever the user held a live grant for the same app and
//! permissions, whatever the screen said when they approved: the wording of
//! 2026-11-06, which first named injuries and where activities took place,
//! re-asked nobody. Each approval now records the text's version and its
//! language, and a text changed since, in that language, is asked again.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::connect_info::MockConnectInfo;
use base64::{engine::general_purpose, Engine as _};
use dravr_tronc::dates::Timestamp;
use pierre_auth::oauth2_server::client_registration::ClientRegistrationManager;
use pierre_auth::oauth2_server::models::ClientRegistrationRequest;
use pierre_auth::security::cookies::auth_cookie_name;
use pierre_contremaitre::consent_strings::KEY_CONSENT_EXPRESS;
use pierre_core::constants::oauth2_client_retention::MAX_PENDING_REGISTRATIONS;
use pierre_core::models::{OAuthClientGrant, User};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_mcp_server::mcp::multitenant::ProviderToolRouter;
use pierre_mcp_server::mcp::resources::ServerContext;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::common::{self, create_test_tenant};
use crate::helpers::axum_test::{AxumTestRequest, AxumTestResponse};
use crate::helpers::consent_form::{hidden_field, review_digest, with_grant_lines};

const REDIRECT: &str = "https://client.example.test/callback";
const VERIFIER: &str = "consent-version-pkce-verifier-0123456789-abcdefghijklmnop";

fn app(resources: &Arc<ServerContext>) -> axum::Router {
    ProviderToolRouter::build_http_app(resources)
        .layer(MockConnectInfo(SocketAddr::from(([127, 0, 0, 1], 41_660))))
}

fn challenge() -> String {
    general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(VERIFIER.as_bytes()))
}

async fn register(resources: &Arc<ServerContext>) -> String {
    ClientRegistrationManager::new(resources.common.repos.oauth2_server.clone())
        .register_client(
            ClientRegistrationRequest {
                redirect_uris: vec![REDIRECT.to_owned()],
                client_name: Some("Consent version client".to_owned()),
                client_uri: None,
                grant_types: None,
                response_types: None,
                scope: None,
            },
            MAX_PENDING_REGISTRATIONS,
        )
        .await
        .unwrap()
        .client_id
}

/// An athlete signed in to the web app, and their session cookie.
struct Athlete {
    user: User,
    cookie: String,
}

async fn athlete(resources: &Arc<ServerContext>, email: &str) -> Athlete {
    let (user, session) = create_test_tenant(resources, email).await.unwrap();
    Athlete {
        user,
        cookie: format!("{}={session}", auth_cookie_name()),
    }
}

/// The athlete's authorization of `client_id`, with a fresh `state`.
async fn authorize(
    resources: &Arc<ServerContext>,
    client_id: &str,
    who: &Athlete,
) -> AxumTestResponse {
    let uri = format!(
        "/oauth2/authorize?response_type=code&client_id={}&redirect_uri={}&state={}&code_challenge={}&code_challenge_method=S256",
        urlencoding::encode(client_id),
        urlencoding::encode(REDIRECT),
        Uuid::new_v4().simple(),
        challenge(),
    );
    AxumTestRequest::get(&uri)
        .header("cookie", &who.cookie)
        .send(app(resources))
        .await
}

/// Whether the authorization showed the consent screen (200), rather than
/// redirecting with a code (303).
async fn asks(resources: &Arc<ServerContext>, client_id: &str, who: &Athlete) -> bool {
    let response = authorize(resources, client_id, who).await;
    let status = response.status();
    let location = response.header("location").map(str::to_owned);
    let body = response.text();
    match status {
        200 => {
            assert!(
                body.contains("name=\"csrf_token\""),
                "the consent screen: {body}"
            );
            true
        }
        303 => {
            assert!(
                location.is_some_and(|location| location.contains("code=")),
                "a remembered approval redirects with a code"
            );
            false
        }
        other => panic!("unexpected authorization answer {other}: {body}"),
    }
}

/// The athlete approves `client_id` on the screen they are shown.
async fn approve(resources: &Arc<ServerContext>, client_id: &str, who: &Athlete) {
    let page = authorize(resources, client_id, who).await;
    assert_eq!(page.status(), 200, "the consent screen is shown");
    let html = page.text();
    let token = html
        .split("name=\"csrf_token\" value=\"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .unwrap()
        .to_owned();
    let challenge = challenge();
    let approved = AxumTestRequest::post("/oauth2/consent")
        .header("cookie", &who.cookie)
        .form(&[
            ("response_type", "code"),
            ("client_id", client_id),
            ("redirect_uri", REDIRECT),
            ("state", "approve-state"),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", "S256"),
            ("decision", "approve"),
            ("csrf_token", token.as_str()),
            ("review_digest", review_digest(&html).as_str()),
        ])
        .send(app(resources))
        .await;
    assert_eq!(approved.status(), 303, "{}", approved.text());
}

async fn grants(resources: &Arc<ServerContext>, who: &Athlete) -> Vec<OAuthClientGrant> {
    let repos = &resources.common.repos;
    let tenant = repos
        .tenants
        .first_for_user(who.user.id)
        .await
        .unwrap()
        .expect("the athlete has a tenant");
    repos
        .oauth2_server
        .list_client_grants(&who.user.id.to_string(), &tenant.to_string())
        .await
        .unwrap()
}

/// Reword the express-consent line in `locale`, as a contremaitre bundle
/// synced at runtime does.
fn reword(resources: &Arc<ServerContext>, locale: &str, text: &str) {
    resources.mcp.messaging_strings_registry.update(
        KEY_CONSENT_EXPRESS,
        locale,
        text.to_owned(),
        format!("reworded-{}", Uuid::new_v4().simple()),
    );
}

async fn resources() -> Arc<ServerContext> {
    common::init_server_config();
    common::create_test_server_resources().await.unwrap()
}

#[tokio::test]
async fn an_approval_records_the_version_and_language_it_was_given_under() {
    let resources = resources().await;
    let client_id = register(&resources).await;
    let who = athlete(&resources, "consent-version-record@example.test").await;

    approve(&resources, &client_id, &who).await;

    let recorded = grants(&resources, &who).await;
    assert_eq!(recorded.len(), 1, "bug: one approval, one grant");
    let version = recorded[0]
        .consent_version
        .as_deref()
        .expect("bug: the approval records the version of the text shown");
    assert_eq!(version.len(), 64, "a SHA-256 digest: {version}");
    assert_eq!(
        recorded[0].consent_locale.as_deref(),
        Some(who.user.locale.as_str()),
        "bug: and the language the screen was shown in"
    );
    assert!(
        !asks(&resources, &client_id, &who).await,
        "an approval under the current text is remembered"
    );
}

#[tokio::test]
async fn a_changed_consent_text_asks_again_and_a_new_approval_is_remembered() {
    let resources = resources().await;
    let client_id = register(&resources).await;
    let who = athlete(&resources, "consent-version-change@example.test").await;
    approve(&resources, &client_id, &who).await;
    let before = grants(&resources, &who).await[0].clone();

    reword(
        &resources,
        &who.user.locale,
        "Nouvelle formulation du consentement exprès.",
    );
    assert!(
        asks(&resources, &client_id, &who).await,
        "bug: a grant approved under older text is shown the screen again"
    );

    approve(&resources, &client_id, &who).await;
    let after = grants(&resources, &who).await;
    assert_eq!(after.len(), 1, "the same grant, approved again");
    assert_eq!(
        after[0].id, before.id,
        "bug: approving again keeps the same grant row"
    );
    assert_ne!(
        after[0].consent_version, before.consent_version,
        "bug: the grant now records the text it was approved under"
    );
    assert!(
        !asks(&resources, &client_id, &who).await,
        "bug: once approved under the new text it is not asked again"
    );
}

#[tokio::test]
async fn a_change_in_another_language_does_not_ask_again() {
    let resources = resources().await;
    let client_id = register(&resources).await;
    let who = athlete(&resources, "consent-version-other-lang@example.test").await;
    approve(&resources, &client_id, &who).await;
    let other = if who.user.locale == "en" { "fr" } else { "en" };

    reword(&resources, other, "Another wording of the express consent.");

    assert!(
        !asks(&resources, &client_id, &who).await,
        "only the language the athlete read their consent in is versioned for them"
    );
}

/// Every grant approved before versions were recorded is asked again once.
#[tokio::test]
async fn a_grant_recorded_without_a_version_asks_again() {
    let resources = resources().await;
    let client_id = register(&resources).await;
    let who = athlete(&resources, "consent-version-legacy@example.test").await;
    let tenant = resources
        .common
        .repos
        .tenants
        .first_for_user(who.user.id)
        .await
        .unwrap()
        .unwrap();
    resources
        .common
        .repos
        .oauth2_server
        .store_client_grant(&OAuthClientGrant {
            id: Uuid::new_v4().to_string(),
            user_id: who.user.id.to_string(),
            tenant_id: tenant.to_string(),
            client_id: client_id.clone(),
            scope: OAuthScope::render_granted(&OAuthScope::default_grant()),
            granted_at: Timestamp::now(),
            revoked_at: None,
            consent_version: None,
            consent_locale: None,
            authorization_details: None,
            expires_at: None,
        })
        .await
        .unwrap();

    assert!(
        asks(&resources, &client_id, &who).await,
        "bug: a grant whose consent text is unknown is shown the screen again"
    );
    approve(&resources, &client_id, &who).await;
    assert!(
        grants(&resources, &who).await[0].consent_version.is_some(),
        "the approval records its version on the grant it already had"
    );
    assert!(
        !asks(&resources, &client_id, &who).await,
        "bug: a grant approved under the current text is not asked again"
    );
}

/// A preset grant records the review digest of its screen: the text version
/// bound to the preset's terms, never the bare version a plain grant records.
/// A change to any line the picker shows asks it again too.
#[tokio::test]
async fn a_preset_grant_records_its_review_digest_and_a_picker_line_change_asks_again() {
    let resources = resources().await;
    with_grant_lines(&resources.mcp.messaging_strings_registry);
    let client_id = register(&resources).await;
    let who = athlete(&resources, "consent-version-preset@example.test").await;

    let screen = authorize(&resources, &client_id, &who).await;
    assert_eq!(screen.status(), 200);
    let page = screen.text();
    let plain_digest = hidden_field(&page, "review_digest_everything").unwrap();
    let digest = hidden_field(&page, "review_digest_training_coach").unwrap();
    assert_ne!(digest, plain_digest, "each choice is its own review");
    let token = resources
        .auth
        .csrf_manager
        .generate_token(who.user.id)
        .unwrap();
    let challenge = challenge();
    let approved = AxumTestRequest::post("/oauth2/consent")
        .header("cookie", &who.cookie)
        .form(&[
            ("response_type", "code"),
            ("client_id", client_id.as_str()),
            ("redirect_uri", REDIRECT),
            ("state", "preset-state"),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", "S256"),
            ("grant", "training_coach"),
            ("review_digest_training_coach", digest.as_str()),
            ("decision", "approve"),
            ("csrf_token", token.as_str()),
        ])
        .send(app(&resources))
        .await;
    assert_eq!(approved.status(), 303, "{}", approved.text());

    let recorded = grants(&resources, &who).await;
    assert_eq!(
        recorded[0].consent_version.as_deref(),
        Some(digest.as_str())
    );
    assert!(
        !asks(&resources, &client_id, &who).await,
        "a scope-only request is answered by the preset grant"
    );

    resources.mcp.messaging_strings_registry.update(
        "oauth.consent.grantSleepCoach",
        "en",
        "Sleep and recovery coach".to_owned(),
        "reworded-picker".to_owned(),
    );
    assert!(
        asks(&resources, &client_id, &who).await,
        "bug: a grant approved before a picker line changed is shown the screen again"
    );
}
