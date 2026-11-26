// ABOUTME: The door Cloud Tasks delivers a refused sign-in's follow-up to, ten minutes after the refusal
// ABOUTME: OIDC-verified like a routine wake; answers 200 with what the check found so the queue stops retrying
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `POST /internal/sign-in/follow-up` (carnet#1837).
//!
//! The messaging router mounts it beside the turn-run route and the routine
//! wake, only where Cloud Tasks delivers; locally the process holds each
//! follow-up as a timer and the path is absent.

use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use dravr_tronc::dates::Timestamp;
use pierre_core::errors::{AppError, AppResult};
use pierre_routes_auth::sign_in_follow_up::{check, FollowUp};
use serde_json::json;
use tracing::warn;

use crate::mcp::resources::ServerContext;

/// `POST /internal/sign-in/follow-up`: run the follow-up the body names.
///
/// It checks whether the athlete connected after the refusal it was queued
/// for.
///
/// Authenticated by the OIDC token alone, exactly as a routine wake is: the
/// backend runs with invoker IAM off. A follow-up whose refusal is no longer
/// the athlete's last answers 200 with `superseded`, so Cloud Tasks stops
/// retrying it; so do `still_not_connected` and `recovered`, after the line
/// each logs.
///
/// # Errors
///
/// 401 for a missing or refused token; 400 for a body that is not a
/// follow-up; an error when the connections cannot be read, which Cloud Tasks
/// retries.
pub async fn deliver(
    State(resources): State<Arc<ServerContext>>,
    headers: HeaderMap,
    Json(follow_up): Json<FollowUp>,
) -> AppResult<impl IntoResponse> {
    let Some(runner) = resources.common.turn_runner.cloud_tasks() else {
        return Err(AppError::not_found("sign-in follow-up"));
    };
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or_else(|| {
            warn!(task = %follow_up.task_id(), "sign-in follow-up carried no bearer token");
            AppError::auth_required()
        })?;
    runner.verify_delivery(token).await?;
    let verdict = check(
        &resources.common.cache,
        &resources.common.repos,
        &follow_up,
        Timestamp::now(),
    )
    .await?;
    Ok((StatusCode::OK, Json(json!({ "status": verdict.as_str() }))))
}
