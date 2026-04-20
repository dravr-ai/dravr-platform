// ABOUTME: SSE route exposing AG-UI event streams to clients subscribed by run_id
// ABOUTME: Clients connect to /api/agui/runs/{run_id}/stream; server verifies owner before streaming
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! AG-UI Server-Sent Events route.
//!
//! Exposes `GET /api/agui/runs/{run_id}/stream` — subscribers receive a
//! live stream of [`super::events::AgUiEvent`]s (one JSON payload per
//! SSE data frame) for the requested run. The route terminates when the
//! pipeline unregisters the run or the client disconnects.
//!
//! # Authorization
//!
//! Subscription is gated by two checks:
//! 1. The request MUST carry a Bearer JWT; the `auth_middleware`
//!    resolves it to `(user_id, tenant_id)`. Missing / invalid /
//!    expired tokens are rejected 401.
//! 2. The authenticated caller's identity MUST match the
//!    [`super::registry::RunOwner`] recorded at `register()` time.
//!    Unknown runs return 404; known runs owned by somebody else
//!    return 403.
//!
//! Without both checks, a caller that learns another user's `run_id`
//! (URL capture, log leak, guessing) could read their AG-UI stream —
//! which carries `TEXT_MESSAGE_CONTENT`, `TOOL_CALL_ARGS`, and tool
//! results and is therefore PII.
//!
//! ## No super-admin bypass
//!
//! Admin / support staff cannot subscribe to another user's AG-UI
//! run by virtue of holding a super-admin token; the owner check is
//! strict equality on `(user_id, tenant_id)`. This is deliberate:
//! AG-UI streams carry the model's reasoning trace and tool inputs,
//! which often contain health PII the support persona is not
//! authorized to read directly. The supported path for ops triage
//! is to use the existing impersonation flow
//! ([`crate::routes::impersonation`]) — that produces a JWT scoped
//! to the target user with full audit logging, and the resulting
//! token then satisfies the owner check normally.
//!
//! ## Feature gating
//!
//! This module only compiles when both `agui` (the protocol types)
//! and `transport-sse` (the HTTP delivery) are present. The route
//! mount in [`crate::mcp::multitenant`] is gated identically. A
//! pierre-server build without `transport-sse` still exposes the
//! [`super::registry::RunRegistry`] for future non-SSE transports
//! (e.g. long-poll, websocket bridge) but no HTTP entry point for
//! AG-UI.

use super::registry::{AuthorizedSubscribe, RunOwner, RunRegistry, RunSubscription};
use crate::errors::{AppError, ErrorCode};
use crate::mcp::resources::ServerResources;
use crate::models::TenantId;
use crate::utils::auth::extract_bearer_token_owned as extract_token;
use axum::{
    extract::{Path, State},
    http::HeaderMap,
    response::sse::{Event, KeepAlive, Sse},
    routing::get,
    Router,
};
use futures_util::stream::Stream;
use std::{convert::Infallible, sync::Arc, time::Duration};
use tokio::sync::broadcast;
use tokio::task;
use tracing::{info, warn};

/// AG-UI SSE routes mounted at `/api/agui/*`.
pub struct AgUiRoutes;

impl AgUiRoutes {
    /// Build the AG-UI router.
    ///
    /// The returned router expects the [`RunRegistry`] plus
    /// [`ServerResources`] in its state so the chat pipeline and the
    /// SSE subscribers share the same broadcast channels, and the
    /// handler can authenticate Bearer tokens via
    /// `resources.auth_middleware`.
    pub fn routes(registry: RunRegistry, resources: Arc<ServerResources>) -> Router {
        Router::new()
            .route(
                "/api/agui/runs/{run_id}/stream",
                get(Self::handle_run_stream),
            )
            .with_state((registry, resources))
    }

    /// Handle an incoming SSE subscription for `run_id`.
    ///
    /// REQUIRES: Bearer JWT authentication. The caller's `user_id`
    /// and `tenant_id` MUST match the owner the run was registered
    /// with.
    ///
    /// # Errors
    ///
    /// - 401 `AuthInvalid` — missing / malformed Authorization header,
    ///   or `auth_middleware` rejects the token.
    /// - 404 `ResourceNotFound` — the `run_id` is not registered (the
    ///   run has not started or has already finished).
    /// - 403 `PermissionDenied` — the token resolves to a user that
    ///   does not own the run.
    async fn handle_run_stream(
        Path(run_id): Path<String>,
        headers: HeaderMap,
        State((registry, resources)): State<(RunRegistry, Arc<ServerResources>)>,
    ) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, AppError> {
        let caller = authenticate_caller(&headers, &resources).await?;
        let subscription = authorize_and_subscribe(&registry, &run_id, caller)?;

        // Yield cooperatively before handing the stream back so a burst
        // of subscribes cannot monopolize the executor. This also
        // satisfies axum's async handler trait bound so we can keep the
        // function as `async fn` for symmetry with the other SSE routes.
        task::yield_now().await;

        info!(
            run_id = %run_id,
            user_id = %caller.user_id,
            backlog_len = subscription.backlog.len(),
            "AG-UI SSE client subscribed"
        );
        let run_id_log = run_id;
        let RunSubscription {
            backlog,
            mut receiver,
        } = subscription;

        let stream = async_stream::stream! {
            let mut event_id: u64 = 0;

            event_id = event_id.wrapping_add(1);
            yield Ok::<_, Infallible>(
                Event::default()
                    .id(event_id.to_string())
                    .event("connection")
                    .data("connected"),
            );

            // Flush the replay buffer first so clients that subscribe
            // after emission started still receive the earlier events
            // in order. The backlog is a snapshot captured at subscribe
            // time — events produced after the snapshot arrive via the
            // live `receiver` below and are guaranteed to follow the
            // backlog because `publish` appends to the buffer before it
            // broadcasts.
            for payload in backlog {
                event_id = event_id.wrapping_add(1);
                yield Ok(
                    Event::default()
                        .id(event_id.to_string())
                        .event("agui")
                        .data(payload),
                );
            }

            loop {
                match receiver.recv().await {
                    Ok(payload) => {
                        event_id = event_id.wrapping_add(1);
                        yield Ok(
                            Event::default()
                                .id(event_id.to_string())
                                .event("agui")
                                .data(payload),
                        );
                    }
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        warn!(
                            run_id = %run_id_log,
                            skipped = skipped,
                            "AG-UI subscriber lagged; events dropped"
                        );
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        info!(run_id = %run_id_log, "AG-UI run closed; ending stream");
                        break;
                    }
                }
            }
        };

        Ok(Sse::new(stream).keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(15))
                .text("keepalive"),
        ))
    }
}

/// Translate the registry's `AuthorizedSubscribe` outcome into the
/// route's HTTP-shaped error space.
///
/// The atomic owner-match + subscribe under a held `DashMap` entry
/// borrow lives in [`RunRegistry::authorize_and_subscribe`]; this
/// helper is a pure enum mapping plus structured logging so the HTTP
/// handler stays under the workspace cognitive-complexity budget.
///
/// # Errors
///
/// - [`ErrorCode::ResourceNotFound`] — the `run_id` is not (or no
///   longer) registered.
/// - [`ErrorCode::PermissionDenied`] — the caller authenticated but
///   does not own the run.
fn authorize_and_subscribe(
    registry: &RunRegistry,
    run_id: &str,
    caller: RunOwner,
) -> Result<RunSubscription, AppError> {
    match registry.authorize_and_subscribe(run_id, caller) {
        AuthorizedSubscribe::Ok(subscription) => Ok(subscription),
        AuthorizedSubscribe::Forbidden => {
            warn!(
                run_id = %run_id,
                authenticated_user = %caller.user_id,
                "AG-UI subscribe blocked: caller does not own run"
            );
            Err(AppError::new(
                ErrorCode::PermissionDenied,
                "Cannot subscribe to AG-UI run owned by another user",
            ))
        }
        AuthorizedSubscribe::NotFound => {
            warn!(run_id = %run_id, "AG-UI subscribe for unknown run");
            Err(AppError::not_found(format!("AG-UI run {run_id}")))
        }
    }
}

/// Authenticate the Bearer token on the request.
///
/// Returns the caller's `(user_id, tenant_id)` as a [`RunOwner`] so
/// the handler can compare it to the run's recorded owner.
///
/// # Errors
///
/// Returns an [`AppError`] with [`crate::errors::ErrorCode::AuthInvalid`]
/// when the header is missing, malformed, or the token fails
/// validation by `auth_middleware`.
async fn authenticate_caller(
    headers: &HeaderMap,
    resources: &Arc<ServerResources>,
) -> Result<RunOwner, AppError> {
    let auth_header = headers
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .ok_or_else(|| {
            warn!("Missing Authorization header for AG-UI SSE");
            AppError::auth_invalid(
                "Missing Authorization header - JWT token required for AG-UI stream",
            )
        })?;

    let token = extract_token(auth_header).map_err(|_| {
        warn!("Invalid Authorization header format for AG-UI SSE");
        AppError::auth_invalid("Invalid Authorization header format")
    })?;

    let auth_result = resources
        .auth_middleware
        .authenticate_request(Some(&format!("Bearer {token}")))
        .await
        .map_err(|e| {
            warn!(error = %e, "Failed to authenticate JWT token for AG-UI SSE");
            AppError::auth_invalid(format!("Authentication failed: {e}"))
        })?;

    // AG-UI runs are always tenant-scoped (the chat pipeline registers
    // them with the user's conversation tenant). A JWT without an
    // `active_tenant_id` claim cannot name a tenant to subscribe
    // within, so treat it as unauthenticated for the AG-UI surface.
    let tenant_id = auth_result.active_tenant_id.ok_or_else(|| {
        warn!(
            user_id = %auth_result.user_id,
            "JWT missing active_tenant_id claim; AG-UI subscription denied"
        );
        AppError::auth_invalid("JWT missing active_tenant_id claim")
    })?;

    Ok(RunOwner::new(
        auth_result.user_id,
        TenantId::from_uuid(tenant_id),
    ))
}
