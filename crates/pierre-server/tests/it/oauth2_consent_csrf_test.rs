// ABOUTME: The OAuth consent form through the server's own router, CSRF layer included (carnet#646)
// ABOUTME: A web-app session cookie no longer refuses the approval; the form's own token is what it proves
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! An athlete signed in to the web app carries its session cookie to
//! the consent form, which the CSRF layer answered with 401 because an HTML
//! form cannot send `X-CSRF-Token` — no MCP connector could be authorized from
//! a browser that also had the app open. The form now carries a synchronizer
//! token the consent handler checks instead, so a forged approval is still
//! refused.

use crate::common;
use crate::helpers;

use std::fmt::Debug;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::extract::connect_info::MockConnectInfo;
use base64::{engine::general_purpose, Engine as _};
use common::create_test_tenant;
use helpers::axum_test::{AxumTestRequest, AxumTestResponse};
use pierre_auth::oauth2_server::client_registration::ClientRegistrationManager;
use pierre_auth::oauth2_server::models::ClientRegistrationRequest;
use pierre_auth::security::cookies::auth_cookie_name;
use pierre_core::constants::oauth2_client_retention::MAX_PENDING_REGISTRATIONS;
use pierre_core::models::UserStatus;
use pierre_core::permissions::scopes::OAuthScope;
use pierre_database::backends::factory::DatabaseBackend;
use pierre_mcp_server::mcp::multitenant::ProviderToolRouter;
use pierre_mcp_server::mcp::resources::ServerContext;
use regex::Regex;
use sha2::{Digest, Sha256};
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::Layer;
use url::Url;

const REDIRECT: &str = "https://client.example.test/callback";
const STATE: &str = "consent-csrf-state";
const VERIFIER: &str = "consent-csrf-pkce-verifier-0123456789-abcdefghijklmnopqrst";

/// The server's own HTTP composition, CSRF layer included.
fn app(resources: &Arc<ServerContext>) -> axum::Router {
    ProviderToolRouter::build_http_app(resources)
        .layer(MockConnectInfo(SocketAddr::from(([127, 0, 0, 1], 40_646))))
}

async fn register(resources: &Arc<ServerContext>) -> String {
    ClientRegistrationManager::new(resources.common.repos.oauth2_server.clone())
        .register_client(
            ClientRegistrationRequest {
                redirect_uris: vec![REDIRECT.to_owned()],
                client_name: Some("Consent CSRF client".to_owned()),
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

fn challenge() -> String {
    general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(VERIFIER.as_bytes()))
}

/// The authorization request a client names no scope in.
fn authorize_uri(client_id: &str) -> String {
    format!(
        "/oauth2/authorize?response_type=code&client_id={}&redirect_uri={}&state={STATE}&code_challenge={}&code_challenge_method=S256",
        urlencoding::encode(client_id),
        urlencoding::encode(REDIRECT),
        challenge(),
    )
}

/// The consent page the signed-in athlete is shown, and its form token.
async fn consent_page(resources: &Arc<ServerContext>, client_id: &str, cookie: &str) -> String {
    let page = AxumTestRequest::get(&authorize_uri(client_id))
        .header("cookie", cookie)
        .send(app(resources))
        .await;
    assert_eq!(page.status(), 200, "a first authorization asks for consent");
    let html = page.text();
    let token = html
        .split("name=\"csrf_token\" value=\"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .unwrap_or_else(|| panic!("the consent form carries its token: {html}"))
        .to_owned();
    token
}

/// Submit the consent form with `decision`, carrying `token` when given.
async fn submit(
    resources: &Arc<ServerContext>,
    client_id: &str,
    cookie: &str,
    token: Option<&str>,
    decision: &str,
) -> AxumTestResponse {
    let challenge = challenge();
    let mut form = vec![
        ("response_type", "code"),
        ("client_id", client_id),
        ("redirect_uri", REDIRECT),
        ("state", STATE),
        ("code_challenge", challenge.as_str()),
        ("code_challenge_method", "S256"),
        ("decision", decision),
    ];
    if let Some(token) = token {
        form.push(("csrf_token", token));
    }
    AxumTestRequest::post("/oauth2/consent")
        .header("cookie", cookie)
        .form(&form)
        .send(app(resources))
        .await
}

fn redirect_param(response: &AxumTestResponse, name: &str) -> Option<String> {
    let location = response.header("location")?;
    assert!(
        location.starts_with(REDIRECT),
        "sent to the client: {location}"
    );
    Url::parse(location)
        .unwrap()
        .query_pairs()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned())
}

#[tokio::test]
async fn a_web_app_session_approves_through_the_forms_own_token() {
    common::init_server_config();
    let resources = common::create_test_server_resources().await.unwrap();
    let client_id = register(&resources).await;
    let (_user, session) = create_test_tenant(&resources, "consent-web@example.test")
        .await
        .unwrap();
    // The web app's cookie: the one the CSRF layer demands a header for.
    let cookie = format!("{}={session}", auth_cookie_name());

    let token = consent_page(&resources, &client_id, &cookie).await;
    let approved = submit(&resources, &client_id, &cookie, Some(&token), "approve").await;

    assert_eq!(
        approved.status(),
        303,
        "the approval is not refused for want of an X-CSRF-Token header: {}",
        approved.text()
    );
    assert_eq!(redirect_param(&approved, "state").as_deref(), Some(STATE));
    assert!(
        redirect_param(&approved, "code").is_some_and(|code| !code.is_empty()),
        "an approved consent redirects with a code"
    );
}

/// The grant row holds what the code carries, not the request's absent scope, so
/// the athlete's list of connected apps says what the client holds, and the
/// approval is found again on the client's next authorization.
#[tokio::test]
async fn an_approval_naming_no_scope_records_the_default_grant_and_is_remembered() {
    common::init_server_config();
    let resources = common::create_test_server_resources().await.unwrap();
    let client_id = register(&resources).await;
    let (user, session) = create_test_tenant(&resources, "consent-scope@example.test")
        .await
        .unwrap();
    let cookie = format!("{}={session}", auth_cookie_name());

    let token = consent_page(&resources, &client_id, &cookie).await;
    let approved = submit(&resources, &client_id, &cookie, Some(&token), "approve").await;
    assert_eq!(approved.status(), 303, "{}", approved.text());

    let repos = &resources.common.repos;
    let tenant = repos
        .tenants
        .first_for_user(user.id)
        .await
        .unwrap()
        .expect("the athlete has a tenant");
    let grants = repos
        .oauth2_server
        .list_client_grants(&user.id.to_string(), &tenant.to_string())
        .await
        .unwrap();
    assert_eq!(grants.len(), 1, "one approval, one grant");
    assert_eq!(
        grants[0].scope,
        OAuthScope::render_granted(&OAuthScope::default_grant())
    );

    // A new authorization carries a new `state`; the first one is spent.
    let again = AxumTestRequest::get(&authorize_uri(&client_id).replace(STATE, "second-state"))
        .header("cookie", &cookie)
        .send(app(&resources))
        .await;
    assert_eq!(again.status(), 303, "a remembered approval asks nothing");
    assert!(
        redirect_param(&again, "code").is_some_and(|code| !code.is_empty()),
        "a remembered approval redirects with a code"
    );
}

/// An approval the server could not write mints nothing: the athlete could
/// neither see nor revoke the connection it would have started.
#[tokio::test]
async fn an_approval_that_cannot_be_recorded_mints_no_code() {
    common::init_server_config();
    let resources = common::create_test_server_resources().await.unwrap();
    let client_id = register(&resources).await;
    let (_user, session) = create_test_tenant(&resources, "consent-unwritten@example.test")
        .await
        .unwrap();
    let cookie = format!("{}={session}", auth_cookie_name());
    let token = consent_page(&resources, &client_id, &cookie).await;

    match resources.agent.database.backend() {
        DatabaseBackend::SQLite(db) => {
            sqlx::query("DROP TABLE oauth_client_grants")
                .execute(db.pool())
                .await
                .unwrap();
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(db) => {
            sqlx::query("DROP TABLE oauth_client_grants CASCADE")
                .execute(db.pool())
                .await
                .unwrap();
        }
    }
    let refused = submit(&resources, &client_id, &cookie, Some(&token), "approve").await;

    assert_eq!(refused.status(), 303, "{}", refused.text());
    assert_eq!(redirect_param(&refused, "code"), None, "no record, no code");
    assert_eq!(
        redirect_param(&refused, "error").as_deref(),
        Some("server_error")
    );
}

/// A grant keyed to no tenant is no record either. The code's own mint also
/// needs a tenant, so this pins the outcome rather than the consent's refusal,
/// which the unwritable table above makes fire.
#[tokio::test]
async fn an_approval_with_no_tenant_to_record_it_in_mints_no_code() {
    const LEAVE: &str = "DELETE FROM tenant_users WHERE user_id = $1";
    common::init_server_config();
    let resources = common::create_test_server_resources().await.unwrap();
    let client_id = register(&resources).await;
    let (user, _) = create_test_tenant(&resources, "consent-tenantless@example.test")
        .await
        .unwrap();
    // A session naming no tenant leaves the grant's to the membership.
    let session = resources
        .auth
        .auth_manager
        .generate_token(&user, &resources.auth.jwks_manager)
        .unwrap();
    let cookie = format!("{}={session}", auth_cookie_name());
    let token = consent_page(&resources, &client_id, &cookie).await;
    match resources.agent.database.backend() {
        DatabaseBackend::SQLite(db) => {
            sqlx::query(LEAVE)
                .bind(user.id.to_string())
                .execute(db.pool())
                .await
                .unwrap();
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(db) => {
            sqlx::query(LEAVE)
                .bind(user.id)
                .execute(db.pool())
                .await
                .unwrap();
        }
    }

    let refused = submit(&resources, &client_id, &cookie, Some(&token), "approve").await;

    assert_eq!(refused.status(), 303, "{}", refused.text());
    assert_eq!(redirect_param(&refused, "code"), None, "no record, no code");
}

#[tokio::test]
async fn an_approval_without_the_forms_token_is_refused() {
    common::init_server_config();
    let resources = common::create_test_server_resources().await.unwrap();
    let client_id = register(&resources).await;
    let (_user, session) = create_test_tenant(&resources, "consent-forged@example.test")
        .await
        .unwrap();
    let cookie = format!("{}={session}", auth_cookie_name());

    let forged = submit(&resources, &client_id, &cookie, None, "approve").await;
    assert_ne!(forged.status(), 303, "no redirect, so no code");
    assert!(
        forged.header("location").is_none(),
        "a forged approval reaches no client"
    );
    let body = forged.text();
    // The athlete's stored language is the default, French
    assert!(
        body.contains("Échec de la connexion"),
        "the failure page, not a code: {body}"
    );
}

#[tokio::test]
async fn another_athletes_form_token_is_refused() {
    common::init_server_config();
    let resources = common::create_test_server_resources().await.unwrap();
    let client_id = register(&resources).await;
    let (_victim, victim_session) = create_test_tenant(&resources, "consent-victim@example.test")
        .await
        .unwrap();
    let (_other, other_session) = create_test_tenant(&resources, "consent-other@example.test")
        .await
        .unwrap();

    // A token minted for one athlete does not approve for another.
    let others_token = consent_page(
        &resources,
        &client_id,
        &format!("{}={other_session}", auth_cookie_name()),
    )
    .await;
    let forged = submit(
        &resources,
        &client_id,
        &format!("{}={victim_session}", auth_cookie_name()),
        Some(&others_token),
        "approve",
    )
    .await;
    assert!(
        forged.header("location").is_none(),
        "another athlete's token approves nothing"
    );
}

#[tokio::test]
async fn a_denial_needs_no_token_and_reaches_the_client() {
    common::init_server_config();
    let resources = common::create_test_server_resources().await.unwrap();
    let client_id = register(&resources).await;
    let (_user, session) = create_test_tenant(&resources, "consent-deny@example.test")
        .await
        .unwrap();

    let denied = submit(
        &resources,
        &client_id,
        &format!("{}={session}", auth_cookie_name()),
        None,
        "deny",
    )
    .await;
    assert_eq!(denied.status(), 303);
    assert_eq!(
        redirect_param(&denied, "error").as_deref(),
        Some("access_denied")
    );
}

/// Keeps the message of every event logged on this thread.
#[derive(Clone, Default)]
struct MessageCapture {
    messages: Arc<Mutex<Vec<String>>>,
}

struct MessageVisitor<'a>(&'a mut String);

impl Visit for MessageVisitor<'_> {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            *self.0 = format!("{value:?}");
        }
    }
}

impl<S: Subscriber> Layer<S> for MessageCapture {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut message = String::new();
        event.record(&mut MessageVisitor(&mut message));
        self.messages.lock().unwrap().push(message);
    }
}

/// The page `public_mcp_monitoring.tf` sends when a stranger cannot finish
/// connecting matches on the words of this refusal. Reworded on one side only,
/// the page would never fire and nothing else would say so.
#[tokio::test]
async fn the_refusal_an_alert_pages_on_is_the_one_the_server_logs() {
    let terraform = include_str!("../../../../infra/environments/dev/public_mcp_monitoring.tf");
    let reason = &Regex::new(r#"metric\.labels\.reason=\\"([^"\\]+)\\""#)
        .unwrap()
        .captures(terraform)
        .expect("public_mcp_monitoring.tf pages on one consent refusal reason")[1];

    common::init_server_config();
    let resources = common::create_test_server_resources().await.unwrap();
    let client_id = register(&resources).await;
    let (user, session) = create_test_tenant(&resources, "consent-pending@example.test")
        .await
        .unwrap();
    resources
        .common
        .repos
        .users
        .update_status(user.id, UserStatus::Pending, None)
        .await
        .unwrap();
    let cookie = format!("{}={session}", auth_cookie_name());

    let capture = MessageCapture::default();
    let messages = Arc::clone(&capture.messages);
    let guard = tracing_subscriber::registry().with(capture).set_default();
    let page = AxumTestRequest::get(&authorize_uri(&client_id))
        .header("cookie", &cookie)
        .send(app(&resources))
        .await;
    drop(guard);

    let text = page.text();
    assert!(text.contains("en attente d’approbation"), "{text}");
    let wanted = format!("OAuth authorization refused: {reason}");
    let logged = messages.lock().unwrap();
    assert!(
        logged.contains(&wanted),
        "the alert pages on {wanted:?}; the server logged {logged:#?}"
    );
}

/// The consent page a signed-in athlete reads, from a browser asking for
/// `accept_language`.
async fn consent_language(
    resources: &Arc<ServerContext>,
    client_id: &str,
    cookie: &str,
    accept_language: &str,
) -> String {
    let page = AxumTestRequest::get(&authorize_uri(client_id))
        .header("cookie", cookie)
        .header("accept-language", accept_language)
        .send(app(resources))
        .await;
    assert_eq!(page.status(), 200, "a first authorization asks for consent");
    page.text()
}

/// The athlete's stored language wins over the browser's, in both directions.
#[tokio::test]
async fn the_consent_page_reads_in_the_stored_language_over_the_browsers() {
    common::init_server_config();
    let resources = common::create_test_server_resources().await.unwrap();
    let client_id = register(&resources).await;
    let (user, session) = create_test_tenant(&resources, "consent-lang@example.test")
        .await
        .unwrap();
    let cookie = format!("{}={session}", auth_cookie_name());
    let users = resources.common.repos.users.clone();

    users.update_locale(user.id, "fr").await.unwrap();
    let french = consent_language(&resources, &client_id, &cookie, "en-US,en;q=0.9").await;
    assert!(french.contains("<html lang=\"fr\">"), "{french}");
    assert!(french.contains("vous consentez expressément"), "{french}");
    assert!(!french.contains("expressly consent"), "{french}");

    users.update_locale(user.id, "en").await.unwrap();
    let english = consent_language(&resources, &client_id, &cookie, "fr-CA,fr;q=0.9").await;
    assert!(english.contains("<html lang=\"en\">"), "{english}");
    assert!(english.contains("expressly consent"), "{english}");
    assert!(!english.contains("consentez"), "{english}");
}

/// A refusal after the screen speaks to the athlete in their language too.
#[tokio::test]
async fn a_refused_approval_reads_in_the_stored_language() {
    common::init_server_config();
    let resources = common::create_test_server_resources().await.unwrap();
    let client_id = register(&resources).await;
    let (user, session) = create_test_tenant(&resources, "consent-refused-fr@example.test")
        .await
        .unwrap();
    resources
        .common
        .repos
        .users
        .update_locale(user.id, "fr")
        .await
        .unwrap();
    let cookie = format!("{}={session}", auth_cookie_name());

    let body = submit(&resources, &client_id, &cookie, None, "approve")
        .await
        .text();

    assert!(body.contains("<html lang=\"fr\">"), "{body}");
    assert!(body.contains("<h1>Échec de la connexion</h1>"), "{body}");
    assert!(body.contains("L’écran de consentement a expiré"), "{body}");
    assert!(!body.contains("Connection Failed"), "{body}");
}

/// With nobody signed in the browser's language decides, and one the platform
/// does not speak, or none, is English rather than the platform's default.
#[tokio::test]
async fn a_refusal_with_no_session_follows_the_browser_then_english() {
    common::init_server_config();
    let resources = common::create_test_server_resources().await.unwrap();
    let refused = |accept_language: Option<&'static str>| {
        let resources = resources.clone();
        async move {
            let mut request = AxumTestRequest::get(&authorize_uri("no-such-client"));
            if let Some(language) = accept_language {
                request = request.header("accept-language", language);
            }
            request.send(app(&resources)).await.text()
        }
    };

    let french = refused(Some("fr-CA,fr;q=0.9,en;q=0.8")).await;
    assert!(french.contains("<html lang=\"fr\">"), "{french}");
    assert!(
        french.contains("<h1>Échec de la connexion</h1>"),
        "{french}"
    );
    assert!(
        french.contains("Cette application n’est pas reconnue"),
        "{french}"
    );
    assert!(!french.contains("Client authentication failed"), "{french}");

    for language in [Some("ja-JP"), None] {
        let english = refused(language).await;
        assert!(english.contains("<html lang=\"en\">"), "{english}");
        assert!(english.contains("<h1>Connection Failed</h1>"), "{english}");
    }
}
