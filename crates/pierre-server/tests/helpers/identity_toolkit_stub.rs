// ABOUTME: A local stand-in for Identity Toolkit and the metadata token endpoint, recording each accounts:delete
// ABOUTME: What the account-deletion suites read back to prove the Firebase identity was deleted on the wire
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
#![allow(dead_code)]

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use pierre_auth::config::oauth::FirebaseConfig;
use serde_json::{json, Value};
use tokio::net::TcpListener;

/// The Firebase project the stub serves.
pub const PROJECT: &str = "dravr-test-firebase";

/// The access token the stub's metadata endpoint mints.
pub const ACCESS_TOKEN: &str = "ya29.firebase-admin";

/// How the stub answers every `accounts:delete`.
#[derive(Debug, Clone, Copy)]
pub enum Answer {
    /// 200: the user was deleted.
    Deleted,
    /// 400 `USER_NOT_FOUND`: there was no such user.
    UserNotFound,
    /// 503: Identity Toolkit is down.
    Unavailable,
}

/// One `accounts:delete` as the stand-in received it.
#[derive(Debug, Clone)]
pub struct DeleteCall {
    /// The project in the request path.
    pub project: String,
    /// The `Authorization` header the call carried.
    pub authorization: Option<String>,
    /// The `localId` in the body: the Firebase uid.
    pub local_id: String,
}

/// The stand-in: answers every delete with `answer` and keeps the calls.
pub struct IdentityToolkitStub {
    answer: Mutex<Answer>,
    calls: Mutex<Vec<DeleteCall>>,
}

impl IdentityToolkitStub {
    /// A stand-in answering every delete with `answer`.
    #[must_use]
    pub fn answering(answer: Answer) -> Arc<Self> {
        Arc::new(Self {
            answer: Mutex::new(answer),
            calls: Mutex::new(Vec::new()),
        })
    }

    /// Answer every later delete with `answer`: Identity Toolkit coming back
    /// from an outage, or going down.
    pub fn set_answer(&self, answer: Answer) {
        *self.answer.lock().expect("mutex") = answer;
    }

    /// Every `accounts:delete` so far, in order.
    #[must_use]
    pub fn calls(&self) -> Vec<DeleteCall> {
        self.calls.lock().expect("mutex").clone()
    }

    /// Serve the stand-in from a local listener and return the Firebase
    /// config that points the server at it.
    pub async fn serve(self: &Arc<Self>) -> FirebaseConfig {
        async fn token() -> impl IntoResponse {
            Json(
                json!({ "access_token": ACCESS_TOKEN, "expires_in": 3600, "token_type": "Bearer" }),
            )
        }
        async fn delete(
            State(stub): State<Arc<IdentityToolkitStub>>,
            Path(project): Path<String>,
            headers: HeaderMap,
            Json(body): Json<Value>,
        ) -> impl IntoResponse {
            stub.calls.lock().expect("mutex").push(DeleteCall {
                project,
                authorization: headers
                    .get("authorization")
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_owned),
                local_id: body["localId"].as_str().unwrap_or_default().to_owned(),
            });
            let answer = *stub.answer.lock().expect("mutex");
            match answer {
                Answer::Deleted => (
                    StatusCode::OK,
                    Json(json!({ "kind": "identitytoolkit#DeleteAccountResponse" })),
                ),
                Answer::UserNotFound => (
                    StatusCode::BAD_REQUEST,
                    Json(json!({ "error": { "code": 400, "message": "USER_NOT_FOUND" } })),
                ),
                Answer::Unavailable => (
                    StatusCode::SERVICE_UNAVAILABLE,
                    Json(json!({ "error": { "code": 503, "message": "UNAVAILABLE" } })),
                ),
            }
        }
        let app = Router::new()
            .route("/token", get(token))
            .route("/v1/projects/{project}/accounts:delete", post(delete))
            .with_state(Arc::clone(self));
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr: SocketAddr = listener.local_addr().expect("local addr");
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve");
        });
        FirebaseConfig {
            project_id: Some(PROJECT.to_owned()),
            api_key: None,
            enabled: true,
            identity_toolkit_url: format!("http://{addr}"),
            access_token_url: format!("http://{addr}/token"),
        }
    }
}
