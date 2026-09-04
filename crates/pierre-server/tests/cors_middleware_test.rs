// ABOUTME: Pins the CORS layer's Access-Control-Expose-Headers contract for the refusal challenge
// ABOUTME: Asserts a browser can read WWW-Authenticate cross-origin, by name and never by wildcard
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Integration tests for the CORS layer the HTTP server mounts.
//!
//! A refused request recovers by its RFC 6750 challenge, not by its status: a
//! 403 carrying `error="insufficient_scope"` recovers like a 401, while every
//! other 403 is a standing authorization decision that must leave the session
//! signed in. The challenge rides in `WWW-Authenticate`, which is not a
//! CORS-safelisted response header — so a browser hands it to JS only when
//! `Access-Control-Expose-Headers` names it. These tests drive a real response
//! through [`setup_cors`] and assert that header's value.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use axum::{
    body::Body,
    http::{
        header::{
            ACCESS_CONTROL_ALLOW_CREDENTIALS, ACCESS_CONTROL_EXPOSE_HEADERS, ORIGIN,
            WWW_AUTHENTICATE,
        },
        HeaderValue, Request as HttpRequest, StatusCode,
    },
    response::Response,
    routing::get,
    Router,
};
use pierre_middleware::setup_cors;
use std::error::Error;
use tower::ServiceExt;

/// A production origin, used as both the configured allowlist entry and the
/// request's `Origin`. Naming an origin puts `setup_cors` on its credentialed
/// branch, which is the branch a browser serves the web SPA from.
const BROWSER_ORIGIN: &str = "https://app.dravr.ai";

/// The challenge a scope refusal answers with — the one shape that recovers by
/// re-authenticating rather than by signing the athlete out.
const INSUFFICIENT_SCOPE_CHALLENGE: &str =
    r#"Bearer error="insufficient_scope", scope="coach:write""#;

/// Answer the way an authorization refusal does: a 403 whose challenge header
/// carries the reason the credential fell short.
async fn refuse_with_challenge() -> Response {
    let mut response = Response::new(Body::empty());
    *response.status_mut() = StatusCode::FORBIDDEN;
    response.headers_mut().insert(
        WWW_AUTHENTICATE,
        HeaderValue::from_static(INSUFFICIENT_SCOPE_CHALLENGE),
    );
    response
}

/// Drive a cross-origin GET through the real CORS layer and return the response.
async fn refused_response(allowed_origins: &str) -> Result<Response, Box<dyn Error>> {
    let app = Router::new()
        .route("/", get(refuse_with_challenge))
        .layer(setup_cors(allowed_origins));

    let request = HttpRequest::builder()
        .uri("/")
        .header(ORIGIN, BROWSER_ORIGIN)
        .body(Body::empty())?;

    Ok(app.oneshot(request).await?)
}

/// The challenge must reach browser JS on the credentialed origin list, which
/// is the configuration the deployed SPA runs under once it is served from a
/// different origin than the API.
#[tokio::test]
async fn test_challenge_header_is_exposed_to_a_listed_origin() -> Result<(), Box<dyn Error>> {
    let response = refused_response(BROWSER_ORIGIN).await?;

    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    let challenge = response
        .headers()
        .get(WWW_AUTHENTICATE)
        .expect("refusal carries a WWW-Authenticate challenge")
        .to_str()?;
    assert_eq!(challenge, INSUFFICIENT_SCOPE_CHALLENGE);

    let exposed = response
        .headers()
        .get(ACCESS_CONTROL_EXPOSE_HEADERS)
        .expect("CORS layer exposes response headers to JS")
        .to_str()?;
    assert_eq!(exposed, "www-authenticate");

    // The listed-origin branch turns credentials on, so the exposure has to be
    // by name: browsers discard a wildcard expose list on a credentialed
    // request, and tower-http refuses the combination outright.
    let credentials = response
        .headers()
        .get(ACCESS_CONTROL_ALLOW_CREDENTIALS)
        .expect("listed origins allow credentials")
        .to_str()?;
    assert_eq!(credentials, "true");

    Ok(())
}

/// The wildcard branch (`CORS_ALLOWED_ORIGINS=*`, how dev runs) exposes the
/// same header, so a challenge read in development is a challenge read in
/// production.
#[tokio::test]
async fn test_challenge_header_is_exposed_under_wildcard_origins() -> Result<(), Box<dyn Error>> {
    let response = refused_response("*").await?;

    let exposed = response
        .headers()
        .get(ACCESS_CONTROL_EXPOSE_HEADERS)
        .expect("CORS layer exposes response headers to JS")
        .to_str()?;
    assert_eq!(exposed, "www-authenticate");

    Ok(())
}

/// An empty `CORS_ALLOWED_ORIGINS` falls back to any origin, and that fallback
/// exposes the challenge too — the header is a property of the layer, not of
/// how the origin list happened to parse.
#[tokio::test]
async fn test_challenge_header_is_exposed_when_list_is_empty() -> Result<(), Box<dyn Error>> {
    let response = refused_response("").await?;

    let exposed = response
        .headers()
        .get(ACCESS_CONTROL_EXPOSE_HEADERS)
        .expect("CORS layer exposes response headers to JS")
        .to_str()?;
    assert_eq!(exposed, "www-authenticate");

    Ok(())
}

/// The exposure names the header instead of sending `*`. A wildcard reads as
/// permissive but is ignored for credentialed requests, which is exactly the
/// request shape the web adapter sends.
#[tokio::test]
async fn test_exposure_names_the_header_instead_of_a_wildcard() -> Result<(), Box<dyn Error>> {
    for allowed_origins in [BROWSER_ORIGIN, "*", ""] {
        let response = refused_response(allowed_origins).await?;
        let exposed = response
            .headers()
            .get(ACCESS_CONTROL_EXPOSE_HEADERS)
            .expect("CORS layer exposes response headers to JS")
            .to_str()?;

        assert!(
            !exposed.contains('*'),
            "expose list must name headers, got {exposed:?} for origins {allowed_origins:?}"
        );
        assert!(
            exposed.contains("www-authenticate"),
            "expose list must name the challenge, got {exposed:?} for origins {allowed_origins:?}"
        );
    }

    Ok(())
}
