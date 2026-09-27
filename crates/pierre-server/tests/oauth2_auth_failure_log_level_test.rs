// ABOUTME: A refused OAuth2 or JWT caller logs at WARN and never at ERROR, the level that pages the ops channel
// ABOUTME: Pins the rl-probe token path, control-character input, federated-only logins, and that a database failure still pages
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! # Only the server's own failures page
//!
//! `dravr-tronc`'s error notifier posts every `ERROR` event to the operators'
//! Slack channel and inbox. On 2026-09-26 a rate-limit probe sent bad
//! `POST /oauth2/token` calls as client `rl-probe`, and each one logged
//! "OAuth client validation failed" at `ERROR`: 284 events in the ops channel
//! for requests anyone can send.
//!
//! A refused caller (unknown client, wrong secret, malformed parameter,
//! expired or forged token, a password tried on an account that has none) is
//! the caller's mistake and logs at `WARN`. Each refusal below is paired with
//! the server failing on the same path, which must still log at `ERROR`: a
//! fix that silenced everything would pass the refusals alone.

mod common;
mod helpers;

use std::collections::HashMap;
use std::fmt::Debug as FmtDebug;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::extract::connect_info::MockConnectInfo;
use common::{create_test_server_resources, get_shared_test_jwks};
use helpers::axum_test::AxumTestRequest;
use pierre_auth::auth::AuthManager;
use pierre_auth::oauth2_server::client_registration::ClientRegistrationManager;
use pierre_auth::oauth2_server::endpoints::OAuth2AuthorizationServer;
use pierre_auth::oauth2_server::models::{
    AuthorizeRejection, AuthorizeRequest, ClientRegistrationRequest, ClientRegistrationResponse,
    TokenRequest,
};
use pierre_auth::oauth2_server::rate_limiting::OAuth2RateLimiter;
use pierre_core::constants::oauth2_client_retention::MAX_PENDING_REGISTRATIONS;
use pierre_core::models::{User, UserStatus, FEDERATED_ONLY_PASSWORD_HASH};
use pierre_database::backends::factory::Database;
use pierre_database::backends::DatabaseProvider;
use pierre_database::database::generate_encryption_key;
use pierre_database::database::test_utils::create_test_db_with_key;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_routes_identity::oauth2::{OAuth2Context, OAuth2Routes};
use tracing::field::{Field, Visit};
use tracing::subscriber::DefaultGuard;
use tracing::{Level, Subscriber};
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::Layer;

/// The client id the 2026-09-26 probe used.
const PROBE_CLIENT_ID: &str = "rl-probe";

/// A PKCE challenge of the length RFC 7636 requires (43 characters).
const CODE_CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";

const REDIRECT_URI: &str = "https://example.com/callback";

// ── Event capture ────────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
struct CapturedEvent {
    level: Level,
    message: String,
    fields: HashMap<String, String>,
}

#[derive(Clone, Default)]
struct CaptureLayer {
    events: Arc<Mutex<Vec<CapturedEvent>>>,
}

#[derive(Default)]
struct FieldVisitor {
    message: String,
    fields: HashMap<String, String>,
}

impl Visit for FieldVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn FmtDebug) {
        let rendered = format!("{value:?}");
        if field.name() == "message" {
            self.message.clone_from(&rendered);
        }
        self.fields.insert(field.name().to_owned(), rendered);
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            value.clone_into(&mut self.message);
        }
        self.fields
            .insert(field.name().to_owned(), value.to_owned());
    }
}

impl<S> Layer<S> for CaptureLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let mut visitor = FieldVisitor::default();
        event.record(&mut visitor);
        self.events.lock().unwrap().push(CapturedEvent {
            level: *event.metadata().level(),
            message: visitor.message,
            fields: visitor.fields,
        });
    }
}

/// Every event emitted on this thread while the guard lives.
struct Captured {
    events: Arc<Mutex<Vec<CapturedEvent>>>,
    _guard: DefaultGuard,
}

impl Captured {
    fn start() -> Self {
        // The process-wide subscriber goes in first. A scoped `set_default`
        // installs the `log` bridge globally, after which the global `init`
        // that `create_test_server_resources` runs would panic.
        common::init_test_logging();
        let layer = CaptureLayer::default();
        let events = Arc::clone(&layer.events);
        let guard = tracing_subscriber::registry().with(layer).set_default();
        Self {
            events,
            _guard: guard,
        }
    }

    fn at(&self, level: Level) -> Vec<CapturedEvent> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.level == level)
            .cloned()
            .collect()
    }

    /// Fails the test, naming every event, when anything was logged at ERROR.
    fn assert_nothing_paged(&self) {
        let errors = self.at(Level::ERROR);
        assert!(
            errors.is_empty(),
            "a caller's mistake must not log at ERROR (it pages the ops channel): {errors:#?}"
        );
    }

    /// The single WARN event whose message is `message`.
    fn warning(&self, message: &str) -> CapturedEvent {
        let matching: Vec<_> = self
            .at(Level::WARN)
            .into_iter()
            .filter(|e| e.message == message)
            .collect();
        assert_eq!(
            matching.len(),
            1,
            "expected exactly one WARN {message:?}, got: {:#?}",
            self.events.lock().unwrap()
        );
        matching.into_iter().next().unwrap()
    }

    /// The single ERROR event whose message is `message`.
    fn page(&self, message: &str) -> CapturedEvent {
        let matching: Vec<_> = self
            .at(Level::ERROR)
            .into_iter()
            .filter(|e| e.message == message)
            .collect();
        assert_eq!(
            matching.len(),
            1,
            "expected exactly one ERROR {message:?}, got: {:#?}",
            self.events.lock().unwrap()
        );
        matching.into_iter().next().unwrap()
    }

    fn all_text(&self) -> String {
        format!("{:?}", self.events.lock().unwrap())
    }
}

// ── Fixtures ─────────────────────────────────────────────────────────────────

struct Fixture {
    database: Arc<Database>,
    server: OAuth2AuthorizationServer,
    registrations: ClientRegistrationManager,
}

async fn fixture() -> Fixture {
    let database = Arc::new(
        create_test_db_with_key(generate_encryption_key().to_vec())
            .await
            .unwrap(),
    );
    database.migrate().await.unwrap();
    let repos = database.repositories();
    let server = OAuth2AuthorizationServer::new(
        repos.oauth2_server.clone(),
        repos.tenants.clone(),
        repos.users.clone(),
        Arc::new(AuthManager::new(24)),
        get_shared_test_jwks(),
    );
    let registrations = ClientRegistrationManager::new(repos.oauth2_server.clone());
    Fixture {
        database,
        server,
        registrations,
    }
}

async fn register(fixture: &Fixture) -> ClientRegistrationResponse {
    fixture
        .registrations
        .register_client(
            registration(Some("Log Level Client")),
            MAX_PENDING_REGISTRATIONS,
        )
        .await
        .unwrap()
}

fn registration(client_name: Option<&str>) -> ClientRegistrationRequest {
    ClientRegistrationRequest {
        redirect_uris: vec![REDIRECT_URI.to_owned()],
        client_name: client_name.map(str::to_owned),
        client_uri: None,
        grant_types: None,
        response_types: None,
        scope: None,
    }
}

fn client_credentials(client_id: &str, client_secret: &str) -> TokenRequest {
    TokenRequest {
        grant_type: "client_credentials".to_owned(),
        code: None,
        redirect_uri: None,
        client_id: client_id.to_owned(),
        client_secret: client_secret.to_owned(),
        scope: None,
        refresh_token: None,
        code_verifier: None,
    }
}

fn authorize_request(client_id: &str, state: &str) -> AuthorizeRequest {
    AuthorizeRequest {
        response_type: "code".to_owned(),
        client_id: client_id.to_owned(),
        redirect_uri: REDIRECT_URI.to_owned(),
        scope: None,
        state: Some(state.to_owned()),
        code_challenge: Some(CODE_CHALLENGE.to_owned()),
        code_challenge_method: Some("S256".to_owned()),
    }
}

/// Moves `oauth2_clients` out of reach, so every client lookup fails in the
/// database itself, the way an outage or a broken schema would.
async fn break_client_table(database: &Database) {
    const RENAME: &str = "ALTER TABLE oauth2_clients RENAME TO oauth2_clients_unreachable";
    match database {
        Database::SQLite(db) => {
            sqlx::query(RENAME).execute(db.pool()).await.unwrap();
        }
        #[cfg(feature = "postgresql")]
        Database::PostgreSQL(db) => {
            sqlx::query(RENAME).execute(db.pool()).await.unwrap();
        }
    }
}

// ── The token endpoint: the incident path ────────────────────────────────────

#[tokio::test]
async fn an_unknown_client_at_the_token_endpoint_warns_and_never_pages() {
    let fixture = fixture().await;
    let captured = Captured::start();

    let refused = fixture
        .server
        .token(client_credentials(PROBE_CLIENT_ID, "not-the-secret"))
        .await
        .unwrap_err();

    assert_eq!(refused.error, "invalid_client");
    captured.assert_nothing_paged();
    let unknown = captured.warning("OAuth2 client refused: unknown client_id");
    assert_eq!(unknown.fields["client_id"], format!("{PROBE_CLIENT_ID:?}"));
    let refusal = captured.warning("OAuth client validation failed");
    assert_eq!(refusal.fields["grant_type"], "client_credentials");
    assert_eq!(refusal.fields["error"], "invalid_client");
}

#[tokio::test]
async fn a_wrong_client_secret_warns_and_never_pages() {
    let fixture = fixture().await;
    let client = register(&fixture).await;
    let captured = Captured::start();

    let refused = fixture
        .server
        .token(client_credentials(&client.client_id, "not-the-secret"))
        .await
        .unwrap_err();

    assert_eq!(refused.error, "invalid_client");
    captured.assert_nothing_paged();
    captured.warning("OAuth client validation failed");
    assert!(
        captured
            .at(Level::WARN)
            .iter()
            .any(|e| e.message.contains("secret validation failed")),
        "the wrong secret is named where it was detected: {:#?}",
        captured.at(Level::WARN)
    );
}

#[tokio::test]
async fn a_client_id_carrying_a_nul_byte_is_refused_as_unknown_without_a_query() {
    let fixture = fixture().await;
    let captured = Captured::start();

    let refused = fixture
        .server
        .token(client_credentials("rl\u{0}probe", "not-the-secret"))
        .await
        .unwrap_err();

    assert_eq!(refused.error, "invalid_client");
    captured.assert_nothing_paged();
    let unknown = captured.warning("OAuth2 client refused: unknown client_id");
    assert_eq!(unknown.fields["client_id"], "\"rl\\0probe\"");
}

#[tokio::test]
async fn a_database_failure_during_the_client_lookup_still_pages() {
    let fixture = fixture().await;
    let client = register(&fixture).await;
    break_client_table(&fixture.database).await;
    let captured = Captured::start();

    let refused = fixture
        .server
        .token(client_credentials(&client.client_id, &client.client_secret))
        .await
        .unwrap_err();

    assert_eq!(refused.error, "invalid_client");
    let page = captured.page("OAuth2 client lookup failed");
    assert_eq!(page.fields["client_id"], client.client_id);
    assert!(
        page.fields["error"].contains("oauth2_clients"),
        "the page names the failing query: {page:#?}"
    );
    assert!(
        captured
            .at(Level::WARN)
            .iter()
            .all(|e| e.message != "OAuth2 client refused: unknown client_id"),
        "an outage is never reported as an unknown client"
    );
}

#[tokio::test]
async fn a_token_request_carrying_a_control_character_is_refused_before_any_query() {
    let fixture = fixture().await;
    let client = register(&fixture).await;
    let captured = Captured::start();

    let refused = fixture
        .server
        .token(TokenRequest {
            grant_type: "authorization_code".to_owned(),
            code: Some("code\u{0}".to_owned()),
            redirect_uri: Some(REDIRECT_URI.to_owned()),
            code_verifier: Some("v".repeat(43)),
            ..client_credentials(&client.client_id, &client.client_secret)
        })
        .await
        .unwrap_err();

    assert_eq!(refused.error, "invalid_request");
    assert_eq!(
        refused.error_description.as_deref(),
        Some("Request parameters must not contain control characters")
    );
    captured.assert_nothing_paged();
    captured.warning("OAuth2 request refused: a parameter carries a control character");
}

// ── The authorization endpoint ───────────────────────────────────────────────

#[tokio::test]
async fn an_unknown_client_at_the_authorize_endpoint_warns_and_never_pages() {
    let fixture = fixture().await;
    let captured = Captured::start();

    let rejection = fixture
        .server
        .check_authorize_request(&authorize_request(PROBE_CLIENT_ID, "state-1"))
        .await
        .unwrap_err();

    let AuthorizeRejection::ShownToUser(error) = rejection else {
        panic!("an unknown client's error is never redirected: {rejection:?}");
    };
    assert_eq!(error.error, "invalid_client");
    captured.assert_nothing_paged();
    captured.warning("OAuth2 client refused: unknown client_id");
}

#[tokio::test]
async fn a_database_failure_at_the_authorize_endpoint_still_pages() {
    let fixture = fixture().await;
    let client = register(&fixture).await;
    break_client_table(&fixture.database).await;
    let captured = Captured::start();

    let rejection = fixture
        .server
        .check_authorize_request(&authorize_request(&client.client_id, "state-1"))
        .await
        .unwrap_err();

    assert!(matches!(rejection, AuthorizeRejection::ShownToUser(_)));
    captured.page("OAuth2 client lookup failed");
}

#[tokio::test]
async fn an_authorize_state_carrying_a_control_character_goes_back_to_the_client() {
    let fixture = fixture().await;
    let client = register(&fixture).await;
    let captured = Captured::start();

    let rejection = fixture
        .server
        .check_authorize_request(&authorize_request(&client.client_id, "state\u{0}"))
        .await
        .unwrap_err();

    let AuthorizeRejection::RedirectedToClient(error) = rejection else {
        panic!("a verified client's error goes back to it: {rejection:?}");
    };
    assert_eq!(error.error, "invalid_request");
    captured.assert_nothing_paged();

    // The same request with a printable state is accepted: the guard refuses
    // the control character, not the request.
    let scope = fixture
        .server
        .check_authorize_request(&authorize_request(&client.client_id, "state-1"))
        .await
        .unwrap();
    assert!(!scope.is_empty());
}

// ── Dynamic registration ─────────────────────────────────────────────────────

#[tokio::test]
async fn a_registration_carrying_a_control_character_is_refused_as_metadata() {
    let fixture = fixture().await;
    let captured = Captured::start();

    let refused = fixture
        .registrations
        .register_client(registration(Some("probe\u{0}")), MAX_PENDING_REGISTRATIONS)
        .await
        .unwrap_err();

    assert_eq!(refused.error, "invalid_client_metadata");
    captured.assert_nothing_paged();

    let accepted = fixture
        .registrations
        .register_client(registration(Some("probe")), MAX_PENDING_REGISTRATIONS)
        .await
        .unwrap();
    assert!(accepted.client_id.starts_with("mcp_client_"));
}

// ── JWT validation ───────────────────────────────────────────────────────────

#[tokio::test]
async fn a_forged_jwt_warns_and_never_pages() {
    let auth_manager = AuthManager::new(24);
    let jwks = get_shared_test_jwks();
    let athlete = User::new("athlete@example.com".to_owned(), "hash".to_owned(), None);
    let other = User::new("other@example.com".to_owned(), "hash".to_owned(), None);
    let genuine = auth_manager.generate_token(&athlete, &jwks).unwrap();
    let foreign = auth_manager.generate_token(&other, &jwks).unwrap();

    // The athlete's header and claims under the other token's signature: a
    // forgery every part of which decodes.
    let (signed, _) = genuine.rsplit_once('.').unwrap();
    let (_, signature) = foreign.rsplit_once('.').unwrap();
    let forged = format!("{signed}.{signature}");

    let captured = Captured::start();
    let refused = auth_manager.validate_token(&forged, &jwks).unwrap_err();

    assert!(refused.message.contains("JWT validation failed"));
    captured.assert_nothing_paged();
    captured.warning("RS256 JWT validation failed: Error(InvalidSignature)");

    // The genuine token still validates, so the refusal is the signature's.
    let claims = auth_manager.validate_token(&genuine, &jwks).unwrap();
    assert_eq!(claims.email, "athlete@example.com");
}

// ── The OAuth login page ─────────────────────────────────────────────────────

fn oauth2_routes(resources: &Arc<ServerContext>) -> axum::Router {
    let context = OAuth2Context {
        database: resources.agent.database.clone(),
        oauth2_server: resources.common.repos.oauth2_server.clone(),
        tenants: resources.common.repos.tenants.clone(),
        users: resources.common.repos.users.clone(),
        auth_manager: resources.auth.auth_manager.clone(),
        jwks_manager: resources.auth.jwks_manager.clone(),
        config: Arc::new(resources.common.config.oauth2_server.clone()),
        rate_limiter: Arc::new(OAuth2RateLimiter::new(
            None,
            OAuth2RateLimiter::local_window_store(),
            &resources.common.config.rate_limiting,
        )),
    };
    OAuth2Routes::routes(context).layer(MockConnectInfo(SocketAddr::from(([127, 0, 0, 1], 40_000))))
}

#[tokio::test]
async fn a_password_tried_on_a_federated_only_account_is_refused_without_paging() {
    let resources = create_test_server_resources().await.unwrap();
    let mut federated = User::new(
        "federated@example.com".to_owned(),
        FEDERATED_ONLY_PASSWORD_HASH.to_owned(),
        None,
    );
    federated.firebase_uid = Some("firebase-uid-1".to_owned());
    "google.com".clone_into(&mut federated.auth_provider);
    federated.user_status = UserStatus::Active;
    resources
        .common
        .repos
        .users
        .create(&federated)
        .await
        .unwrap();
    assert!(!federated.has_password());

    let captured = Captured::start();
    let response = AxumTestRequest::post("/oauth2/login")
        .form(&[
            ("email", "federated@example.com"),
            ("password", "any password at all"),
        ])
        .send(oauth2_routes(&resources))
        .await;

    assert_eq!(response.status(), 401);
    captured.assert_nothing_paged();
    assert!(
        !captured.all_text().contains(FEDERATED_ONLY_PASSWORD_HASH),
        "the stored password hash never reaches a log line"
    );
    assert!(
        captured
            .at(Level::WARN)
            .iter()
            .any(|e| e.message.contains("Invalid password")),
        "the refusal is logged as a failed sign-in: {:#?}",
        captured.at(Level::WARN)
    );
}

#[tokio::test]
async fn a_password_account_is_recognised_as_having_one() {
    let bcrypt_hash = bcrypt::hash("correct horse", 4).unwrap();
    let with_password = User::new("athlete@example.com".to_owned(), bcrypt_hash, None);
    let federated = User::new(
        "federated@example.com".to_owned(),
        FEDERATED_ONLY_PASSWORD_HASH.to_owned(),
        None,
    );

    assert!(with_password.has_password());
    assert!(!federated.has_password());
}
