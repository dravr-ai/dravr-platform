// ABOUTME: The routine engine's two doors: the wake Cloud Tasks delivers, and the admin reconcile the cron calls
// ABOUTME: The wake runs the firing inside the request, so Cloud Run keeps the instance up exactly as long as it runs

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Routine engine routes (ADR-036, carnet#1790).
//!
//! - `POST /internal/routines/wake` — a queued wake, delivered by Cloud Tasks
//!   with an OIDC token minted for this service ([`wake`]). The messaging
//!   router mounts it beside the turn-run route, only where Cloud Tasks
//!   delivers; locally the process is its own waker.
//! - `POST /admin/routines/reconcile` — queue every pending wake again
//!   ([`reconcile`]). The 3-hourly capture cron calls it, so a wake Cloud
//!   Tasks lost is queued again within three hours.

use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::middleware;
use axum::response::IntoResponse;
use axum::routing::post;
use axum::{Extension, Json, Router};
use dravr_tronc::dates::Timestamp;
use pierre_core::admin::models::{AdminPermission, ValidatedAdminToken};
use pierre_core::errors::{AppError, AppResult};
use pierre_routes_admin::auth::middleware::admin_auth_middleware;
use pierre_routes_admin::auth::service::AdminAuthService;
use serde_json::json;
use tracing::warn;

use crate::mcp::resources::ServerContext;
use crate::services::routine_engine::{RoutineEngine, WakeOutcome};
use crate::services::routine_waker::Wake;

/// The admin reconcile route, behind the admin-token middleware.
pub fn admin_routes(resources: Arc<ServerContext>, auth_service: AdminAuthService) -> Router {
    Router::new()
        .route("/admin/routines/reconcile", post(reconcile))
        .with_state(resources)
        .layer(middleware::from_fn_with_state(
            auth_service,
            admin_auth_middleware,
        ))
}

/// The installed engine, or 404 before the composition root installed it.
fn engine(resources: &ServerContext) -> AppResult<Arc<RoutineEngine>> {
    resources
        .mcp
        .routines
        .get()
        .cloned()
        .ok_or_else(|| AppError::not_found("routine engine"))
}

/// `POST /internal/routines/wake`: run the wake the body names.
///
/// Authenticated by the OIDC token alone, exactly as a turn delivery is: the
/// backend runs with invoker IAM off. A wake that no longer names its
/// routine's next firing answers 200 with `stale`, so Cloud Tasks stops
/// retrying it.
///
/// # Errors
///
/// 401 for a missing or refused token; 400 for a body that is not a wake; an
/// error when the routine cannot be read, which Cloud Tasks retries.
pub async fn wake(
    State(resources): State<Arc<ServerContext>>,
    headers: HeaderMap,
    Json(wake): Json<Wake>,
) -> AppResult<impl IntoResponse> {
    let Some(runner) = resources.common.turn_runner.cloud_tasks() else {
        return Err(AppError::not_found("routine waker"));
    };
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or_else(|| {
            warn!(task = %wake.task_id(), "routine wake carried no bearer token");
            AppError::auth_required()
        })?;
    runner.verify_delivery(token).await?;
    let outcome = engine(&resources)?.handle(wake, Timestamp::now()).await?;
    Ok((
        StatusCode::OK,
        Json(json!({ "status": outcome_label(outcome) })),
    ))
}

/// `POST /admin/routines/reconcile`: queue every pending wake again.
///
/// # Errors
///
/// When the admin token lacks [`AdminPermission::ManageConfiguration`], or
/// the tenant list cannot be read.
pub async fn reconcile(
    State(resources): State<Arc<ServerContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
) -> AppResult<impl IntoResponse> {
    // It queues work on Cloud Tasks, so it takes the manage permission.
    admin_token.require_permission(&AdminPermission::ManageConfiguration)?;
    let outcome = engine(&resources)?.reconcile(Timestamp::now()).await?;
    Ok((
        StatusCode::OK,
        Json(json!({
            "routines": outcome.routines,
            "queued": outcome.queued,
            "errors": outcome.errors,
        })),
    ))
}

/// A wake's outcome as the delivery's body names it.
const fn outcome_label(outcome: WakeOutcome) -> &'static str {
    match outcome {
        WakeOutcome::Posted(_) => "posted",
        WakeOutcome::Withheld(_) => "withheld",
        WakeOutcome::Deferred(_) => "deferred",
        WakeOutcome::Held(reason) => reason.as_str(),
        WakeOutcome::Skipped => "skipped",
        WakeOutcome::Stale => "stale",
        WakeOutcome::Ran => "ran",
    }
}
