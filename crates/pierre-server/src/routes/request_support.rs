// ABOUTME: Serves a REST request made under an admin's web impersonation inside its support session, before any route runs
// ABOUTME: So every read the request records on a member's record names the admin, as a chat turn, MCP or A2A call does (carnet#1511)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The REST half of the support session (carnet#1511).
//!
//! An admin acting as a user on the web holds an impersonation token, and
//! every REST route serves them as that user. A chat turn, an MCP call and an
//! A2A call made in a support session run inside
//! [`SUPPORT_SESSION`], which is what marks a coach read the session makes as
//! support's on the member's record (`read_subject::record_coach_read`) and
//! what refuses what a support session may not leave behind. This layer gives
//! every REST route the same: a request whose credential is a live
//! impersonation token is served inside its session, so a page that reads a
//! member — Coach Home's roster, the athlete's anchors, the thread's plan
//! card, the plan view, a group view — never records the coach as
//! the reader of a read the coach did not make.
//!
//! Authentication runs inside each handler's extractor, after every layer has
//! entered, so this layer reads the session from the same credential
//! ([`extractors::request_credential`]) through the admission's own derivation
//! ([`McpAuthMiddleware::support_session_of`]), which writes nothing: the
//! handler's authentication stays the request's one admission. A credential
//! that is no impersonation, or one the handler will refuse, declares no
//! session. A lookup that fails refuses the request rather than serve it as
//! the user alone when a session may be live.
//!
//! `/mcp` and A2A serve the session their own credential binds inside; work a
//! handler spawns does not inherit it.

use std::sync::Arc;

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use pierre_middleware::extractors;
use pierre_middleware::McpAuthMiddleware;
use pierre_tool_runtime::context::SUPPORT_SESSION;

/// Serve the rest of the request inside the support session its credential
/// acts in, if any.
pub async fn declare_support_session(
    State(auth): State<Arc<McpAuthMiddleware>>,
    request: Request,
    next: Next,
) -> Response {
    let Some(credential) = extractors::request_credential(request.headers()) else {
        return next.run(request).await;
    };
    match auth.support_session_of(&credential).await {
        Ok(None) => next.run(request).await,
        Ok(support) => SUPPORT_SESSION.scope(support, next.run(request)).await,
        Err(e) => e.into_response(),
    }
}
