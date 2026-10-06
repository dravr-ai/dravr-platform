// ABOUTME: Dravr's own apps sign in with authorization code + PKCE through the hosted login page (carnet#787)
// ABOUTME: Pins the session the code buys, the mandatory verifier, single use, the redirect policy and the consent skip's limits
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! The apps used to post the athlete's password to `/oauth/token` (the
//! password grant, which RFC 9700 §2.4 forbids). They now send the athlete to
//! the hosted login page and redeem the authorization code it returns with
//! their PKCE verifier. Consent is skipped for them, which is only safe while
//! their codes go nowhere but the deployment's own callbacks and only the
//! holder of the verifier can redeem one — so those are pinned here, as is
//! that the skip never reaches a third-party client.

mod common;
mod helpers;

use common::{
    create_test_server_resources, create_test_server_resources_with_config,
    create_test_user_with_email, test_server_config,
};
use helpers::axum_test::{AxumTestRequest, AxumTestResponse};
use helpers::first_party_sign_in::{
    authorize_uri, callback_param, first_party_router, fresh_state, is_redirect, location, redeem,
    session_cookie, submit_login, FirstPartyClient, Pkce, SignIn, DEFAULT_PEER,
};
use pierre_auth::oauth2_server::client_registration::ClientRegistrationManager;
use pierre_auth::oauth2_server::first_party::{
    MOBILE_CALLBACK_URI, MOBILE_CLIENT_ID, WEB_CLIENT_ID,
};
use pierre_auth::oauth2_server::models::ClientRegistrationRequest;
use pierre_auth::security::cookies::auth_cookie_name;
use pierre_core::constants::oauth2_client_retention::MAX_PENDING_REGISTRATIONS;
use pierre_core::models::UserStatus;
use pierre_mcp_server::mcp::resources::ServerContext;
use serde_json::Value;
use uuid::Uuid;

/// `create_test_user_with_email`'s password.
const PASSWORD: &str = "password123";

async fn athlete(resources: &ServerContext) -> String {
    let email = format!("athlete-{}@example.com", Uuid::new_v4());
    create_test_user_with_email(&resources.agent.database, &email)
        .await
        .unwrap();
    email
}

async fn athlete_with_status(resources: &ServerContext, status: UserStatus) -> (Uuid, String) {
    let email = format!("athlete-{}@example.com", Uuid::new_v4());
    let (user_id, _) = create_test_user_with_email(&resources.agent.database, &email)
        .await
        .unwrap();
    resources
        .common
        .repos
        .users
        .update_status(user_id, status, None)
        .await
        .unwrap();
    (user_id, email)
}

fn error_of(response: AxumTestResponse) -> (u16, String) {
    let status = response.status();
    let body: Value = response.json();
    (
        status,
        body["error"].as_str().unwrap_or_default().to_owned(),
    )
}

/// `GET /oauth2/authorize` for `client_id` at `redirect_uri`, signed out.
async fn authorize_signed_out(
    resources: &ServerContext,
    client_id: &str,
    redirect_uri: &str,
) -> AxumTestResponse {
    let pkce = Pkce::generate();
    AxumTestRequest::get(&authorize_uri(
        client_id,
        redirect_uri,
        &pkce.challenge,
        &fresh_state(),
    ))
    .send(first_party_router(resources, DEFAULT_PEER))
    .await
}

/// `GET /oauth2/authorize` for `client_id` at `redirect_uri` carrying the
/// authorization server's session for `user_id`, as the hosted login sets it.
async fn authorize_signed_in(
    resources: &ServerContext,
    user_id: Uuid,
    client_id: &str,
    redirect_uri: &str,
) -> AxumTestResponse {
    let pkce = Pkce::generate();
    let uri = authorize_uri(client_id, redirect_uri, &pkce.challenge, &fresh_state());
    authorize_with_session(resources, user_id, &uri).await
}

/// The authorization server's session for `user_id`, as a `Cookie` value.
async fn session_for(resources: &ServerContext, user_id: Uuid) -> String {
    let user = resources
        .common
        .repos
        .users
        .get_global(user_id)
        .await
        .unwrap()
        .unwrap();
    let jwt = resources
        .auth
        .auth_manager
        .generate_authorization_session_token(&user, &resources.auth.jwks_manager, None)
        .unwrap();
    format!("pierre_session={jwt}")
}

/// `GET uri` carrying the authorization server's session for `user_id`.
async fn authorize_with_session(
    resources: &ServerContext,
    user_id: Uuid,
    uri: &str,
) -> AxumTestResponse {
    AxumTestRequest::get(uri)
        .header("cookie", &session_for(resources, user_id).await)
        .send(first_party_router(resources, DEFAULT_PEER))
        .await
}

/// Assert an `/oauth2/authorize` answer sent no code anywhere: the error
/// page, never a redirect to `redirect_uri`.
fn assert_no_code_sent(response: &AxumTestResponse, redirect_uri: &str) {
    assert!(
        !is_redirect(response),
        "{redirect_uri}: {} {:?}",
        response.status(),
        response.header("location")
    );
    assert_eq!(response.status(), 400, "{}", response.body_text());
    assert!(
        response.body_text().contains("Invalid redirect_uri"),
        "{}",
        response.body_text()
    );
}

// ── The session a code buys ──────────────────────────────────────────────────

/// The web app gets the session the password grant used to mint: the JWT,
/// the CSRF token, the user, and the auth and CSRF cookies — and no refresh
/// token, which it never asked for.
#[tokio::test]
async fn the_web_app_signs_in_with_cookies_csrf_and_the_user() {
    let resources = create_test_server_resources().await.unwrap();
    let email = athlete(&resources).await;

    let response = SignIn::new(&email, PASSWORD).run(&resources).await.token();
    assert_eq!(response.status(), 200, "{}", response.body_text());

    let cookies = response.header_all("set-cookie").join("\n");
    assert!(
        cookies.contains(&format!("{}=", auth_cookie_name())),
        "{cookies}"
    );
    assert!(cookies.contains("csrf_token="), "{cookies}");

    let body: Value = response.json();
    assert!(body["access_token"].is_string(), "{body}");
    assert_eq!(body["token_type"], "Bearer", "{body}");
    assert!(body["csrf_token"].is_string(), "{body}");
    assert_eq!(body["user"]["email"], email.as_str(), "{body}");
    assert_eq!(body["user"]["user_status"], "active", "{body}");
    assert!(body.get("refresh_token").is_none(), "{body}");

    // The access token is a first-party session the API accepts.
    let me = AxumTestRequest::get("/api/auth/session")
        .header(
            "authorization",
            &format!("Bearer {}", body["access_token"].as_str().unwrap()),
        )
        .send(first_party_router(&resources, DEFAULT_PEER))
        .await;
    assert_eq!(me.status(), 200, "{}", me.body_text());
}

/// The mobile app gets a refresh token only when it asks for one, and the
/// refresh grant still exchanges it for a fresh session.
#[tokio::test]
async fn the_mobile_app_gets_a_refresh_token_only_with_offline_access() {
    let resources = create_test_server_resources().await.unwrap();
    let email = athlete(&resources).await;

    let without = SignIn::new(&email, PASSWORD)
        .client(FirstPartyClient::Mobile)
        .run(&resources)
        .await
        .signed_in();
    assert!(without["access_token"].is_string(), "{without}");
    assert!(without.get("refresh_token").is_none(), "{without}");

    let with = SignIn::new(&email, PASSWORD)
        .client(FirstPartyClient::Mobile)
        .offline_access()
        .run(&resources)
        .await
        .signed_in();
    let refresh_token = with["refresh_token"]
        .as_str()
        .expect("offline_access carries a refresh token");

    let refreshed = AxumTestRequest::post("/oauth/token")
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
        ])
        .send(first_party_router(&resources, DEFAULT_PEER))
        .await;
    assert_eq!(refreshed.status(), 200, "{}", refreshed.body_text());
    let refreshed: Value = refreshed.json();
    assert!(refreshed["access_token"].is_string(), "{refreshed}");
    assert_eq!(refreshed["user"]["email"], email.as_str(), "{refreshed}");
    assert_ne!(refreshed["refresh_token"], with["refresh_token"]);
}

/// A pending account signs in, so the app can show it the approval it
/// awaits; the session says so.
#[tokio::test]
async fn a_pending_account_signs_in_and_is_told_it_is_pending() {
    let resources = create_test_server_resources().await.unwrap();
    let (_, email) = athlete_with_status(&resources, UserStatus::Pending).await;

    let body = SignIn::new(&email, PASSWORD)
        .run(&resources)
        .await
        .signed_in();
    assert_eq!(body["user"]["user_status"], "pending", "{body}");
}

/// A suspended account is refused at the hosted form — told why, given no
/// session — and, holding a session from before, no code at
/// `/oauth2/authorize` either.
#[tokio::test]
async fn a_suspended_account_is_refused_and_no_code_is_issued() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, email) = athlete_with_status(&resources, UserStatus::Suspended).await;

    let refused = SignIn::new(&email, PASSWORD)
        .run(&resources)
        .await
        .login_refused();
    assert!(refused.header("set-cookie").is_none());
    assert!(
        refused.body_text().contains("suspended"),
        "{}",
        refused.body_text()
    );

    let redirect_uri = FirstPartyClient::Web.redirect_uri(&resources.common.config.oauth2_server);
    let authorized = authorize_signed_in(&resources, user_id, WEB_CLIENT_ID, &redirect_uri).await;
    assert!(
        !authorized
            .header("location")
            .is_some_and(|location| location.contains("code=")),
        "{:?}",
        authorized.header("location")
    );
    assert!(
        authorized.body_text().contains("suspended"),
        "{}",
        authorized.body_text()
    );
}

// ── Redeeming the code ───────────────────────────────────────────────────────

/// The password grant is not served, to anyone.
#[tokio::test]
async fn the_password_grant_is_unsupported() {
    let resources = create_test_server_resources().await.unwrap();
    let email = athlete(&resources).await;

    let response = AxumTestRequest::post("/oauth/token")
        .form(&[
            ("grant_type", "password"),
            ("client_id", WEB_CLIENT_ID),
            ("username", email.as_str()),
            ("password", PASSWORD),
        ])
        .send(first_party_router(&resources, DEFAULT_PEER))
        .await;
    let body_text = response.body_text();
    assert_eq!(
        error_of(response),
        (400, "unsupported_grant_type".to_owned()),
        "{body_text}"
    );
}

/// The apps are public clients: the verifier is their only proof, so a
/// code redeemed without one is refused — and is not spent by the attempt.
#[tokio::test]
async fn a_code_without_its_verifier_is_invalid_request() {
    let resources = create_test_server_resources().await.unwrap();
    let email = athlete(&resources).await;
    let issued = SignIn::new(&email, PASSWORD).issue_code(&resources).await;

    let refused = redeem(
        issued.router.clone(),
        issued.client_id,
        &issued.code,
        &issued.redirect_uri,
        None,
        None,
    )
    .await;
    assert_eq!(error_of(refused), (400, "invalid_request".to_owned()));

    let redeemed = issued.redeem(None).await;
    assert_eq!(redeemed.status(), 200, "{}", redeemed.body_text());
}

/// A verifier that is not the one the challenge was made from is refused:
/// whoever intercepted the code cannot redeem it.
#[tokio::test]
async fn a_code_with_the_wrong_verifier_is_invalid_grant() {
    let resources = create_test_server_resources().await.unwrap();
    let email = athlete(&resources).await;
    let issued = SignIn::new(&email, PASSWORD).issue_code(&resources).await;

    let stolen = redeem(
        issued.router.clone(),
        issued.client_id,
        &issued.code,
        &issued.redirect_uri,
        Some(&Pkce::generate().verifier),
        None,
    )
    .await;
    assert_eq!(error_of(stolen), (400, "invalid_grant".to_owned()));

    // The failed attempt spent the code: the right verifier is now too late.
    let late = issued.redeem(None).await;
    assert_eq!(error_of(late), (400, "invalid_grant".to_owned()));
}

/// A code is redeemed once.
#[tokio::test]
async fn a_code_redeemed_twice_is_invalid_grant() {
    let resources = create_test_server_resources().await.unwrap();
    let email = athlete(&resources).await;
    let issued = SignIn::new(&email, PASSWORD).issue_code(&resources).await;

    let first = issued.redeem(None).await;
    assert_eq!(first.status(), 200, "{}", first.body_text());

    let replay = issued.redeem(None).await;
    assert_eq!(error_of(replay), (400, "invalid_grant".to_owned()));
}

/// A code is bound to the app and the callback it was issued to.
#[tokio::test]
async fn a_code_redeemed_by_the_other_app_or_at_another_callback_is_invalid_grant() {
    let resources = create_test_server_resources().await.unwrap();
    let email = athlete(&resources).await;

    let issued = SignIn::new(&email, PASSWORD).issue_code(&resources).await;
    let as_mobile = redeem(
        issued.router.clone(),
        MOBILE_CLIENT_ID,
        &issued.code,
        &issued.redirect_uri,
        Some(&issued.verifier),
        None,
    )
    .await;
    assert_eq!(error_of(as_mobile), (400, "invalid_grant".to_owned()));

    let issued = SignIn::new(&email, PASSWORD).issue_code(&resources).await;
    let elsewhere = redeem(
        issued.router.clone(),
        issued.client_id,
        &issued.code,
        MOBILE_CALLBACK_URI,
        Some(&issued.verifier),
        None,
    )
    .await;
    assert_eq!(error_of(elsewhere), (400, "invalid_grant".to_owned()));
}

/// The confidential-client token endpoint refuses the apps' ids: they have
/// no secret, and their codes buy a first-party session only at
/// `/oauth/token`.
#[tokio::test]
async fn the_third_party_token_endpoint_refuses_the_first_party_ids() {
    let resources = create_test_server_resources().await.unwrap();
    let email = athlete(&resources).await;

    for client_id in [WEB_CLIENT_ID, MOBILE_CLIENT_ID] {
        let issued = SignIn::new(&email, PASSWORD).issue_code(&resources).await;
        let response = AxumTestRequest::post("/oauth2/token")
            .form(&[
                ("grant_type", "authorization_code"),
                ("client_id", client_id),
                ("client_secret", "!"),
                ("code", issued.code.as_str()),
                ("redirect_uri", issued.redirect_uri.as_str()),
                ("code_verifier", issued.verifier.as_str()),
            ])
            .send(issued.router.clone())
            .await;
        let body_text = response.body_text();
        let (_, error) = error_of(response);
        assert_eq!(error, "invalid_client", "{client_id}: {body_text}");
        assert!(!body_text.contains("access_token"), "{body_text}");
    }
}

// ── Where a code may go ──────────────────────────────────────────────────────

/// Consent is skipped for the apps, so their codes go only to this
/// deployment's callbacks. Anything else is refused at `/oauth2/authorize`
/// before the athlete is asked to sign in, and — signed in — no code is
/// minted for it.
#[tokio::test]
async fn a_redirect_outside_the_policy_is_refused_and_gets_no_code() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = athlete_with_status(&resources, UserStatus::Active).await;

    let refused = [
        (WEB_CLIENT_ID, "https://evil.example/auth/callback"),
        (WEB_CLIENT_ID, "http://localhost:8081/auth/callback/x"),
        (
            WEB_CLIENT_ID,
            "http://localhost:8081.evil.example/auth/callback",
        ),
        (WEB_CLIENT_ID, MOBILE_CALLBACK_URI),
        (MOBILE_CLIENT_ID, "https://app.dravr.ai/auth/callback"),
        (MOBILE_CLIENT_ID, "http://localhost:8081/auth/callback"),
        (MOBILE_CLIENT_ID, "exp://192.168.1.20:8082/--/auth/callback"),
    ];
    for (client_id, redirect_uri) in refused {
        let signed_out = authorize_signed_out(&resources, client_id, redirect_uri).await;
        assert_no_code_sent(&signed_out, redirect_uri);

        let signed_in = authorize_signed_in(&resources, user_id, client_id, redirect_uri).await;
        assert_no_code_sent(&signed_in, redirect_uri);
    }
}

/// A development build in Expo Go may sign in only on a server that opted
/// in: there the `exp://` callback receives its code.
#[tokio::test]
async fn an_expo_go_callback_is_accepted_when_the_server_opts_in() {
    let mut config = test_server_config();
    config.oauth2_server.first_party_redirects.allow_expo_go = true;
    let resources = create_test_server_resources_with_config(config)
        .await
        .unwrap();
    let email = athlete(&resources).await;

    let body = SignIn::new(&email, PASSWORD)
        .client(FirstPartyClient::Mobile)
        .redirect_uri("exp://192.168.1.20:8082/--/auth/callback")
        .run(&resources)
        .await
        .signed_in();
    assert_eq!(body["user"]["email"], email.as_str(), "{body}");

    // The opt-in admits Expo Go's callback shape only.
    let smuggled = authorize_signed_out(
        &resources,
        MOBILE_CLIENT_ID,
        "exp://evil.example/x/--/auth/callback",
    )
    .await;
    assert_no_code_sent(&smuggled, "exp://evil.example/x/--/auth/callback");
}

/// The consent skip is the apps' alone: a registered integration signed in
/// on the same hosted page still gets the consent screen, and no code.
#[tokio::test]
async fn a_third_party_client_still_gets_the_consent_screen() {
    let resources = create_test_server_resources().await.unwrap();
    let email = athlete(&resources).await;
    let redirect_uri = "https://integration.example/callback";
    let client_id = ClientRegistrationManager::new(resources.common.repos.oauth2_server.clone())
        .register_client(
            ClientRegistrationRequest {
                redirect_uris: vec![redirect_uri.to_owned()],
                client_name: Some("An integration".to_owned()),
                client_uri: None,
                grant_types: None,
                response_types: None,
                scope: None,
            },
            MAX_PENDING_REGISTRATIONS,
        )
        .await
        .unwrap()
        .client_id;

    let router = first_party_router(&resources, DEFAULT_PEER);
    let pkce = Pkce::generate();
    let state = fresh_state();
    let login = submit_login(
        router.clone(),
        &client_id,
        redirect_uri,
        &pkce.challenge,
        &state,
        &email,
        PASSWORD,
    )
    .await;
    let resume = location(&login);
    assert!(resume.starts_with("/oauth2/authorize?"), "{resume}");

    let authorized = AxumTestRequest::get(&resume)
        .header("cookie", &session_cookie(&login))
        .send(router)
        .await;
    assert_eq!(authorized.status(), 200, "{}", authorized.body_text());
    assert!(authorized.header("location").is_none());
    let page = authorized.body_text();
    assert!(page.contains("/oauth2/consent"), "{page}");
    assert!(page.contains("An integration"), "{page}");
    assert!(
        callback_param(&page, redirect_uri, "code").is_none(),
        "no code before consent"
    );
}

/// The web app's callback is honoured on every configured origin, so a
/// deployment whose frontend and API differ signs in on the frontend's.
#[tokio::test]
async fn the_web_callback_follows_the_configured_origins() {
    let mut config = test_server_config();
    config.oauth2_server.first_party_redirects.web_origins = vec![
        "https://app.example.test".to_owned(),
        "http://localhost:8081".to_owned(),
    ];
    let resources = create_test_server_resources_with_config(config)
        .await
        .unwrap();
    let email = athlete(&resources).await;

    for redirect_uri in [
        "https://app.example.test/auth/callback",
        "http://localhost:8081/auth/callback",
    ] {
        let body = SignIn::new(&email, PASSWORD)
            .redirect_uri(redirect_uri)
            .run(&resources)
            .await
            .signed_in();
        assert_eq!(body["user"]["email"], email.as_str(), "{redirect_uri}");
    }
}

// ── prompt=login ────────────────────────────────────────────────────────────

/// Without `prompt`, a session the authorization server already holds signs
/// the athlete in to the app directly: the code goes to the callback.
#[tokio::test]
async fn an_existing_session_issues_a_code_without_prompt() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = athlete_with_status(&resources, UserStatus::Active).await;
    let redirect_uri = FirstPartyClient::Web.redirect_uri(&resources.common.config.oauth2_server);

    let authorized = authorize_signed_in(&resources, user_id, WEB_CLIENT_ID, &redirect_uri).await;
    assert!(is_redirect(&authorized), "{}", authorized.body_text());
    assert!(
        callback_param(&location(&authorized), &redirect_uri, "code").is_some(),
        "{}",
        location(&authorized)
    );
}

/// The apps always send `prompt=login`: a session left on the
/// authorization server (another athlete on a shared browser) is not used —
/// the athlete is sent to the login page and no code is issued. The login
/// page's continuation drops `prompt`, so once the form signs them in the
/// flow completes instead of looping back to the login page.
#[tokio::test]
async fn prompt_login_asks_for_the_password_despite_a_session_and_does_not_loop() {
    let resources = create_test_server_resources().await.unwrap();
    let (stale_user, _) = athlete_with_status(&resources, UserStatus::Active).await;
    let email = athlete(&resources).await;
    let client = FirstPartyClient::Web;
    let redirect_uri = client.redirect_uri(&resources.common.config.oauth2_server);
    let pkce = Pkce::generate();
    let state = fresh_state();
    let uri = format!(
        "{}&prompt=login",
        authorize_uri(client.client_id(), &redirect_uri, &pkce.challenge, &state)
    );

    let prompted = authorize_with_session(&resources, stale_user, &uri).await;
    assert!(is_redirect(&prompted), "{}", prompted.body_text());
    let login_page = location(&prompted);
    assert!(login_page.starts_with("/oauth2/login?"), "{login_page}");
    assert!(!login_page.contains("prompt"), "{login_page}");
    assert!(
        !login_page.contains("code="),
        "no code is issued: {login_page}"
    );

    // The athlete who typed the password is the one signed in.
    let router = first_party_router(&resources, DEFAULT_PEER);
    let login = submit_login(
        router.clone(),
        client.client_id(),
        &redirect_uri,
        &pkce.challenge,
        &state,
        &email,
        PASSWORD,
    )
    .await;
    let resume = location(&login);
    assert!(resume.starts_with("/oauth2/authorize?"), "{resume}");
    assert!(
        !resume.contains("prompt"),
        "the continuation cannot loop: {resume}"
    );

    let authorized = AxumTestRequest::get(&resume)
        .header("cookie", &session_cookie(&login))
        .send(router.clone())
        .await;
    let callback = location(&authorized);
    let code = callback_param(&callback, &redirect_uri, "code").expect("a code once signed in");

    let body: Value = redeem(
        router,
        client.client_id(),
        &code,
        &redirect_uri,
        Some(&pkce.verifier),
        None,
    )
    .await
    .json();
    assert_eq!(body["user"]["email"], email.as_str(), "{body}");
}

/// The hosted login is the apps' sign-in, so it speaks the athlete's language:
/// a French browser gets the French form and a French refusal, from the
/// apps' own catalogue.
#[tokio::test]
async fn the_hosted_login_speaks_the_browser_language() {
    let resources = create_test_server_resources().await.unwrap();
    let email = athlete(&resources).await;
    let router = first_party_router(&resources, [127, 0, 0, 1]);
    let redirect_uri =
        FirstPartyClient::Mobile.redirect_uri(&resources.common.config.oauth2_server);
    let pkce = Pkce::generate();
    let state = fresh_state();

    let page = AxumTestRequest::get(&format!(
        "/oauth2/login?response_type=code&client_id={MOBILE_CLIENT_ID}&redirect_uri={}\
         &code_challenge={}&code_challenge_method=S256&state={state}",
        urlencoding::encode(&redirect_uri),
        pkce.challenge,
    ))
    .header("accept-language", "fr-CA,fr;q=0.9,en;q=0.8")
    .send(router.clone())
    .await;
    assert_eq!(page.status(), 200);
    let html = page.text();
    assert!(html.contains(r#"<html lang="fr">"#), "{html}");
    assert!(html.contains(">Se connecter</button>"), "{html}");
    assert!(html.contains(">Mot de passe</label>"), "{html}");

    let refused = AxumTestRequest::post("/oauth2/login")
        .header("accept-language", "fr")
        .form(&[
            ("response_type", "code"),
            ("client_id", MOBILE_CLIENT_ID),
            ("redirect_uri", redirect_uri.as_str()),
            ("code_challenge", pkce.challenge.as_str()),
            ("code_challenge_method", "S256"),
            ("state", state.as_str()),
            ("email", email.as_str()),
            ("password", "not-the-password"),
        ])
        .send(router)
        .await;
    assert_eq!(refused.status(), 401);
    let html = refused.text();
    assert!(
        html.contains("Adresse e-mail ou mot de passe incorrect"),
        "{html}"
    );
    assert!(html.contains("Retour à la connexion"), "{html}");
}

/// The app's own language wins over the browser's: Dravr's apps send
/// `ui_locales` with the language they are showing, it survives the authorize
/// redirect to the form, and the form carries it into its failure page.
#[tokio::test]
async fn the_hosted_login_speaks_the_app_language_over_the_browser() {
    let resources = create_test_server_resources().await.unwrap();
    let router = first_party_router(&resources, [127, 0, 0, 1]);
    let redirect_uri =
        FirstPartyClient::Mobile.redirect_uri(&resources.common.config.oauth2_server);
    let pkce = Pkce::generate();
    let state = fresh_state();

    let authorize = AxumTestRequest::get(&format!(
        "{}&prompt=login&ui_locales=de",
        authorize_uri(MOBILE_CLIENT_ID, &redirect_uri, &pkce.challenge, &state)
    ))
    .header("accept-language", "en-US")
    .send(router.clone())
    .await;
    assert!(is_redirect(&authorize));
    let login_url = location(&authorize);
    assert!(login_url.starts_with("/oauth2/login?"), "{login_url}");
    assert!(login_url.ends_with("&ui_locales=de"), "{login_url}");
    assert!(!login_url.contains("prompt="), "{login_url}");

    let page = AxumTestRequest::get(&login_url)
        .header("accept-language", "en-US")
        .send(router)
        .await;
    let html = page.text();
    assert!(html.contains(r#"<html lang="de">"#), "{html}");
    assert!(html.contains(">Anmelden</button>"), "{html}");
    assert!(
        html.contains(r#"<input type="hidden" name="ui_locales" value="de">"#),
        "{html}"
    );
}
