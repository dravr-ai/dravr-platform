// ABOUTME: carnet#1504 — every step of a sciotte sign-in reaches the sign-in service, reads stay on the read service
// ABOUTME: Pins the routing across two loopback stand-ins, the one-service fallback, and a lost parked sign-in

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A sign-in parks its browser in one sciotte instance's memory while the
//! athlete fetches a code, so a deployment runs sign-ins on their own service,
//! capped at one instance, and reads on another (carnet#1504). Every step of a
//! sign-in — the login, its code step, the export of the session it signed
//! in to, and the drop of that copy — must reach the sign-in service, and
//! nothing of it the read service.
//!
//! The two services here are loopback stand-ins (test doubles, per the repo's
//! mock rule) that record every request they answer. Each runs on a thread of
//! its own, because each `#[tokio::test]` drops its runtime when it ends and a
//! stand-in spawned on one would die with the first test to finish.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;
use crate::helpers;

use std::sync::{mpsc, Arc, Mutex};
use std::thread;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use common::{
    create_test_server_resources_with_config, create_test_user, generate_test_token,
    test_server_config_with_sciotte, test_server_config_with_sciotte_planes,
};
use helpers::axum_test::AxumTestRequest;
use pierre_config::environment::ServerConfig;
use pierre_core::constants::oauth::providers::provider_terms_version;
use pierre_core::constants::oauth_providers::SCIOTTE_COROS;
use pierre_core::models::TenantId;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_routes_auth::{require_remote_flow, AuthRoutes, LOGIN_FLOW_EXPIRED_REASON};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::runtime::Builder;
use uuid::Uuid;

/// The code the stand-in signs in on.
const ACCEPTED_CODE: &str = "246810";

/// A code that reaches an instance that no longer holds the parked browser:
/// the sign-in service scaled to zero while the athlete fetched the code.
const LOST_CODE: &str = "135791";

/// An account the stand-in offers a choice of second factor for.
const TWO_FACTOR_PREFIX: &str = "two-factor";

/// The session the accepted code signs in to.
const SESSION: &str = "coros-1504";

/// Every request one stand-in answered, as `METHOD path`, shared between the
/// stand-in's handlers on its own thread and the test that reads it.
type Seen = Arc<Mutex<Vec<String>>>;

/// A scraper stand-in started on a thread of its own: its base URL and what
/// it saw.
fn stand_in() -> (String, Seen) {
    let seen = Seen::default();
    let state = Arc::clone(&seen);
    let (sender, receiver) = mpsc::channel();
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
            axum::serve(listener, routes(state)).await.unwrap();
        });
    });
    (receiver.recv().unwrap(), seen)
}

fn note(seen: &Seen, request: &str) {
    seen.lock().unwrap().push(request.to_owned());
}

async fn login(State(seen): State<Seen>, Json(body): Json<Value>) -> Response {
    note(&seen, "POST /auth/login-with-credentials");
    if body["email"]
        .as_str()
        .is_some_and(|email| email.starts_with(TWO_FACTOR_PREFIX))
    {
        return Json(json!({
            "status": "two_factor_choice",
            "options": [{ "id": "sms", "label": "Text message" }],
            "flow_id": "flow-1504-2fa",
            "provider": "coros"
        }))
        .into_response();
    }
    Json(json!({
        "status": "otp_required",
        "reason": "COROS sent a verification code",
        "flow_id": "flow-1504",
        "provider": "coros"
    }))
    .into_response()
}

async fn submit_otp(State(seen): State<Seen>, Json(body): Json<Value>) -> Response {
    note(&seen, "POST /auth/submit-otp");
    if body["code"] == LOST_CODE {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "no_pending_login" })),
        )
            .into_response();
    }
    Json(json!({
        "status": "authenticated",
        "session_id": SESSION,
        "provider": "coros"
    }))
    .into_response()
}

async fn select_two_factor(State(seen): State<Seen>) -> Response {
    note(&seen, "POST /auth/select-2fa");
    Json(json!({
        "status": "authenticated",
        "session_id": SESSION,
        "provider": "coros"
    }))
    .into_response()
}

async fn export(State(seen): State<Seen>) -> Response {
    note(&seen, "GET /auth/sessions/:id/export");
    Json(json!({
        "provider": "coros",
        "session": {
            "session_id": SESSION,
            "cookies": [],
            "created_at": "2026-11-15T19:00:00Z",
            "expires_at": null
        }
    }))
    .into_response()
}

async fn drop_session(State(seen): State<Seen>, Path(id): Path<String>) -> Response {
    note(&seen, "DELETE /auth/sessions/:id");
    Json(json!({ "status": "removed", "session_id": id })).into_response()
}

async fn import(State(seen): State<Seen>) -> Response {
    note(&seen, "POST /auth/import-session");
    Json(json!({ "session_id": SESSION })).into_response()
}

async fn activities(State(seen): State<Seen>) -> Response {
    note(&seen, "GET /api/activities");
    Json(json!({ "count": 0, "activities": [], "head_complete": true })).into_response()
}

async fn daily_summary(State(seen): State<Seen>) -> Response {
    note(&seen, "GET /api/daily-summary");
    Json(json!({ "provider": "coros" })).into_response()
}

fn routes(seen: Seen) -> Router {
    Router::new()
        .route("/auth/login-with-credentials", post(login))
        .route("/auth/submit-otp", post(submit_otp))
        .route("/auth/select-2fa", post(select_two_factor))
        .route("/auth/sessions/{session_id}/export", get(export))
        .route("/auth/sessions/{session_id}", delete(drop_session))
        .route("/auth/import-session", post(import))
        .route("/api/activities", get(activities))
        .route("/api/daily-summary", get(daily_summary))
        .with_state(seen)
}

/// The sign-in steps, as the service that runs them sees them.
const SIGN_IN_STEPS: [&str; 4] = [
    "POST /auth/login-with-credentials",
    "POST /auth/submit-otp",
    "GET /auth/sessions/:id/export",
    "DELETE /auth/sessions/:id",
];

/// One athlete with COROS's notice accepted, on a server built from `config`.
struct Athlete {
    resources: Arc<ServerContext>,
    user_id: Uuid,
    tenant_id: TenantId,
    bearer: String,
}

impl Athlete {
    async fn new(config: ServerConfig) -> Self {
        let resources = create_test_server_resources_with_config(config)
            .await
            .unwrap();
        let (user_id, user) = create_test_user(&resources.agent.database).await.unwrap();
        if let Some(version) = provider_terms_version(SCIOTTE_COROS) {
            resources
                .common
                .repos
                .users
                .record_provider_terms(user_id, SCIOTTE_COROS, version)
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

    async fn sign_in(&self, code: &str) -> (u16, Value) {
        let (status, login) = self
            .post(
                "/api/providers/sciotte/login",
                json!({
                    "email": "athlete@example.com",
                    "password": "not-a-real-password",
                    "target": "coros",
                    "tos_consent": true,
                }),
            )
            .await;
        assert_eq!(status, 200, "{login}");
        assert_eq!(login["status"], "otp_required", "{login}");
        self.post("/api/providers/sciotte/submit-otp", json!({ "code": code }))
            .await
    }

    async fn sign_in_by_second_factor(&self) -> (u16, Value) {
        let (status, login) = self
            .post(
                "/api/providers/sciotte/login",
                json!({
                    "email": format!("{TWO_FACTOR_PREFIX}@example.com"),
                    "password": "not-a-real-password",
                    "target": "coros",
                    "tos_consent": true,
                }),
            )
            .await;
        assert_eq!(status, 200, "{login}");
        assert_eq!(login["status"], "two_factor_choice", "{login}");
        self.post(
            "/api/providers/sciotte/select-2fa",
            json!({ "option_id": "sms" }),
        )
        .await
    }

    async fn has_coros_session(&self) -> bool {
        self.resources
            .common
            .repos
            .oauth_tokens
            .get_token(self.user_id, self.tenant_id, SCIOTTE_COROS)
            .await
            .unwrap()
            .is_some()
    }

    async fn holds_flow(&self) -> bool {
        require_remote_flow(
            &self.resources.common.cache,
            self.tenant_id.as_uuid(),
            self.user_id,
        )
        .await
        .is_ok()
    }
}

/// Whether `seen` holds every one of `steps`.
fn saw_all(seen: &Seen, steps: &[&str]) -> bool {
    let seen = seen.lock().unwrap();
    steps.iter().all(|step| seen.iter().any(|got| got == step))
}

#[tokio::test]
async fn every_sign_in_step_reaches_the_sign_in_service_and_none_the_read_service() {
    let (read_url, read_seen) = stand_in();
    let (login_url, login_seen) = stand_in();
    let athlete = Athlete::new(test_server_config_with_sciotte_planes(
        &read_url, &login_url,
    ))
    .await;

    let (status, signed_in) = athlete.sign_in(ACCEPTED_CODE).await;
    assert_eq!(status, 200, "{signed_in}");
    assert!(athlete.has_coros_session().await, "{signed_in}");

    assert!(
        saw_all(&login_seen, &SIGN_IN_STEPS),
        "the sign-in service ran every step, the drop of its copy included: {:?}",
        login_seen.lock().unwrap()
    );
    let read = read_seen.lock().unwrap().clone();
    assert!(
        SIGN_IN_STEPS
            .iter()
            .all(|step| !read.contains(&(*step).to_owned())),
        "no sign-in step reached the read service: {read:?}"
    );
    let login = login_seen.lock().unwrap().clone();
    assert!(
        login
            .iter()
            .all(|step| SIGN_IN_STEPS.contains(&step.as_str())),
        "no read reached the sign-in service: {login:?}"
    );

    // Reads, and the session drops a disconnect sends, go through the read
    // client every sciotte provider holds.
    athlete
        .resources
        .fitness
        .provider_registry
        .require_sciotte_reads()
        .unwrap()
        .for_session(SESSION)
        .delete_session(SESSION)
        .await
        .unwrap();
    // The sign-in's own reads (the prefetch it starts) may still be landing on
    // the read service, so look for the drop rather than at the last request;
    // none reached it before, as asserted above.
    assert!(
        read_seen
            .lock()
            .unwrap()
            .iter()
            .any(|request| request == "DELETE /auth/sessions/:id"),
        "{:?}",
        read_seen.lock().unwrap()
    );
}

#[tokio::test]
async fn with_no_sign_in_service_a_sign_in_runs_on_the_one_service() {
    let (url, seen) = stand_in();
    let athlete = Athlete::new(test_server_config_with_sciotte(&url)).await;

    let (status, signed_in) = athlete.sign_in(ACCEPTED_CODE).await;
    assert_eq!(status, 200, "{signed_in}");
    assert!(athlete.has_coros_session().await, "{signed_in}");
    assert!(
        saw_all(&seen, &SIGN_IN_STEPS[..3]),
        "{:?}",
        seen.lock().unwrap()
    );
    // With one service the signed-in copy is the one the next reads use.
    assert!(
        !seen
            .lock()
            .unwrap()
            .iter()
            .any(|request| request == "DELETE /auth/sessions/:id"),
        "the one service keeps the session it signed in: {:?}",
        seen.lock().unwrap()
    );
}

/// The sign-in service scales to zero between sign-ins, so the instance that
/// parked the browser can be gone when the code arrives. That is a sign-in to
/// start again — the same refusal a lapsed flow gets — never a system failure.
#[tokio::test]
async fn a_code_the_sign_in_service_no_longer_holds_a_browser_for_is_a_lapsed_sign_in() {
    let (read_url, _read_seen) = stand_in();
    let (login_url, _login_seen) = stand_in();
    let athlete = Athlete::new(test_server_config_with_sciotte_planes(
        &read_url, &login_url,
    ))
    .await;

    let (status, lapsed) = athlete.sign_in(LOST_CODE).await;
    assert_eq!(status, 400, "{lapsed}");
    assert_eq!(
        lapsed["details"]["reason"], LOGIN_FLOW_EXPIRED_REASON,
        "{lapsed}"
    );
    assert!(
        !athlete.holds_flow().await,
        "the flow the service no longer holds is forgotten"
    );
    assert!(
        !athlete.has_coros_session().await,
        "a lapsed sign-in stores no session: {lapsed}"
    );
}

#[tokio::test]
async fn a_second_factor_choice_reaches_the_sign_in_service() {
    let (read_url, read_seen) = stand_in();
    let (login_url, login_seen) = stand_in();
    let athlete = Athlete::new(test_server_config_with_sciotte_planes(
        &read_url, &login_url,
    ))
    .await;

    let (status, signed_in) = athlete.sign_in_by_second_factor().await;
    assert_eq!(status, 200, "{signed_in}");
    assert!(athlete.has_coros_session().await, "{signed_in}");
    assert!(
        saw_all(&login_seen, &["POST /auth/select-2fa"]),
        "the choice reached the sign-in service: {:?}",
        login_seen.lock().unwrap()
    );
    assert!(
        !read_seen
            .lock()
            .unwrap()
            .iter()
            .any(|request| request == "POST /auth/select-2fa"),
        "the choice never reached the read service: {:?}",
        read_seen.lock().unwrap()
    );
}
