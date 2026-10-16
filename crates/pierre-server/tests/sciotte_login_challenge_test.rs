// ABOUTME: carnet#1035 — a Garmin sign-in stopped at a captcha is the athlete's to retry, never an operator page
// ABOUTME: Pins the challenge copy and reason, and a re-login sending only the caller's own dead session as prior

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Garmin's SSO puts a captcha in front of a sign-in it does not trust, and
//! the scraper service stops there without answering it: `403
//! {"error":"login_challenge"}`. Read as an unrecognised login answer, that
//! paged an operator for a provider's bot check and told the athlete nothing
//! useful. The platform now answers it with its own copy and the reason the
//! hosted pages select on, and goes around the system-failure path.
//!
//! A re-login also sends the athlete's previous session as `prior_session`, so
//! the provider sees the device it trusted last time instead of a stranger —
//! but only once the platform has observed that session dead: its expiry is
//! past, or a read the provider refused flagged the athlete's connection
//! `needs_reauth`. Sciotte sessions carry no expiry in practice, so the flag
//! is what makes the case reachable. One not observed dead (no past expiry,
//! an `active` connection), however old, is never sent: its live cookies
//! could sign the browser in to the account a coach is switching away from.
//! The session is read by the caller's own (user, tenant): another athlete's
//! is never sent, and a fresh connect sends none.
//!
//! The scraper is a loopback stand-in (a test double, per the repo's mock
//! rule) that records, per email, whether a login carried a prior session and
//! which session it named. It runs on a thread of its own, because each
//! `#[tokio::test]` drops its runtime when it ends and a stand-in spawned on
//! one would die with the first test to finish.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

use std::collections::HashMap;
use std::env;
use std::fmt::Debug as FmtDebug;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::thread;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::Utc;
use common::{create_test_server_resources, create_test_user_with_email, generate_test_token};
use dravr_sciotte::client::{ENV_AUDIENCE, ENV_REMOTE_URL};
use helpers::axum_test::AxumTestRequest;
use pierre_core::constants::oauth::providers::provider_terms_version;
use pierre_core::constants::oauth_providers::{SCIOTTE_GARMIN, TOKEN_TYPE_SESSION};
use pierre_core::models::{ConnectionStatus, ConnectionType, ReauthMark, TenantId, UserOAuthToken};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_middleware::provider_link_token::{mint_link_token, MintProviderLinkTokenArgs};
use pierre_routes_auth::{
    friendly_login_challenge_message, require_remote_flow, AuthRoutes, LOGIN_CHALLENGE_REASON,
};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::runtime::Builder;
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::subscriber::{set_default, DefaultGuard};
use tracing::{Event, Metadata, Subscriber};
use uuid::Uuid;

/// Logins whose email starts with this are stopped at a challenge.
const CHALLENGED: &str = "challenged";

/// Logins whose email starts with this ask for a code; the code step is then
/// stopped at a challenge.
const CODE_STEP: &str = "code-step";

/// The session a successful login signs in to.
const NEW_SESSION: &str = "garmin-1035-new";

/// The service's own words for a challenge. The platform never shows them.
const SERVICE_WORDS: &str = "Garmin showed a captcha frame on the sign-in page";

/// What one login sent as its prior session: the provider it was exported
/// for, the session id it names, and how many cookies it carried. The cookies
/// themselves are never kept.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PriorSeen {
    provider: String,
    session_id: String,
    cookies: usize,
}

/// Every login the stand-in received, by email: `None` for a login that sent
/// no prior session.
#[derive(Default)]
struct Seen {
    logins: Mutex<Vec<(String, Option<PriorSeen>)>>,
}

static SCRAPER: OnceLock<Arc<Seen>> = OnceLock::new();

/// What the stand-in saw, started once for the binary on a thread with its
/// own runtime, which also points the platform at it.
fn scraper() -> &'static Arc<Seen> {
    SCRAPER.get_or_init(|| {
        let seen = Arc::new(Seen::default());
        let (sender, receiver) = mpsc::channel();
        let state = Arc::clone(&seen);
        thread::spawn(move || {
            let runtime = Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(async move {
                let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
                sender
                    .send(format!("http://{}", listener.local_addr().unwrap()))
                    .unwrap();
                axum::serve(listener, stand_in(state)).await.unwrap();
            });
        });
        let url = receiver.recv().unwrap();
        env::set_var(ENV_REMOTE_URL, &url);
        env::remove_var(ENV_AUDIENCE);
        seen
    })
}

fn challenge() -> Response {
    (
        StatusCode::FORBIDDEN,
        Json(json!({ "error": "login_challenge", "message": SERVICE_WORDS })),
    )
        .into_response()
}

async fn login(State(seen): State<Arc<Seen>>, Json(body): Json<Value>) -> Response {
    let email = body["email"].as_str().unwrap_or_default().to_owned();
    let prior = body.get("prior_session").map(|prior| PriorSeen {
        provider: prior["provider"].as_str().unwrap_or_default().to_owned(),
        session_id: prior["session"]["session_id"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
        cookies: prior["session"]["cookies"].as_array().map_or(0, Vec::len),
    });
    seen.logins.lock().unwrap().push((email.clone(), prior));
    if email.starts_with(CHALLENGED) {
        return challenge();
    }
    if email.starts_with(CODE_STEP) {
        return Json(json!({
            "status": "otp_required",
            "reason": "Garmin sent a verification code",
            "flow_id": "flow-1035",
            "provider": "garmin"
        }))
        .into_response();
    }
    Json(json!({
        "status": "authenticated",
        "session_id": NEW_SESSION,
        "provider": "garmin"
    }))
    .into_response()
}

/// The scraper service stand-in: a Garmin login that records its prior
/// session, a code step stopped at a challenge, and the session the platform
/// exports and reads after a sign-in.
fn stand_in(seen: Arc<Seen>) -> Router {
    Router::new()
        .route("/auth/login-with-credentials", post(login))
        .route("/auth/submit-otp", post(|| async { challenge() }))
        .route(
            "/auth/sessions/{session_id}/export",
            get(|Path(_session_id): Path<String>| async {
                Json(json!({
                    "session": {
                        "session_id": NEW_SESSION,
                        "cookies": [],
                        "created_at": "2026-10-16T12:00:00Z",
                        "expires_at": null
                    }
                }))
            }),
        )
        .route(
            "/auth/import-session",
            post(|| async { Json(json!({ "session_id": NEW_SESSION })) }),
        )
        .route(
            "/api/activities",
            get(|| async { Json(json!({ "count": 0, "activities": [], "head_complete": true })) }),
        )
        .route(
            "/api/athlete",
            get(|| async { Json(json!({ "display_name": "Garmin Tester" })) }),
        )
        .route(
            "/api/daily-summary",
            get(|| async { Json(json!({ "provider": "garmin" })) }),
        )
        .with_state(seen)
}

/// The prior sessions the logins from `email` sent, in order.
fn priors_sent_by(email: &str) -> Vec<Option<PriorSeen>> {
    scraper()
        .logins
        .lock()
        .unwrap()
        .iter()
        .filter(|(from, _)| from == email)
        .map(|(_, prior)| prior.clone())
        .collect()
}

/// A unique email under `prefix`, so tests sharing the stand-in never read
/// each other's logins.
fn email(prefix: &str) -> String {
    format!("{prefix}-{}@example.com", Uuid::new_v4())
}

/// One athlete, with Garmin's notice (if it has one) already accepted.
struct Athlete {
    resources: Arc<ServerContext>,
    user_id: Uuid,
    tenant_id: TenantId,
    bearer: String,
}

impl Athlete {
    async fn new() -> Self {
        scraper();
        let resources = create_test_server_resources().await.unwrap();
        Self::in_resources(resources).await
    }

    /// Another athlete on the same server as `self`.
    async fn neighbour(&self) -> Self {
        Self::in_resources(Arc::clone(&self.resources)).await
    }

    async fn in_resources(resources: Arc<ServerContext>) -> Self {
        let (user_id, user) =
            create_test_user_with_email(&resources.agent.database, &email("athlete"))
                .await
                .unwrap();
        if let Some(version) = provider_terms_version(SCIOTTE_GARMIN) {
            resources
                .common
                .repos
                .users
                .record_provider_terms(user_id, SCIOTTE_GARMIN, version)
                .await
                .unwrap();
        }
        let tenant_id = resources
            .common
            .repos
            .tenants
            .list_for_user(user_id)
            .await
            .unwrap()
            .first()
            .expect("user has a tenant")
            .id;
        let bearer = format!("Bearer {}", generate_test_token(&resources, &user).await);
        Self {
            resources,
            user_id,
            tenant_id,
            bearer,
        }
    }

    /// The same athlete, signing in through a hosted Garmin link instead.
    fn with_link_token(mut self) -> Self {
        let token = mint_link_token(
            &MintProviderLinkTokenArgs {
                user_id: self.user_id,
                tenant_id: self.tenant_id.as_uuid(),
                provider: "sciotte",
                target: "garmin",
                channel: "telegram",
                channel_thread: None,
            },
            &self.resources.auth.admin_jwt_secret,
        )
        .unwrap();
        self.bearer = format!("Bearer {token}");
        self
    }

    async fn post(&self, path: &str, body: Value) -> (u16, Value) {
        let resp = AxumTestRequest::post(path)
            .header("authorization", &self.bearer)
            .json(&body)
            .send(AuthRoutes::routes(self.resources.auth_routes_context()))
            .await;
        let status = resp.status();
        let text = resp.text();
        let body = serde_json::from_str(&text).unwrap_or_else(|_| panic!("{path}: {text}"));
        (status, body)
    }

    async fn login(&self, email: &str) -> (u16, Value) {
        self.post(
            "/api/providers/sciotte/login",
            json!({
                "email": email,
                "password": "not-a-real-password",
                "target": "garmin",
                "tos_consent": true,
            }),
        )
        .await
    }

    async fn garmin_session(&self) -> Option<UserOAuthToken> {
        self.resources
            .common
            .repos
            .oauth_tokens
            .get_token(self.user_id, self.tenant_id, SCIOTTE_GARMIN)
            .await
            .unwrap()
    }

    /// Store a Garmin session for this athlete under `tenant_id`, as a login
    /// stores one, whose expiry has already passed: the re-login case.
    async fn store_expired_session_under(&self, tenant_id: TenantId, session_id: &str) {
        let blob = session_blob(session_id, Some(Utc::now() - chrono::Duration::hours(1)));
        self.store_blob_under(tenant_id, blob).await;
    }

    /// Store a Garmin session for this athlete, written an hour ago (past the
    /// reuse window) and expiring tomorrow: still live at the provider.
    async fn store_old_live_session(&self, session_id: &str) {
        let blob = session_blob(session_id, Some(Utc::now() + chrono::Duration::days(1)));
        self.store_old_blob(blob).await;
    }

    /// Store a Garmin session for this athlete, written an hour ago (past the
    /// reuse window) and carrying no expiry at all: what sciotte stores.
    async fn store_old_session_without_expiry(&self, session_id: &str) {
        self.store_old_blob(session_blob(session_id, None)).await;
    }

    async fn store_old_blob(&self, blob: String) {
        let mut token = self.session_token(self.tenant_id, blob);
        token.updated_at = Utc::now() - chrono::Duration::hours(1);
        self.upsert(&token).await;
    }

    /// Register the athlete's Garmin connection, as the login that stored the
    /// session did. It starts `active`.
    async fn register_garmin_connection(&self) {
        self.resources
            .common
            .repos
            .provider_connections
            .register_connection(
                self.user_id,
                self.tenant_id,
                SCIOTTE_GARMIN,
                &ConnectionType::Manual,
                None,
            )
            .await
            .expect("register the Garmin connection");
    }

    /// The status of the athlete's Garmin connection.
    async fn garmin_connection_status(&self) -> ConnectionStatus {
        self.resources
            .common
            .repos
            .provider_connections
            .get_for_user(self.user_id, Some(self.tenant_id))
            .await
            .unwrap()
            .into_iter()
            .find(|connection| connection.provider == SCIOTTE_GARMIN)
            .expect("the Garmin connection")
            .status
    }

    /// Flag the athlete's Garmin connection as a scrape the provider answered
    /// `session_expired` does.
    async fn flag_garmin_connection_as_a_refused_read_does(&self) {
        let mark = self
            .resources
            .common
            .repos
            .provider_connections
            .mark_needs_reauth(
                self.user_id,
                self.tenant_id,
                SCIOTTE_GARMIN,
                Some("session_expired"),
                Utc::now(),
            )
            .await
            .unwrap();
        assert_eq!(mark, ReauthMark::Flagged, "the connection was flipped");
    }

    async fn store_blob_under(&self, tenant_id: TenantId, blob: String) {
        let token = self.session_token(tenant_id, blob);
        self.upsert(&token).await;
    }

    fn session_token(&self, tenant_id: TenantId, blob: String) -> UserOAuthToken {
        let mut token = UserOAuthToken::new(
            self.user_id,
            tenant_id.to_string(),
            SCIOTTE_GARMIN.to_owned(),
            blob,
            None,
            None,
            None,
        );
        TOKEN_TYPE_SESSION.clone_into(&mut token.token_type);
        token
    }

    async fn upsert(&self, token: &UserOAuthToken) {
        self.resources
            .common
            .repos
            .oauth_tokens
            .upsert_token(token)
            .await
            .expect("store a Garmin session");
    }
}

/// A stored Garmin session blob carrying one device-trust cookie and
/// `expires_at`, `null` when `None`.
fn session_blob(session_id: &str, expires_at: Option<chrono::DateTime<Utc>>) -> String {
    json!({
        "session_id": session_id,
        "cookies": [{
            "name": "GARMIN-SSO-CUST-GUID",
            "value": "device-trust-cookie",
            "domain": ".garmin.com",
            "path": "/",
            "secure": true,
            "http_only": true
        }],
        "created_at": (Utc::now() - chrono::Duration::days(30)).to_rfc3339(),
        "expires_at": expires_at.map(|expires_at| expires_at.to_rfc3339()),
    })
    .to_string()
}

// ── Capture of what the platform logs ──

#[derive(Clone, Debug)]
struct CapturedEvent {
    level: tracing::Level,
    fields: HashMap<String, String>,
}

#[derive(Default)]
struct FieldVisitor {
    fields: HashMap<String, String>,
}

impl Visit for FieldVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn FmtDebug) {
        self.fields
            .insert(field.name().to_owned(), format!("{value:?}"));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.fields
            .insert(field.name().to_owned(), value.to_owned());
    }
}

/// Records every event and keeps no span state.
///
/// A login here runs through sqlx, whose SQLite worker threads drop spans
/// created on the test thread. Under a `tracing_subscriber::Registry`
/// installed with `set_default`, that drop can reach a registry that no
/// longer holds the span and panic the worker ("tried to drop a ref to
/// Id(1), but no such span exists"), failing the login before the handler
/// logs. Handing out span ids without tracking them leaves nothing to drop.
#[derive(Default)]
struct CaptureSubscriber {
    events: Arc<Mutex<Vec<CapturedEvent>>>,
    next_span: AtomicU64,
}

impl Subscriber for CaptureSubscriber {
    fn enabled(&self, _metadata: &Metadata<'_>) -> bool {
        true
    }

    fn new_span(&self, _span: &Attributes<'_>) -> Id {
        Id::from_u64(self.next_span.fetch_add(1, Ordering::Relaxed) + 1)
    }

    fn record(&self, _span: &Id, _values: &Record<'_>) {}

    fn record_follows_from(&self, _span: &Id, _follows: &Id) {}

    fn event(&self, event: &Event<'_>) {
        let mut visitor = FieldVisitor::default();
        event.record(&mut visitor);
        self.events.lock().unwrap().push(CapturedEvent {
            level: *event.metadata().level(),
            fields: visitor.fields,
        });
    }

    fn enter(&self, _span: &Id) {}

    fn exit(&self, _span: &Id) {}
}

fn setup_capture() -> (Arc<Mutex<Vec<CapturedEvent>>>, DefaultGuard) {
    let capture = CaptureSubscriber::default();
    let events = Arc::clone(&capture.events);
    let guard = set_default(capture);
    (events, guard)
}

/// Whether the capture saw the handler's own challenge `WARN` for `stage`,
/// which proves it was listening while the handler ran.
fn saw_challenge_warning(events: &Arc<Mutex<Vec<CapturedEvent>>>, stage: &str) -> bool {
    events.lock().unwrap().iter().any(|e| {
        e.level == tracing::Level::WARN && e.fields.get("stage").map(String::as_str) == Some(stage)
    })
}

/// The `ERROR` events and `sync.failed` business events among `events`.
fn system_failure_signals(events: &Arc<Mutex<Vec<CapturedEvent>>>) -> (usize, usize) {
    let events = events.lock().unwrap();
    let errors = events
        .iter()
        .filter(|e| e.level == tracing::Level::ERROR)
        .count();
    let sync_failed = events
        .iter()
        .filter(|e| e.fields.get("event").map(String::as_str) == Some("sync.failed"))
        .count();
    (errors, sync_failed)
}

/// The strings a hosted page embeds for its script.
fn script_strings(body: &str) -> Value {
    let start = body.find("strings: ").expect("the page embeds its strings") + "strings: ".len();
    let json = &body[start..];
    let end = json.find(",\n").expect("the strings end their line");
    serde_json::from_str(&json[..end]).expect("the embedded strings are JSON")
}

/// The athlete-facing answer to a challenge: `400`, the platform's own copy
/// (never the service's words), and the reason the hosted pages select on.
fn assert_is_the_challenge_refusal(status: u16, body: &Value) {
    assert_eq!(status, 400, "{body}");
    assert_eq!(
        body["message"],
        Value::from(friendly_login_challenge_message(SCIOTTE_GARMIN)),
        "{body}"
    );
    assert_eq!(body["details"]["reason"], LOGIN_CHALLENGE_REASON, "{body}");
    assert!(
        !body.to_string().contains(SERVICE_WORDS),
        "the service's own words stay out of the answer: {body}"
    );
}

/// A challenged sign-in answers the challenge refusal, stores nothing, holds
/// no flow, and emits neither the operator alert nor `sync.failed`.
async fn a_challenge_is_the_athletes_to_retry(athlete: &Athlete) {
    let from = email(CHALLENGED);
    let (events, guard) = setup_capture();
    let (status, body) = athlete.login(&from).await;
    let (errors, sync_failed) = system_failure_signals(&events);
    let warned = saw_challenge_warning(&events, "credential_login");
    drop(guard);

    assert!(warned, "the capture saw the handler's challenge warning");

    assert_is_the_challenge_refusal(status, &body);
    assert_eq!(
        errors, 0,
        "a provider's bot check must not emit the error! dravr-tronc forwards to Slack"
    );
    assert_eq!(
        sync_failed, 0,
        "a challenge is not a sync.failed system failure"
    );
    assert_eq!(
        priors_sent_by(&from).len(),
        1,
        "the login reached the service once"
    );
    assert!(athlete.garmin_session().await.is_none());
    assert!(
        require_remote_flow(
            &athlete.resources.common.cache,
            athlete.tenant_id.as_uuid(),
            athlete.user_id
        )
        .await
        .is_err(),
        "a challenged sign-in parks no flow"
    );
}

#[tokio::test]
async fn a_challenged_sign_in_answers_its_own_copy_and_pages_nobody() {
    a_challenge_is_the_athletes_to_retry(&Athlete::new().await).await;
}

#[tokio::test]
async fn a_challenged_hosted_sign_in_answers_the_reason_its_page_reads() {
    let athlete = Athlete::new().await.with_link_token();
    // The page words itself in the athlete's locale; English is the one the
    // platform's own fallback copy is written in.
    athlete
        .resources
        .common
        .repos
        .users
        .update_locale(athlete.user_id, "en")
        .await
        .expect("store the athlete's locale");
    a_challenge_is_the_athletes_to_retry(&athlete).await;

    // The hosted login page compares the refusal's reason against this exact
    // literal, and carries the challenged sentence naming Garmin. Its script
    // choosing that sentence is run in `hostedLoginCodeStep.test.ts`.
    let token = athlete
        .bearer
        .strip_prefix("Bearer ")
        .expect("a bearer")
        .to_owned();
    let page = AxumTestRequest::get(&format!("/providers/sciotte/login?token={token}"))
        .send(AuthRoutes::routes(athlete.resources.auth_routes_context()))
        .await
        .text();
    assert!(
        page.contains(&format!(
            r#"loginChallengeReason: "{LOGIN_CHALLENGE_REASON}","#
        )),
        "the hosted page knows the challenged-sign-in reason"
    );
    assert_eq!(
        script_strings(&page)["signInChallenged"],
        Value::from(friendly_login_challenge_message(SCIOTTE_GARMIN)),
        "the English page carries the challenged sentence naming Garmin, which the \
         platform's own fallback copy matches"
    );
}

#[tokio::test]
async fn a_challenge_at_the_code_step_ends_the_sign_in() {
    let athlete = Athlete::new().await;
    let (status, login) = athlete.login(&email(CODE_STEP)).await;
    assert_eq!(status, 200, "{login}");
    assert_eq!(login, json!({ "status": "otp_required" }));

    let (events, guard) = setup_capture();
    let (status, body) = athlete
        .post(
            "/api/providers/sciotte/submit-otp",
            json!({ "code": "246810" }),
        )
        .await;
    let (errors, sync_failed) = system_failure_signals(&events);
    let warned = saw_challenge_warning(&events, "otp");
    drop(guard);

    assert!(warned, "the capture saw the handler's challenge warning");

    assert_is_the_challenge_refusal(status, &body);
    assert_eq!((errors, sync_failed), (0, 0), "no system-failure signal");
    assert!(
        require_remote_flow(
            &athlete.resources.common.cache,
            athlete.tenant_id.as_uuid(),
            athlete.user_id
        )
        .await
        .is_err(),
        "the service stopped the login, so the platform holds no flow for it"
    );
}

#[tokio::test]
async fn a_re_login_after_expiry_sends_the_stored_session_as_prior() {
    let athlete = Athlete::new().await;
    athlete
        .store_expired_session_under(athlete.tenant_id, "garmin-1035-prior")
        .await;

    let from = email("relogin");
    let (status, body) = athlete.login(&from).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["status"], "connected", "{body}");

    assert_eq!(
        priors_sent_by(&from),
        [Some(PriorSeen {
            provider: "garmin".to_owned(),
            session_id: "garmin-1035-prior".to_owned(),
            cookies: 1,
        })],
        "the re-login seeds the browser with the session it replaces"
    );
    let stored = athlete.garmin_session().await.expect("the new session");
    assert!(
        stored.access_token.contains(NEW_SESSION),
        "the sign-in replaced the stored session"
    );
}

#[tokio::test]
async fn a_session_without_expiry_whose_connection_needs_reauth_is_sent_as_prior() {
    // What production stores: sciotte never sets `expires_at`. A scrape the
    // provider answered `session_expired` flagged the connection, so the
    // session's sign-in cookies are dead and only its device trust is left.
    let athlete = Athlete::new().await;
    athlete
        .store_old_session_without_expiry("garmin-1035-flagged")
        .await;
    athlete.register_garmin_connection().await;
    athlete
        .flag_garmin_connection_as_a_refused_read_does()
        .await;

    let from = email("flagged");
    let (status, body) = athlete.login(&from).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["status"], "connected", "{body}");
    assert_eq!(
        priors_sent_by(&from),
        [Some(PriorSeen {
            provider: "garmin".to_owned(),
            session_id: "garmin-1035-flagged".to_owned(),
            cookies: 1,
        })],
        "a session the provider refused seeds the browser with its device trust"
    );
    let stored = athlete.garmin_session().await.expect("the new session");
    assert!(
        stored.access_token.contains(NEW_SESSION),
        "the sign-in replaced the stored session"
    );
}

#[tokio::test]
async fn a_session_without_expiry_on_an_active_connection_is_not_sent_as_prior() {
    // The same session, but nothing has seen it fail: its sign-in cookies may
    // be live, and this login may be a coach switching accounts.
    let athlete = Athlete::new().await;
    athlete
        .store_old_session_without_expiry("garmin-1035-unflagged")
        .await;
    athlete.register_garmin_connection().await;
    assert_eq!(
        athlete.garmin_connection_status().await,
        ConnectionStatus::Active
    );

    let from = email("unflagged");
    let (status, body) = athlete.login(&from).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["status"], "connected", "{body}");
    assert_eq!(
        priors_sent_by(&from),
        [None],
        "a session not observed dead never seeds the login browser"
    );
}

#[tokio::test]
async fn an_old_session_that_has_not_expired_is_not_sent_as_prior() {
    // A coach connected account A an hour ago and now signs in to account B.
    // A's session is past the reuse window, has not expired, and its
    // connection is active: seeding the browser with it could skip Garmin's
    // sign-in form and store A's session under B's credentials.
    let athlete = Athlete::new().await;
    athlete.store_old_live_session("garmin-1035-live").await;
    athlete.register_garmin_connection().await;
    assert_eq!(
        athlete.garmin_connection_status().await,
        ConnectionStatus::Active
    );

    let from = email("switch");
    let (status, body) = athlete.login(&from).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["status"], "connected", "{body}");
    assert_eq!(
        priors_sent_by(&from),
        [None],
        "a session not observed dead never seeds the login browser"
    );
    let stored = athlete.garmin_session().await.expect("the new session");
    assert!(
        stored.access_token.contains(NEW_SESSION),
        "the sign-in replaced the stored session"
    );
}

#[tokio::test]
async fn a_fresh_connect_sends_no_prior_session() {
    let athlete = Athlete::new().await;
    let from = email("fresh");
    let (status, body) = athlete.login(&from).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["status"], "connected", "{body}");
    assert_eq!(priors_sent_by(&from), [None]);
}

#[tokio::test]
async fn an_unreadable_stored_session_is_not_sent_as_prior() {
    let athlete = Athlete::new().await;
    athlete
        .store_blob_under(athlete.tenant_id, "not a session".to_owned())
        .await;
    let from = email("unreadable");
    let (status, body) = athlete.login(&from).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(priors_sent_by(&from), [None]);
}

#[tokio::test]
async fn another_athletes_stored_session_is_never_sent_as_prior() {
    let owner = Athlete::new().await;
    let other = owner.neighbour().await;
    // The owner's session under the owner's own tenant, and one filed under
    // the other athlete's tenant: neither is the other athlete's.
    owner
        .store_expired_session_under(owner.tenant_id, "owner-own-tenant")
        .await;
    owner
        .store_expired_session_under(other.tenant_id, "owner-in-other-tenant")
        .await;

    let from = email("other");
    let (status, body) = other.login(&from).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        priors_sent_by(&from),
        [None],
        "a login is seeded only by the caller's own session"
    );

    // The owner's own re-login still finds the session under its own tenant.
    let from = email("owner");
    let (status, body) = owner.login(&from).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        priors_sent_by(&from)
            .first()
            .cloned()
            .flatten()
            .map(|prior| prior.session_id),
        Some("owner-own-tenant".to_owned())
    );
}
