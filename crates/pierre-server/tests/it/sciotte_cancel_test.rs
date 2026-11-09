// ABOUTME: carnet#1300 — closing a sign-in window cancels its login on the scraper by a key bound to the caller
// ABOUTME: Pins the key on every step, the per-user binding, the 400, the quiet 409, the released flow, and the cancelled attempt refused on any instance

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A closed sign-in window aborts its request, but on a deployed stack that
//! disconnect never crosses the hops in front of the scraper service, so the
//! login ran on with its browser and its permit. The window now names each
//! opening with an attempt id, and on close posts
//! `POST /api/providers/sciotte/cancel` naming it. The platform keeps no
//! registry: it derives a cancel key — the hex SHA-256 of
//! `"{tenant}:{user}:{attempt}"` — sends it on every step in
//! `x-sciotte-cancel-key`, and asks the service to cancel the same key.
//! The cancel also files the attempt as cancelled in the shared cache, so a
//! step that ran on a service instance the cancel never reached — a second
//! instance a cold start brought up — still links nothing.
//!
//! The scraper is a loopback stand-in (a test double, per the repo's mock
//! rule) recording the keys each step and each cancel named. It runs on a
//! thread of its own, because each `#[tokio::test]` drops its runtime when it
//! ends and a stand-in spawned on one would die with the first test to finish.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;
use crate::helpers;

use std::collections::HashMap;
use std::fmt::Debug as FmtDebug;
use std::net::TcpListener as StdTcpListener;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::thread;
use std::time::Duration;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use common::{create_test_user_with_email, generate_test_token};
use helpers::axum_test::AxumTestRequest;
use pierre_core::constants::oauth::providers::provider_terms_version;
use pierre_core::constants::oauth_providers::SCIOTTE_GARMIN;
use pierre_core::models::TenantId;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_middleware::provider_link_token::{mint_link_token, MintProviderLinkTokenArgs};
use pierre_routes_auth::{remember_remote_flow, require_remote_flow, AuthRoutes};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::net::TcpListener;
use tokio::runtime::Builder;
use tokio::time::sleep;
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::subscriber::{set_default, DefaultGuard};
use tracing::{Event, Metadata, Subscriber};
use uuid::Uuid;

/// The header a step names its cancel key in — a wire value the scraper
/// service parses (`dravr_sciotte::wire::HEADER_CANCEL_KEY`).
const CANCEL_KEY_HEADER: &str = "x-sciotte-cancel-key";

/// `details.reason` of a step the cancel ended — the scraper service's own
/// wire marker (`dravr_sciotte::wire::LOGIN_CANCELLED`).
const LOGIN_CANCELLED_REASON: &str = "login_cancelled";

/// The service's three sign-in step paths.
const LOGIN_PATH: &str = "/auth/login-with-credentials";
const SELECT_2FA_PATH: &str = "/auth/select-2fa";
const SUBMIT_OTP_PATH: &str = "/auth/submit-otp";

/// Logins whose email starts with this are ended by a cancel.
const CANCELLED: &str = "cancelled";

/// Logins whose email starts with this run on an instance the cancel never
/// reaches: each waits for a cancel of its key to arrive, then signs in.
const OUTLIVED: &str = "outlived";

/// The session an outliving login signs in with.
const OUTLIVED_SESSION: &str = "session-outlived-1300";

/// What the stand-in received.
#[derive(Default)]
struct Seen {
    /// Every login, by email, with the cancel key its header named.
    logins: Mutex<Vec<(String, Option<String>)>>,
    /// Every step, by service path, with the cancel key its header named.
    steps: Mutex<Vec<(&'static str, Option<String>)>>,
    /// Every cancel key a cancel named, in order.
    cancels: Mutex<Vec<String>>,
    /// What a cancel of a key answers; `(false, false)` for any other key.
    outcomes: Mutex<HashMap<String, (bool, bool)>>,
}

static SCRAPER: OnceLock<(Arc<Seen>, String)> = OnceLock::new();

/// What the stand-in saw and the URL it serves at, started once for the
/// binary on a thread with its own runtime.
fn scraper() -> &'static (Arc<Seen>, String) {
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
        (seen, receiver.recv().unwrap())
    })
}

fn cancelled_step() -> Response {
    (
        StatusCode::CONFLICT,
        Json(json!({ "error": "login_cancelled", "message": "the caller cancelled this login" })),
    )
        .into_response()
}

fn header_key(headers: &HeaderMap) -> Option<String> {
    headers
        .get(CANCEL_KEY_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

async fn login(
    State(seen): State<Arc<Seen>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let email = body["email"].as_str().unwrap_or_default().to_owned();
    seen.steps
        .lock()
        .unwrap()
        .push((LOGIN_PATH, header_key(&headers)));
    seen.logins
        .lock()
        .unwrap()
        .push((email.clone(), header_key(&headers)));
    if email.starts_with(CANCELLED) {
        return cancelled_step();
    }
    if email.starts_with(OUTLIVED) {
        let key = header_key(&headers).unwrap_or_default();
        for _ in 0..200 {
            if seen.cancels.lock().unwrap().contains(&key) {
                break;
            }
            sleep(Duration::from_millis(10)).await;
        }
        return Json(json!({
            "status": "authenticated",
            "session_id": OUTLIVED_SESSION,
            "provider": "garmin"
        }))
        .into_response();
    }
    Json(json!({
        "status": "otp_required",
        "reason": "Garmin sent a verification code",
        "flow_id": "flow-1300",
        "provider": "garmin"
    }))
    .into_response()
}

async fn cancel(State(seen): State<Arc<Seen>>, Json(body): Json<Value>) -> Json<Value> {
    let key = body["cancel_key"].as_str().unwrap_or_default().to_owned();
    seen.cancels.lock().unwrap().push(key.clone());
    let (cancelled_running, released_parked) = seen
        .outcomes
        .lock()
        .unwrap()
        .get(&key)
        .copied()
        .unwrap_or_default();
    Json(json!({ "cancelled_running": cancelled_running, "released_parked": released_parked }))
}

/// A continuation step (a 2FA pick or a code) that records its cancel key
/// under `path` and is ended by a cancel.
fn continuation(seen: &Seen, path: &'static str, headers: &HeaderMap) -> Response {
    seen.steps.lock().unwrap().push((path, header_key(headers)));
    cancelled_step()
}

async fn select_2fa(State(seen): State<Arc<Seen>>, headers: HeaderMap) -> Response {
    continuation(&seen, SELECT_2FA_PATH, &headers)
}

async fn submit_otp(State(seen): State<Arc<Seen>>, headers: HeaderMap) -> Response {
    continuation(&seen, SUBMIT_OTP_PATH, &headers)
}

/// The scraper service stand-in: a Garmin login, a 2FA pick and a code step
/// that record their cancel key (the two continuations ended by a cancel),
/// and the cancel itself.
fn stand_in(seen: Arc<Seen>) -> Router {
    Router::new()
        .route(LOGIN_PATH, post(login))
        .route(SELECT_2FA_PATH, post(select_2fa))
        .route(SUBMIT_OTP_PATH, post(submit_otp))
        .route("/auth/cancel-login", post(cancel))
        .with_state(seen)
}

/// The service paths of every step that named `key`, in arrival order.
fn steps_keyed(key: &str) -> Vec<&'static str> {
    scraper()
        .0
        .steps
        .lock()
        .unwrap()
        .iter()
        .filter(|(_, sent)| sent.as_deref() == Some(key))
        .map(|(path, _)| *path)
        .collect()
}

/// How many cancels named `key`.
fn cancels_of(key: &str) -> usize {
    scraper()
        .0
        .cancels
        .lock()
        .unwrap()
        .iter()
        .filter(|sent| *sent == key)
        .count()
}

/// The cancel keys the logins from `email` named.
fn login_keys_of(email: &str) -> Vec<Option<String>> {
    scraper()
        .0
        .logins
        .lock()
        .unwrap()
        .iter()
        .filter(|(from, _)| from == email)
        .map(|(_, key)| key.clone())
        .collect()
}

/// Make a cancel of `key` answer `(cancelled_running, released_parked)`.
fn answer_cancel_of(key: &str, outcome: (bool, bool)) {
    scraper()
        .0
        .outcomes
        .lock()
        .unwrap()
        .insert(key.to_owned(), outcome);
}

/// A unique email under `prefix`, so tests sharing the stand-in never read
/// each other's logins.
fn email(prefix: &str) -> String {
    format!("{prefix}-{}@example.com", Uuid::new_v4())
}

/// A fresh attempt id, as a login window mints one.
fn attempt() -> String {
    Uuid::new_v4().to_string()
}

/// The key the platform must derive, computed here independently of it.
fn expected_key(tenant_id: TenantId, user_id: Uuid, attempt_id: &str) -> String {
    hex::encode(Sha256::digest(format!(
        "{}:{user_id}:{attempt_id}",
        tenant_id.as_uuid()
    )))
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
        Self::on(&scraper().1).await
    }

    /// An athlete on a server whose scraper lives at `url`.
    async fn on(url: &str) -> Self {
        let resources = common::create_test_server_resources_with_config(
            common::test_server_config_with_sciotte_usr_pwd_garmin(url),
        )
        .await
        .unwrap();
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

    async fn cancel(&self, attempt_id: &str) -> (u16, Value) {
        self.post(
            "/api/providers/sciotte/cancel",
            json!({ "attempt_id": attempt_id }),
        )
        .await
    }

    async fn login(&self, email: &str, attempt_id: &str) -> (u16, Value) {
        self.post(
            "/api/providers/sciotte/login",
            json!({
                "email": email,
                "password": "not-a-real-password",
                "target": "garmin",
                "tos_consent": true,
                "attempt_id": attempt_id,
            }),
        )
        .await
    }

    async fn remember_flow(&self, flow_id: &str) {
        remember_remote_flow(
            &self.resources.common.cache,
            self.tenant_id.as_uuid(),
            self.user_id,
            flow_id.to_owned(),
            SCIOTTE_GARMIN,
        )
        .await;
    }

    async fn holds_garmin_session(&self) -> bool {
        self.resources
            .common
            .repos
            .oauth_tokens
            .get_token(self.user_id, self.tenant_id, SCIOTTE_GARMIN)
            .await
            .unwrap()
            .is_some()
    }

    async fn holds_a_flow(&self) -> bool {
        require_remote_flow(
            &self.resources.common.cache,
            self.tenant_id.as_uuid(),
            self.user_id,
        )
        .await
        .is_ok()
    }
}

// ── The cancel key ──

#[tokio::test]
async fn a_cancel_names_the_callers_own_key_to_the_service() {
    let athlete = Athlete::new().await;
    let attempt_id = attempt();
    let key = expected_key(athlete.tenant_id, athlete.user_id, &attempt_id);
    answer_cancel_of(&key, (true, false));

    let (status, body) = athlete.cancel(&attempt_id).await;

    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body,
        json!({ "cancelled": true }),
        "the attempt is cancelled"
    );
    assert_eq!(
        cancels_of(&key),
        1,
        "the service was asked to cancel the key"
    );
}

#[tokio::test]
async fn another_users_attempt_id_names_another_key() {
    let athlete = Athlete::new().await;
    let other = athlete.neighbour().await;
    let attempt_id = attempt();
    let own_key = expected_key(athlete.tenant_id, athlete.user_id, &attempt_id);
    answer_cancel_of(&own_key, (true, true));
    athlete.remember_flow("athletes-parked-flow").await;

    let (status, body) = other.cancel(&attempt_id).await;

    let other_key = expected_key(other.tenant_id, other.user_id, &attempt_id);
    assert_ne!(
        other_key, own_key,
        "the key binds the attempt to its caller"
    );
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        cancels_of(&other_key),
        1,
        "the neighbour's own key was sent"
    );
    assert_eq!(cancels_of(&own_key), 0, "the athlete's key was never sent");
    assert!(
        athlete.holds_a_flow().await,
        "the athlete's flow is untouched"
    );
    let from = email("neighbour-cancelled");
    let (status, body) = athlete.login(&from, &attempt_id).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        login_keys_of(&from),
        vec![Some(own_key)],
        "the neighbour's cancel refuses none of the athlete's steps"
    );
}

#[tokio::test]
async fn every_step_names_the_key_its_attempt_derives() {
    let athlete = Athlete::new().await;
    let attempt_id = attempt();
    let key = expected_key(athlete.tenant_id, athlete.user_id, &attempt_id);
    let from = email("keyed");

    let (status, body) = athlete.login(&from, &attempt_id).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["status"], "otp_required", "{body}");
    assert_eq!(
        login_keys_of(&from),
        vec![Some(key.clone())],
        "the login named its attempt's key"
    );

    // The stand-in ends each continuation as a cancel would, which drops the
    // remembered flow, so park one again before each.
    athlete.remember_flow("flow-1300").await;
    let (status, body) = athlete
        .post(
            "/api/providers/sciotte/select-2fa",
            json!({ "option_id": "email", "attempt_id": attempt_id }),
        )
        .await;
    assert_eq!(status, 409, "{body}");

    athlete.remember_flow("flow-1300").await;
    let (status, body) = athlete
        .post(
            "/api/providers/sciotte/submit-otp",
            json!({ "code": "123456", "attempt_id": attempt_id }),
        )
        .await;
    assert_eq!(status, 409, "{body}");

    assert_eq!(
        steps_keyed(&key),
        vec![LOGIN_PATH, SELECT_2FA_PATH, SUBMIT_OTP_PATH],
        "all three steps named the same key"
    );
}

#[tokio::test]
async fn a_step_without_an_attempt_names_no_key() {
    let athlete = Athlete::new().await;
    let from = email("unkeyed");

    let (status, body) = athlete
        .post(
            "/api/providers/sciotte/login",
            json!({
                "email": from,
                "password": "not-a-real-password",
                "target": "garmin",
                "tos_consent": true,
            }),
        )
        .await;

    assert_eq!(status, 200, "{body}");
    assert_eq!(
        login_keys_of(&from),
        vec![None],
        "a step naming no attempt sends no cancel key, as before"
    );
}

#[tokio::test]
async fn a_malformed_attempt_id_is_refused_before_the_service() {
    let athlete = Athlete::new().await;
    let from = email("malformed");
    for malformed in [
        "short",
        "has space in it",
        "under_score_id",
        &"a".repeat(65),
    ] {
        let (status, body) = athlete.cancel(malformed).await;
        assert_eq!(status, 400, "cancel {malformed:?}: {body}");
        assert_eq!(body["code"], "InvalidInput", "{body}");

        let (status, body) = athlete.login(&from, malformed).await;
        assert_eq!(status, 400, "login {malformed:?}: {body}");
    }
    assert!(
        login_keys_of(&from).is_empty(),
        "no login with a malformed attempt id reached the service"
    );
}

// ── A step the cancel ended ──

#[tokio::test]
async fn a_step_the_cancel_ended_is_a_quiet_409() {
    let athlete = Athlete::new().await;
    let attempt_id = attempt();
    let from = email(CANCELLED);

    let (events, guard) = setup_capture();
    let (status, body) = athlete.login(&from, &attempt_id).await;
    let (errors, sync_failed) = system_failure_signals(&events);
    let noted = saw_cancel_note(&events, "credential_login");
    drop(guard);

    assert!(noted, "the capture saw the handler's cancel note");
    assert_eq!(status, 409, "{body}");
    assert_eq!(body["details"]["reason"], LOGIN_CANCELLED_REASON, "{body}");
    assert_eq!(errors, 0, "a cancelled login is no operator alert");
    assert_eq!(sync_failed, 0, "a cancelled login is not a sync.failed");
    assert!(
        !athlete.holds_a_flow().await,
        "a cancelled login holds no flow"
    );
}

#[tokio::test]
async fn a_code_step_the_cancel_ended_drops_the_flow_quietly() {
    let athlete = Athlete::new().await;
    athlete.remember_flow("flow-1300").await;

    let (events, guard) = setup_capture();
    let (status, body) = athlete
        .post(
            "/api/providers/sciotte/submit-otp",
            json!({ "code": "123456", "attempt_id": attempt() }),
        )
        .await;
    let (errors, sync_failed) = system_failure_signals(&events);
    let noted = saw_cancel_note(&events, "otp");
    drop(guard);

    assert!(noted, "the capture saw the handler's cancel note");
    assert_eq!(status, 409, "{body}");
    assert_eq!(body["details"]["reason"], LOGIN_CANCELLED_REASON, "{body}");
    assert_eq!((errors, sync_failed), (0, 0), "no alert, no sync.failed");
    assert!(
        !athlete.holds_a_flow().await,
        "the cancelled flow is forgotten"
    );
}

// ── Best effort, and the released flow ──

#[tokio::test]
async fn a_cancel_that_cannot_reach_the_service_still_refuses_the_attempt() {
    // A port nothing listens on: bound, read, then released.
    let unbound = {
        let listener = StdTcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap()
    };
    let athlete = Athlete::on(&format!("http://{unbound}")).await;
    let attempt_id = attempt();

    let (events, guard) = setup_capture();
    let (status, body) = athlete.cancel(&attempt_id).await;
    let (errors, sync_failed) = system_failure_signals(&events);
    drop(guard);

    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body,
        json!({ "cancelled": true }),
        "the platform refuses the attempt whether or not the service heard"
    );
    assert_eq!((errors, sync_failed), (0, 0), "a cancel never alerts");

    let (status, body) = athlete.login(&email("unreached"), &attempt_id).await;
    assert_eq!(status, 409, "{body}");
    assert_eq!(body["details"]["reason"], LOGIN_CANCELLED_REASON, "{body}");
}

// ── A cancel the service instance running the step never saw ──

#[tokio::test]
async fn a_step_after_its_cancel_never_reaches_the_service() {
    let athlete = Athlete::new().await;
    let attempt_id = attempt();
    let key = expected_key(athlete.tenant_id, athlete.user_id, &attempt_id);

    let (status, body) = athlete.cancel(&attempt_id).await;
    assert_eq!(status, 200, "{body}");

    let from = email("after-cancel");
    let (status, body) = athlete.login(&from, &attempt_id).await;
    assert_eq!(status, 409, "{body}");
    assert_eq!(body["details"]["reason"], LOGIN_CANCELLED_REASON, "{body}");

    athlete.remember_flow("flow-1300").await;
    let (status, body) = athlete
        .post(
            "/api/providers/sciotte/submit-otp",
            json!({ "code": "123456", "attempt_id": attempt_id }),
        )
        .await;
    assert_eq!(status, 409, "{body}");
    assert_eq!(body["details"]["reason"], LOGIN_CANCELLED_REASON, "{body}");

    assert!(
        login_keys_of(&from).is_empty(),
        "the login never reached the service"
    );
    assert!(
        steps_keyed(&key).is_empty(),
        "no step of the cancelled attempt reached the service"
    );
}

#[tokio::test]
async fn a_step_that_outlived_its_cancel_links_nothing() {
    let athlete = Athlete::new().await;
    let attempt_id = attempt();
    let key = expected_key(athlete.tenant_id, athlete.user_id, &attempt_id);
    let from = email(OUTLIVED);

    let (events, guard) = setup_capture();
    let login = athlete.login(&from, &attempt_id);
    let cancel = async {
        while login_keys_of(&from).is_empty() {
            sleep(Duration::from_millis(10)).await;
        }
        athlete.cancel(&attempt_id).await
    };
    let ((status, body), (cancel_status, cancel_body)) = tokio::join!(login, cancel);
    let (errors, sync_failed) = system_failure_signals(&events);
    drop(guard);

    assert_eq!(cancel_status, 200, "{cancel_body}");
    assert_eq!(
        cancel_body,
        json!({ "cancelled": true }),
        "the cancel filed the attempt"
    );
    assert_eq!(status, 409, "{body}");
    assert_eq!(body["details"]["reason"], LOGIN_CANCELLED_REASON, "{body}");
    assert_eq!((errors, sync_failed), (0, 0), "no alert, no sync.failed");
    assert!(
        !athlete.holds_garmin_session().await,
        "the sign-in the service finished stored no session"
    );
    assert!(
        !athlete.holds_a_flow().await,
        "the sign-in the service finished parked no flow"
    );
    assert_eq!(
        cancels_of(&key),
        2,
        "the window's cancel, then the release once the step answered"
    );
}

#[tokio::test]
async fn a_released_parked_flow_is_forgotten() {
    let athlete = Athlete::new().await;
    athlete.remember_flow("flow-parked-on-a-code").await;
    let attempt_id = attempt();
    answer_cancel_of(
        &expected_key(athlete.tenant_id, athlete.user_id, &attempt_id),
        (false, true),
    );

    let (status, body) = athlete.cancel(&attempt_id).await;

    assert_eq!(status, 200, "{body}");
    assert_eq!(body, json!({ "cancelled": true }));
    assert!(
        !athlete.holds_a_flow().await,
        "a continuation must not name the flow the service let go"
    );
}

#[tokio::test]
async fn a_cancel_that_released_nothing_parked_keeps_the_flow() {
    let athlete = Athlete::new().await;
    athlete.remember_flow("flow-still-parked").await;
    let attempt_id = attempt();
    answer_cancel_of(
        &expected_key(athlete.tenant_id, athlete.user_id, &attempt_id),
        (true, false),
    );

    let (status, body) = athlete.cancel(&attempt_id).await;

    assert_eq!(status, 200, "{body}");
    assert_eq!(body, json!({ "cancelled": true }));
    assert!(
        athlete.holds_a_flow().await,
        "only a released flow is forgotten"
    );
}

// ── The hosted sign-in pages ──

/// The hosted pages sign in with their link-token, so the cancel they send
/// on `pagehide` carries it too: it must reach the route and the caller's key.
#[tokio::test]
async fn a_hosted_pages_link_token_reaches_the_cancel() {
    let athlete = Athlete::new().await;
    let token = mint_link_token(
        &MintProviderLinkTokenArgs {
            user_id: athlete.user_id,
            tenant_id: athlete.tenant_id.as_uuid(),
            provider: "sciotte",
            target: "garmin",
            channel: "telegram",
            channel_thread: None,
        },
        &athlete.resources.auth.admin_jwt_secret,
    )
    .unwrap();
    let hosted = Athlete {
        bearer: format!("Bearer {token}"),
        ..athlete
    };
    let attempt_id = attempt();
    let key = expected_key(hosted.tenant_id, hosted.user_id, &attempt_id);
    answer_cancel_of(&key, (true, false));

    let (status, body) = hosted.cancel(&attempt_id).await;

    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body,
        json!({ "cancelled": true }),
        "the link-token's caller key"
    );
    assert_eq!(cancels_of(&key), 1, "the hosted caller's key was sent");
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

/// Records every event and keeps no span state: sqlx's worker threads drop
/// spans created on the test thread, which a span-tracking registry
/// installed with `set_default` can panic on (see
/// `sciotte_login_challenge_test`).
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

/// Whether the capture saw the handler's own cancel note for `stage`, which
/// proves it was listening while the handler ran.
fn saw_cancel_note(events: &Arc<Mutex<Vec<CapturedEvent>>>, stage: &str) -> bool {
    events.lock().unwrap().iter().any(|e| {
        e.level == tracing::Level::INFO && e.fields.get("stage").map(String::as_str) == Some(stage)
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
