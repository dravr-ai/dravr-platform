// ABOUTME: carnet#728 — a code COROS refuses keeps the same sign-in on its code step, on the age it already had
// ABOUTME: Pins the flow kept on a refused code, its lifetime left alone, and an ended or superseded sign-in dropping it

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A COROS sign-in asks for a six-digit code. When the athlete typed a wrong
//! one, the scraper service used to end the login, and the platform forgot the
//! flow: the athlete was sent back to the credentials to start again. The
//! service now keeps its browser parked on the code step and answers
//! `otp_required` with `rejected`, and the platform keeps the flow it holds —
//! without writing it again, so the flow still lapses with the browser it
//! names.
//!
//! The scraper here is a loopback stand-in (a test double, per the repo's mock
//! rule) that refuses every code but one, refuses an incomplete code the same
//! way, and records every code it was sent, by flow. It runs on a thread of
//! its own, because each `#[tokio::test]` drops its runtime when it ends and a
//! stand-in spawned on one would die with the first test to finish.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

use std::env;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::thread;
use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use common::{create_test_server_resources, create_test_user, generate_test_token};
use dravr_sciotte::client::{ENV_AUDIENCE, ENV_REMOTE_URL};
use helpers::axum_test::AxumTestRequest;
use pierre_core::constants::oauth::providers::provider_terms_version;
use pierre_core::constants::oauth_providers::{SCIOTTE_COROS, TOKEN_TYPE_SESSION};
use pierre_core::models::TenantId;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_middleware::provider_link_token::{mint_link_token, MintProviderLinkTokenArgs};
use pierre_routes_auth::{
    remember_remote_flow_for, require_remote_flow, AuthRoutes, CODE_REJECTED_REASON,
    LOGIN_FLOW_EXPIRED_REASON,
};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::runtime::Builder;
use tokio::time::sleep;
use uuid::Uuid;

/// The one code the stand-in accepts.
const ACCEPTED_CODE: &str = "246810";

/// A code the stand-in answers with a lockout: the provider ended the sign-in.
const LOCKOUT_CODE: &str = "999999";

/// An email the stand-in refuses the password of.
const WRONG_PASSWORD_EMAIL: &str = "wrong-password@example.com";

/// The session the accepted code signs in to.
const SESSION: &str = "coros-728";

/// The service's own reason for a code its code step did not take as
/// complete. The platform reads only `rejected`, never this prose.
const CODE_NOT_SUBMITTED: &str =
    "The code step did not take the code as complete, so it was not submitted";

/// What the stand-in saw: every code, by the flow it named.
#[derive(Default)]
struct Seen {
    codes: Mutex<Vec<(String, String)>>,
    flows: AtomicUsize,
}

static SCRAPER: OnceLock<(String, Arc<Seen>)> = OnceLock::new();

/// The stand-in's address and what it saw, started once for the binary on a
/// thread with its own runtime, which also points the platform at it.
fn scraper() -> &'static (String, Arc<Seen>) {
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
        (url, seen)
    })
}

fn session_json() -> Value {
    json!({
        "session_id": SESSION,
        "cookies": [],
        "created_at": "2026-10-02T17:07:08Z",
        "expires_at": null
    })
}

async fn login(State(seen): State<Arc<Seen>>, Json(body): Json<Value>) -> Response {
    if body["email"]
        .as_str()
        .is_some_and(|email| email.starts_with("wrong-password"))
    {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "status": "failed", "reason": "Incorrect account or password" })),
        )
            .into_response();
    }
    let flow = seen.flows.fetch_add(1, Ordering::SeqCst);
    Json(json!({
        "status": "otp_required",
        "reason": "COROS sent a verification code",
        "flow_id": format!("flow-{flow}"),
        "provider": "coros"
    }))
    .into_response()
}

async fn submit_otp(State(seen): State<Arc<Seen>>, Json(body): Json<Value>) -> Response {
    let flow_id = body["flow_id"].as_str().unwrap_or_default().to_owned();
    let code = body["code"].as_str().unwrap_or_default().to_owned();
    if !flow_id.starts_with("flow-") {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "no_pending_login" })),
        )
            .into_response();
    }
    seen.codes
        .lock()
        .unwrap()
        .push((flow_id.clone(), code.clone()));
    let answer = match code.as_str() {
        ACCEPTED_CODE => json!({
            "status": "authenticated",
            "session_id": SESSION,
            "provider": "coros"
        }),
        LOCKOUT_CODE => json!({ "status": "failed", "reason": "Too many attempts" }),
        short if short.len() < 6 => json!({
            "status": "otp_required",
            "reason": CODE_NOT_SUBMITTED,
            "flow_id": flow_id,
            "provider": "coros",
            "rejected": "code_rejected"
        }),
        _ => json!({
            "status": "otp_required",
            "reason": "Incorrect code",
            "flow_id": flow_id,
            "provider": "coros",
            "rejected": "code_rejected"
        }),
    };
    Json(answer).into_response()
}

/// The scraper service stand-in: a COROS login that asks for a code, a code
/// step that takes one code, and the session the platform then exports and
/// reads.
fn stand_in(seen: Arc<Seen>) -> Router {
    Router::new()
        .route("/auth/login-with-credentials", post(login))
        .route("/auth/submit-otp", post(submit_otp))
        .route(
            "/auth/sessions/{session_id}/export",
            get(|Path(_session_id): Path<String>| async {
                Json(json!({ "session": session_json() }))
            }),
        )
        .route(
            "/auth/import-session",
            post(|| async { Json(json!({ "session_id": SESSION })) }),
        )
        .route(
            "/api/activities",
            get(|| async { Json(json!({ "count": 0, "activities": [], "head_complete": true })) }),
        )
        .route(
            "/api/daily-summary",
            get(|| async { Json(json!({ "provider": "coros" })) }),
        )
        .with_state(seen)
}

/// The codes the stand-in was sent naming `flow_id`, in order.
fn codes_for(flow_id: &str) -> Vec<String> {
    scraper()
        .1
        .codes
        .lock()
        .unwrap()
        .iter()
        .filter(|(flow, _)| flow == flow_id)
        .map(|(_, code)| code.clone())
        .collect()
}

/// One athlete, with COROS's notice already accepted.
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

    /// The same athlete, signing in through a hosted COROS link instead.
    fn with_link_token(mut self) -> Self {
        let token = mint_link_token(
            &MintProviderLinkTokenArgs {
                user_id: self.user_id,
                tenant_id: self.tenant_id.as_uuid(),
                provider: "sciotte",
                target: "coros",
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
                "target": "coros",
                "tos_consent": true,
            }),
        )
        .await
    }

    async fn submit(&self, code: &str) -> (u16, Value) {
        self.post("/api/providers/sciotte/submit-otp", json!({ "code": code }))
            .await
    }

    /// The flow the platform holds for the athlete, if any.
    async fn held_flow(&self) -> Result<(String, String), Value> {
        require_remote_flow(
            &self.resources.common.cache,
            self.tenant_id.as_uuid(),
            self.user_id,
        )
        .await
        .map_err(|error| error.details.map_or(Value::Null, |details| *details))
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
}

fn code_rejected() -> Value {
    json!({ "status": "otp_required", "reason": CODE_REJECTED_REASON })
}

/// Login, a wrong code, an incomplete one, then the right one: one flow
/// throughout, and the athlete ends connected.
async fn refused_then_accepted(athlete: &Athlete) {
    let (status, login) = athlete.login("athlete@example.com").await;
    assert_eq!(status, 200, "{login}");
    assert_eq!(login, json!({ "status": "otp_required" }));
    let (flow_id, provider) = athlete.held_flow().await.expect("the login holds a flow");
    assert_eq!(provider, SCIOTTE_COROS);
    assert!(!athlete.has_coros_session().await);

    for code in ["111111", "12"] {
        let (status, refused) = athlete.submit(code).await;
        assert_eq!(status, 200, "{refused}");
        assert_eq!(refused, code_rejected(), "code {code}");
        assert_eq!(
            athlete.held_flow().await,
            Ok((flow_id.clone(), SCIOTTE_COROS.to_owned())),
            "a refused code keeps the same flow"
        );
        assert!(!athlete.has_coros_session().await);
    }

    assert_the_right_code_connects(athlete).await;
    assert_eq!(codes_for(&flow_id), ["111111", "12", ACCEPTED_CODE]);
}

/// The accepted code stores the COROS session and ends the flow.
async fn assert_the_right_code_connects(athlete: &Athlete) {
    let (status, connected) = athlete.submit(ACCEPTED_CODE).await;
    assert_eq!(status, 200, "{connected}");
    assert_eq!(connected["status"], "connected", "{connected}");
    assert_eq!(connected["provider"], SCIOTTE_COROS, "{connected}");
    let token = athlete
        .resources
        .common
        .repos
        .oauth_tokens
        .get_token(athlete.user_id, athlete.tenant_id, SCIOTTE_COROS)
        .await
        .unwrap()
        .expect("the right code stores the session");
    assert_eq!(token.provider, SCIOTTE_COROS);
    assert_eq!(token.token_type, TOKEN_TYPE_SESSION);

    assert_eq!(
        athlete
            .held_flow()
            .await
            .expect_err("a connect forgets the flow")["reason"],
        LOGIN_FLOW_EXPIRED_REASON
    );
}

#[tokio::test]
async fn a_refused_code_keeps_the_flow_and_the_right_code_connects() {
    refused_then_accepted(&Athlete::new().await).await;
}

#[tokio::test]
async fn a_hosted_link_token_keeps_the_flow_on_a_refused_code() {
    refused_then_accepted(&Athlete::new().await.with_link_token()).await;
}

#[tokio::test]
async fn a_refused_code_does_not_extend_the_flows_lifetime() {
    let athlete = Athlete::new().await;
    let (_, login) = athlete.login("athlete@example.com").await;
    assert_eq!(login, json!({ "status": "otp_required" }));
    let (flow_id, _) = athlete.held_flow().await.unwrap();

    // The flow has two seconds left of its life when the wrong code arrives.
    remember_remote_flow_for(
        &athlete.resources.common.cache,
        athlete.tenant_id.as_uuid(),
        athlete.user_id,
        flow_id.clone(),
        SCIOTTE_COROS,
        Duration::from_secs(2),
    )
    .await;
    let (_, refused) = athlete.submit("111111").await;
    assert_eq!(refused, code_rejected());

    sleep(Duration::from_secs(3)).await;

    let (status, expired) = athlete.submit(ACCEPTED_CODE).await;
    assert_eq!(
        status, 400,
        "a refused code must not restart the flow's lifetime: {expired}"
    );
    assert_eq!(expired["details"]["reason"], LOGIN_FLOW_EXPIRED_REASON);
    assert_eq!(codes_for(&flow_id), ["111111"]);
    assert!(!athlete.has_coros_session().await);
}

#[tokio::test]
async fn a_failed_code_step_forgets_the_flow() {
    let athlete = Athlete::new().await;
    let (_, login) = athlete.login("athlete@example.com").await;
    assert_eq!(login, json!({ "status": "otp_required" }));
    let (flow_id, _) = athlete.held_flow().await.unwrap();

    let (status, failed) = athlete.submit(LOCKOUT_CODE).await;
    assert_eq!(status, 200, "{failed}");
    assert_eq!(failed["status"], "failed", "{failed}");
    assert!(
        athlete.held_flow().await.is_err(),
        "a sign-in the provider ended holds no flow"
    );

    let (status, expired) = athlete.submit(ACCEPTED_CODE).await;
    assert_eq!(status, 400, "{expired}");
    assert_eq!(expired["details"]["reason"], LOGIN_FLOW_EXPIRED_REASON);
    assert_eq!(codes_for(&flow_id), [LOCKOUT_CODE]);
    assert!(!athlete.has_coros_session().await);
}

/// A login drops the flow it finds parked before it reaches the service, so a
/// refused password leaves the athlete holding nothing. That forget runs ahead
/// of the `Failed` answer, which therefore cannot be told apart here; the
/// `Failed` arm's own forget is pinned by a sign-in the provider ended at its
/// code step, above.
#[tokio::test]
async fn a_new_login_supersedes_the_parked_flow_and_a_refused_password_holds_none() {
    let athlete = Athlete::new().await;
    let (_, login) = athlete.login("athlete@example.com").await;
    assert_eq!(login, json!({ "status": "otp_required" }));
    let (flow_id, _) = athlete.held_flow().await.unwrap();

    let (status, failed) = athlete.login(WRONG_PASSWORD_EMAIL).await;
    assert_eq!(status, 200, "{failed}");
    assert_eq!(failed["status"], "failed", "{failed}");
    assert!(athlete.held_flow().await.is_err());

    let (status, expired) = athlete.submit(ACCEPTED_CODE).await;
    assert_eq!(status, 400, "{expired}");
    assert_eq!(expired["details"]["reason"], LOGIN_FLOW_EXPIRED_REASON);
    assert!(codes_for(&flow_id).is_empty());
}
