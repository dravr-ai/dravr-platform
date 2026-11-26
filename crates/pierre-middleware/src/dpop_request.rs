// ABOUTME: How a protocol request presented its credential: scheme, DPoP proofs, method and the path a proof's htu names
// ABOUTME: Captured from HTTP once (tronc does it for POST /mcp), read back by /mcp, /mcp/tools and A2A into the middleware's check

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The `DPoP` context (RFC 9449) of a JSON-RPC request.
//!
//! Tronc's MCP transport, built `with_dpop_scheme`, records on each request
//! the `Authorization` scheme, the HTTP method and the number of `DPoP`
//! headers, and forwards the proof among the headers. The routes that build a
//! [`JsonRpcRequest`] themselves (`GET /mcp/tools`, A2A) record the same
//! through [`PresentedCall`], plus the path they were reached at. The auth
//! hooks then read the request back into the `Authorization` value the
//! middleware takes ([`authorization_value`]) and its [`DpopContext`].

use std::collections::HashMap;
use std::convert::Infallible;
use std::fmt;
use std::future::{ready, Future};

use axum::extract::{FromRequestParts, OriginalUri};
use axum::http::request::Parts;
use axum::http::{header, HeaderMap};
use dravr_tronc::mcp::auth::{
    AUTHORIZATION_SCHEME_METADATA, DPOP_PROOFS_METADATA, HTTP_METHOD_METADATA,
};
use dravr_tronc::mcp::protocol::JsonRpcRequest;
use dravr_tronc::mcp::transport::http::forwarded_headers;
use dravr_tronc::server::auth::{bearer_credential, dpop_credential};
use pierre_auth::config::{self, OAuth2ServerConfig, DIALED_PROTO_HEADER};
use pierre_auth::dpop::{DpopContext, PresentedProofs, BEARER_SCHEME, DPOP_HEADER, DPOP_SCHEME};
use pierre_core::auth_header::is_api_key_format;
use serde_json::Value;

/// `metadata` key naming the path a request was reached at, which a proof's
/// `htu` names; absent on a request the MCP engine built at its own path.
pub const REQUEST_PATH_METADATA: &str = "pierre-request-path";

/// How an HTTP request presented its credential, captured for the
/// [`JsonRpcRequest`] a route builds from it.
///
/// `Debug` is hand-written: the token is a credential and prints as
/// `[REDACTED]`.
#[derive(Clone, Default)]
pub struct PresentedCall {
    token: Option<String>,
    metadata: HashMap<String, String>,
    headers: Option<HashMap<String, Value>>,
}

impl fmt::Debug for PresentedCall {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PresentedCall")
            .field("token", &self.token.as_ref().map(|_| "[REDACTED]"))
            .field("metadata", &self.metadata)
            .field("headers", &self.headers)
            .finish()
    }
}

impl PresentedCall {
    /// The credential `headers` present under `Bearer` or `DPoP`, with the
    /// method and path a `DPoP` proof must name.
    #[must_use]
    pub fn from_http(headers: &HeaderMap, method: &str, path: &str) -> Self {
        let (token, mut metadata) = presented_credential(headers, method);
        metadata.insert(REQUEST_PATH_METADATA.to_owned(), path.to_owned());
        Self {
            token,
            metadata,
            headers: forwarded_headers(headers),
        }
    }

    /// Put the credential and how it was presented on `request`.
    pub fn attach(self, request: &mut JsonRpcRequest) {
        request.auth_token = self.token;
        request.metadata.extend(self.metadata);
        request.headers = self.headers;
    }
}

impl<S: Send + Sync> FromRequestParts<S> for PresentedCall {
    type Rejection = Infallible;

    fn from_request_parts(
        parts: &mut Parts,
        _state: &S,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send {
        // The path the client dialed, before any nesting stripped a prefix.
        let path = parts
            .extensions
            .get::<OriginalUri>()
            .map_or_else(|| parts.uri.path().to_owned(), |uri| uri.path().to_owned());
        ready(Ok(Self::from_http(
            &parts.headers,
            parts.method.as_str(),
            &path,
        )))
    }
}

/// The token the `Authorization` header presents, and the `metadata` that
/// says how: the scheme, the HTTP method and the number of `DPoP` headers.
///
/// A copy of tronc's private `presented_credential`
/// (`mcp/transport/http.rs`, 7.5.0 and 8.1.0) for a server that accepts the
/// `DPoP` scheme, so `GET /mcp/tools` and A2A record exactly what `POST /mcp`
/// does. Tronc branch `feature/export-presented-credential` (off main, the
/// 8.x line) exports it; this copy is deleted for
/// `dravr_tronc::mcp::transport::http::presented_credential` when the
/// platform pins the release that carries it.
fn presented_credential(
    headers: &HeaderMap,
    http_method: &str,
) -> (Option<String>, HashMap<String, String>) {
    let mut metadata = HashMap::from([
        (HTTP_METHOD_METADATA.to_owned(), http_method.to_owned()),
        (
            DPOP_PROOFS_METADATA.to_owned(),
            headers.get_all(DPOP_HEADER).iter().count().to_string(),
        ),
    ]);
    let authorization = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok());
    let presented = authorization
        .and_then(|value| bearer_credential(value).map(|token| (BEARER_SCHEME, token)))
        .or_else(|| {
            authorization.and_then(|value| dpop_credential(value).map(|token| (DPOP_SCHEME, token)))
        });
    let token = presented.map(|(scheme, token)| {
        metadata.insert(AUTHORIZATION_SCHEME_METADATA.to_owned(), scheme.to_owned());
        token.to_owned()
    });
    (token, metadata)
}

/// Whether `headers` use `DPoP` (RFC 9449): a proof header, or a token under
/// the `DPoP` scheme, as [`presented_credential`] reads them. A refusal then
/// also names that scheme (§7.1).
#[must_use]
pub fn used_dpop(headers: &HeaderMap) -> bool {
    let (_, metadata) = presented_credential(headers, "");
    metadata
        .get(DPOP_PROOFS_METADATA)
        .is_some_and(|count| count != "0")
        || metadata
            .get(AUTHORIZATION_SCHEME_METADATA)
            .is_some_and(|scheme| scheme == DPOP_SCHEME)
}

/// The `Authorization` value the auth middleware reads for `token`, under
/// the scheme `request` was presented with.
///
/// The transport strips the scheme; the middleware expects the header form —
/// `Bearer <jwt>`, `DPoP <jwt>`, or a bare API key — so the scheme is put back,
/// never rewritten: a `DPoP` token rebuilt as Bearer would be refused as a
/// bound token presented without its proof.
#[must_use]
pub fn authorization_value(request: &JsonRpcRequest, token: &str) -> String {
    let dpop = request
        .metadata
        .get(AUTHORIZATION_SCHEME_METADATA)
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case(DPOP_SCHEME));
    if dpop {
        format!("{DPOP_SCHEME} {token}")
    } else if is_api_key_format(token) {
        token.to_owned()
    } else {
        format!("{BEARER_SCHEME} {token}")
    }
}

/// A forwarded header's value.
fn forwarded<'r>(request: &'r JsonRpcRequest, name: &str) -> Option<&'r str> {
    request
        .headers
        .as_ref()
        .and_then(|headers| headers.get(name))
        .and_then(Value::as_str)
}

/// The authority the client dialed, read from the forwarded headers by
/// [`config::dialed_host`].
#[must_use]
pub fn dialed_host(request: &JsonRpcRequest) -> Option<&str> {
    config::dialed_host(|name| forwarded(request, name))
}

/// The URI the client dialed, when its origin is one of the server's: the
/// path the request recorded, else `default_path` (where the engine that
/// built it is mounted). What a proof's `htu` must name.
#[must_use]
pub fn dialed_uri(
    request: &JsonRpcRequest,
    config: &OAuth2ServerConfig,
    default_path: &str,
) -> Option<String> {
    let path = request
        .metadata
        .get(REQUEST_PATH_METADATA)
        .map_or(default_path, String::as_str);
    config.dialed_uri(
        forwarded(request, DIALED_PROTO_HEADER),
        dialed_host(request),
        path,
    )
}

/// The request's `DPoP` context: the forwarded proof, how many `DPoP`
/// headers were counted, the HTTP method, and `htu`.
#[must_use]
pub fn dpop_context<'r>(request: &'r JsonRpcRequest, htu: Option<&'r str>) -> DpopContext<'r> {
    let proof = forwarded(request, DPOP_HEADER);
    let count: usize = request
        .metadata
        .get(DPOP_PROOFS_METADATA)
        .and_then(|count| count.parse().ok())
        .unwrap_or_else(|| usize::from(proof.is_some()));
    DpopContext {
        proofs: PresentedProofs { proof, count },
        htm: request
            .metadata
            .get(HTTP_METHOD_METADATA)
            .map_or("POST", String::as_str),
        htu,
    }
}
