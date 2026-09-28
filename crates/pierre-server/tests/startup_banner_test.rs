// ABOUTME: Pins every line of the startup endpoint banner against the router build_http_app serves
// ABOUTME: A listed path the router does not match, spells differently, or a method it refuses fails the test
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The startup banner is a hand-written list, so nothing but this test keeps
//! it honest. Each line is sent, with every method it names, through the
//! application the server serves; a layer added on top reads the
//! `MatchedPath` axum records when a route matches and echoes it back. A line
//! passes only when the router matched a route, the route's own pattern is
//! the line's path character for character, and the method is not refused
//! with 405. What the handler answers past that (401, 400) is irrelevant: the
//! banner claims a route exists, not that an anonymous call succeeds. A route
//! whose own layer answers an anonymous call before the method is dispatched
//! (the `/admin/*` token check) shows its 401 rather than a 405, so on those
//! lines only the path and its spelling are proven.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use axum::body::Body;
use axum::extract::{MatchedPath, Request};
use axum::http::{HeaderValue, Method, StatusCode};
use axum::middleware::{from_fn, Next};
use axum::response::Response;
use axum::Router;
use common::create_test_server_resources;
use pierre_mcp_server::mcp::multitenant::ProviderToolRouter;
use pierre_mcp_server::startup_banner::ENDPOINT_CATEGORIES;
use tower::ServiceExt;

/// Response header the probe layer writes the matched route pattern into.
const MATCHED_PATH_HEADER: &str = "x-banner-matched-path";

/// Value substituted for every `{placeholder}` segment of a banner path.
const PLACEHOLDER_VALUE: &str = "00000000-0000-4000-8000-000000000000";

/// Echo the route pattern axum matched, if any, on the response.
async fn echo_matched_path(request: Request, next: Next) -> Response {
    let matched = request
        .extensions()
        .get::<MatchedPath>()
        .map(|path| path.as_str().to_owned());
    let mut response = next.run(request).await;
    if let Some(matched) = matched {
        response.headers_mut().insert(
            MATCHED_PATH_HEADER,
            HeaderValue::from_str(&matched).expect("route patterns are header-safe"),
        );
    }
    response
}

/// The production app with the probe layer on every route and the fallback.
async fn probed_app() -> Router {
    let resources = create_test_server_resources().await.unwrap();
    ProviderToolRouter::build_http_app(&resources).layer(from_fn(echo_matched_path))
}

/// A concrete URI for a router pattern: each `{name}` segment becomes a value.
fn concrete_uri(pattern: &str) -> String {
    pattern
        .split('/')
        .map(|segment| {
            if segment.starts_with('{') && segment.ends_with('}') {
                PLACEHOLDER_VALUE
            } else {
                segment
            }
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// Why the router does not serve `method pattern` as written, or `None` when it does.
async fn drift(app: &Router, method: &str, pattern: &str) -> Option<String> {
    let request = Request::builder()
        .method(Method::from_bytes(method.as_bytes()).unwrap())
        .uri(concrete_uri(pattern))
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let matched = response
        .headers()
        .get(MATCHED_PATH_HEADER)
        .map(|value| value.to_str().unwrap().to_owned());
    match matched {
        None => Some(format!(
            "{method} {pattern}: no route matches (status {})",
            response.status()
        )),
        Some(matched) if matched != pattern => Some(format!(
            "{method} {pattern}: the router spells this route {matched}"
        )),
        Some(_) if response.status() == StatusCode::METHOD_NOT_ALLOWED => Some(format!(
            "{method} {pattern}: the route exists but refuses {method} with 405"
        )),
        Some(_) => None,
    }
}

#[tokio::test]
async fn every_banner_line_names_a_route_the_router_serves_as_spelled() {
    let app = probed_app().await;
    let mut lines = 0_usize;
    let mut drifted = Vec::new();
    for category in ENDPOINT_CATEGORIES {
        for (_, methods, pattern) in category.endpoints {
            for method in methods.split('/') {
                lines += 1;
                if let Some(reason) = drift(&app, method, pattern).await {
                    drifted.push(reason);
                }
            }
        }
    }
    assert!(lines >= 30, "the banner lost most of its lines: {lines}");
    assert!(
        drifted.is_empty(),
        "startup banner lines the router does not serve as written:\n{}",
        drifted.join("\n")
    );
}

/// The probe must see each kind of drift, or the test above passes vacuously.
#[tokio::test]
async fn the_probe_reports_an_unserved_path_a_respelled_one_and_a_refused_method() {
    let app = probed_app().await;

    let unserved = drift(&app, "POST", "/auth/login").await.unwrap();
    assert!(unserved.contains("no route matches"), "{unserved}");

    let respelled = drift(&app, "GET", "/a2a/clients/{id}").await.unwrap();
    assert!(
        respelled.contains("spells this route /a2a/clients/{client_id}"),
        "{respelled}"
    );

    let refused = drift(&app, "DELETE", "/health").await.unwrap();
    assert!(refused.contains("refuses DELETE with 405"), "{refused}");

    assert_eq!(drift(&app, "GET", "/health").await, None);
}
