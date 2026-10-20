// ABOUTME: The REST /config and /config/user routes are gone; the configuration tools are the persisted path
// ABOUTME: An authenticated GET or PUT on any of them reaches no handler in the server's HTTP app
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! `GET`/`PUT /config` and `/config/user` built a throwaway runtime
//! configuration per request: a `PUT` answered with the overrides it had
//! "applied" and the next `GET` showed none of them, because nothing was
//! stored. No client on any surface called them; an athlete's configuration
//! is read and written through `get_user_configuration` /
//! `update_user_configuration`, which persist it
//! (`user_configuration_profile_test.rs` pins that round trip). The routes are
//! deleted, so the server's app answers each of them as an unknown path.

use crate::common;

use axum::body::Body;
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE};
use axum::http::{Method, Request, StatusCode};
use common::{create_test_server_resources, create_test_user};
use pierre_mcp_server::mcp::multitenant::ProviderToolRouter;
use serde_json::json;
use tower::ServiceExt;

#[tokio::test]
async fn the_rest_configuration_routes_reach_no_handler() {
    let resources = create_test_server_resources().await.unwrap();
    let (_, user) = create_test_user(&resources.agent.database).await.unwrap();
    let token = resources
        .auth
        .auth_manager
        .generate_token(&user, &resources.auth.jwks_manager)
        .unwrap();
    let overrides = json!({"parameters": {"heart_rate.anaerobic_threshold": 88.0}}).to_string();

    for path in ["/config", "/config/user"] {
        for method in [Method::GET, Method::PUT] {
            let body = if method == Method::PUT {
                Body::from(overrides.clone())
            } else {
                Body::empty()
            };
            let response = ProviderToolRouter::build_http_app(&resources)
                .oneshot(
                    Request::builder()
                        .method(method.clone())
                        .uri(path)
                        .header(AUTHORIZATION, format!("Bearer {token}"))
                        .header(CONTENT_TYPE, "application/json")
                        .body(body)
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                StatusCode::NOT_FOUND,
                "{method} {path} must reach no handler"
            );
        }
    }
}
