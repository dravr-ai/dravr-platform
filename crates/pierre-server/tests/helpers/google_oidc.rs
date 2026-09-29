// ABOUTME: A local stand-in for Google's OpenID Connect token endpoint and signing keys, for the hosted Google sign-in
// ABOUTME: Records every code exchange, answers with the ID token a test minted, and spends each code once as Google does
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
#![allow(dead_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! The authorization server talks to Google twice per sign-in: it exchanges
//! the code at the token endpoint and fetches the keys the ID token is signed
//! with. [`GoogleStub`] serves both from one local listener. The account
//! chooser is never contacted — a test reads the `state` and `nonce` from the
//! redirect to it and calls the callback itself.

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::extract::{Form, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use pierre_auth::admin::jwks::{JsonWebKey, JsonWebKeySet};
use pierre_auth::config::GoogleSignInConfig;
use serde::Serialize;
use serde_json::json;
use tokio::net::TcpListener;

use super::google_token::now_secs;

/// Where the start sends the athlete in tests; never contacted.
pub const GOOGLE_CHOOSER: &str = "https://accounts.google.test/o/oauth2/v2/auth";
/// The Google web client id the tests configure.
pub const GOOGLE_CLIENT_ID: &str = "dravr-test-web-client.apps.googleusercontent.com";
/// Its client secret.
pub const GOOGLE_CLIENT_SECRET: &str = "GOCSPX-test-google-client-secret";

/// The claims of a Google ID token, every one optional so a test can leave
/// any of them out.
#[derive(Debug, Clone, Default, Serialize)]
pub struct GoogleIdClaims {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iss: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aud: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub azp: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sub: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email_verified: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nonce: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exp: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iat: Option<u64>,
}

impl GoogleIdClaims {
    /// What Google issues our client for `email` (verified) under account
    /// `sub`, for the sign-in that minted `nonce`, valid for an hour.
    #[must_use]
    pub fn issued(sub: &str, email: &str, nonce: &str) -> Self {
        let now = now_secs();
        Self {
            iss: Some("https://accounts.google.com".to_owned()),
            aud: Some(GOOGLE_CLIENT_ID.to_owned()),
            azp: Some(GOOGLE_CLIENT_ID.to_owned()),
            sub: Some(sub.to_owned()),
            email: Some(email.to_owned()),
            email_verified: Some(true),
            name: Some("Google Athlete".to_owned()),
            nonce: Some(nonce.to_owned()),
            exp: Some(now + 3600),
            iat: Some(now),
        }
    }
}

/// What the token endpoint answers next.
#[derive(Clone)]
enum Answer {
    /// 200 with this ID token
    IdToken(String),
    /// This status and raw body
    Error(u16, String),
}

#[derive(Default)]
struct StubState {
    requests: Vec<HashMap<String, String>>,
    spent_codes: HashSet<String>,
    answer: Option<Answer>,
    keys: Vec<JsonWebKey>,
}

/// Google's token endpoint and key set, on a local listener.
pub struct GoogleStub {
    /// `http://127.0.0.1:<port>`
    pub base: String,
    state: Arc<Mutex<StubState>>,
}

impl GoogleStub {
    /// Serve the token endpoint and `keys` as Google's signing keys.
    pub async fn serve(keys: Vec<JsonWebKey>) -> Self {
        let state = Arc::new(Mutex::new(StubState {
            keys,
            ..StubState::default()
        }));
        let app = Router::new()
            .route("/token", post(token))
            .route("/certs", get(certs))
            .with_state(Arc::clone(&state));
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr: SocketAddr = listener.local_addr().expect("local addr");
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve");
        });
        Self {
            base: format!("http://{addr}"),
            state,
        }
    }

    /// The Google client the authorization server is configured with,
    /// pointed at this stub.
    #[must_use]
    pub fn config(&self) -> GoogleSignInConfig {
        GoogleSignInConfig {
            authorization_endpoint: GOOGLE_CHOOSER.to_owned(),
            token_endpoint: format!("{}/token", self.base),
            jwks_url: format!("{}/certs", self.base),
            ..GoogleSignInConfig::new(GOOGLE_CLIENT_ID, GOOGLE_CLIENT_SECRET)
        }
    }

    /// Answer the next code exchanges with `id_token`.
    pub fn set_id_token(&self, id_token: &str) {
        self.state.lock().unwrap().answer = Some(Answer::IdToken(id_token.to_owned()));
    }

    /// Answer the next code exchanges with `status` and `body`.
    pub fn set_error(&self, status: u16, body: &str) {
        self.state.lock().unwrap().answer = Some(Answer::Error(status, body.to_owned()));
    }

    /// Every form the token endpoint received, in order.
    #[must_use]
    pub fn token_requests(&self) -> Vec<HashMap<String, String>> {
        self.state.lock().unwrap().requests.clone()
    }
}

async fn token(
    State(state): State<Arc<Mutex<StubState>>>,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let mut state = state.lock().unwrap();
    state.requests.push(form.clone());
    let code = form.get("code").cloned().unwrap_or_default();
    // Google spends a code on its first exchange.
    if !state.spent_codes.insert(code) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "invalid_grant", "error_description": "Bad Request"})),
        )
            .into_response();
    }
    match state.answer.clone() {
        Some(Answer::IdToken(id_token)) => Json(json!({
            "access_token": "ya29.test-access-token",
            "expires_in": 3599,
            "id_token": id_token,
            "scope": "openid https://www.googleapis.com/auth/userinfo.email",
            "token_type": "Bearer"
        }))
        .into_response(),
        Some(Answer::Error(status, body)) => (
            StatusCode::from_u16(status).unwrap(),
            [("content-type", "application/json")],
            body,
        )
            .into_response(),
        None => (StatusCode::INTERNAL_SERVER_ERROR, "no answer configured").into_response(),
    }
}

async fn certs(State(state): State<Arc<Mutex<StubState>>>) -> Json<JsonWebKeySet> {
    Json(JsonWebKeySet {
        keys: state.lock().unwrap().keys.clone(),
    })
}
