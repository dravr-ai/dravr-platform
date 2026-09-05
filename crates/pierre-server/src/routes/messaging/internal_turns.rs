// ABOUTME: POST /internal/turns/{id}/run — the request Cloud Tasks delivers to run one recorded messaging turn
// ABOUTME: Verifies the task's OIDC token, claims the row in conversation order, runs the turn inside the request
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The turn as a request Cloud Run can see.
//!
//! On GCP a messaging turn is not run by the webhook that received it; the
//! webhook records the turn and enqueues its id, and Cloud Tasks delivers
//! this request. The turn runs inside it, so the instance is processing a
//! request for the turn's whole duration and an idle scaledown never picks
//! it (registre#126). The route is mounted only when the Cloud Tasks runner
//! is in use; everywhere else it does not exist.
//!
//! The answer tells Cloud Tasks what to do next: `200` closes the task —
//! the turn was answered, apologised for, or is already gone; `409` asks for
//! a retry — the row is leased to a run that is still going, an older turn
//! of the same conversation is still on file, or this instance drained
//! mid-turn and handed the row back. Never `429` or `503`: Cloud Tasks
//! throttles the whole queue on those, every tenant included.

use std::sync::Arc;
use std::time::Instant;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::{Extension, Json};
use pierre_core::errors::AppError;
use pierre_core::models::TenantId;
use pierre_database::repositories::{ResumableTurnRepository, ResumableTurnRow, TurnClaim};
use serde::Deserialize;
use serde_json::json;
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use super::adapter_factory::ChannelAdapterFactory;
use crate::mcp::resources::ServerContext;
use crate::services::messaging_ingress::resume::{
    run_lease, run_recorded_turn, CLAIM_RETRY, MAX_TURN_ATTEMPTS,
};
use crate::services::messaging_ingress::TurnClose;

/// The body a task carries: the session tenant the row lives under, so the
/// claim runs under the per-tenant filter every statement on the table has.
#[derive(Debug, Deserialize)]
pub struct RunTurnBody {
    /// Session tenant of the recorded turn.
    pub tenant_id: String,
}

/// Run one recorded turn inside this request.
///
/// # Errors
///
/// `401` when the bearer token is missing or is not a Google ID token minted
/// for this service's audience on behalf of the turn runner's service
/// account; `404` when the Cloud Tasks runner is not in use.
pub async fn run_turn(
    State(resources): State<Arc<ServerContext>>,
    Extension(adapters): Extension<Arc<dyn ChannelAdapterFactory>>,
    Path(turn_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<RunTurnBody>,
) -> Result<impl IntoResponse, AppError> {
    let Some(runner) = resources.common.turn_runner.cloud_tasks() else {
        return Err(AppError::not_found("turn runner"));
    };

    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or_else(|| {
            warn!(turn_id = %turn_id, "turn delivery carried no bearer token");
            AppError::auth_required()
        })?;
    runner.verifier().verify(token).await?;

    let tenant_id = TenantId::parse_str(&body.tenant_id)
        .map_err(|_| AppError::invalid_input("tenant_id is not a tenant id"))?;
    let repo = resources.common.repos.resumable_turns.as_ref();
    let deadline = Instant::now() + runner.claim_wait();
    let drain = resources.common.turns.drain_token();

    let row = match claim_for_delivery(repo, tenant_id, &turn_id, deadline, &drain).await? {
        Delivery::Run(row) => row,
        Delivery::Answer(status, outcome) => return Ok(answer(status, outcome)),
    };
    Ok(
        match run_recorded_turn(&resources, adapters.as_ref(), *row).await {
            Some(TurnClose::HandedOff) => {
                info!(turn_id = %turn_id, "turn drained mid-run; asking Cloud Tasks to deliver it again");
                answer(StatusCode::CONFLICT, "drained")
            }
            Some(TurnClose::Finished) => answer(StatusCode::OK, "finished"),
            None => answer(StatusCode::OK, "unrunnable"),
        },
    )
}

/// What a delivery does once the claim has been settled.
enum Delivery {
    /// The row is leased to this request; run it.
    Run(Box<ResumableTurnRow>),
    /// Nothing to run; this is the answer Cloud Tasks gets.
    Answer(StatusCode, &'static str),
}

/// Claim the delivered turn, waiting in the request while an older turn of
/// the same conversation is still answering, and stop waiting when the wait
/// runs out or this instance starts draining.
async fn claim_for_delivery(
    repo: &dyn ResumableTurnRepository,
    tenant_id: TenantId,
    turn_id: &str,
    deadline: Instant,
    drain: &CancellationToken,
) -> Result<Delivery, AppError> {
    loop {
        match repo
            .claim_resumable_turn(tenant_id, turn_id, &run_lease())
            .await?
        {
            TurnClaim::Claimed(row) => return Ok(Delivery::Run(row)),
            TurnClaim::Blocked if Instant::now() < deadline && !drain.is_cancelled() => {
                sleep(CLAIM_RETRY).await;
            }
            TurnClaim::Blocked => {
                info!(turn_id, "turn still blocked after the claim wait; asking Cloud Tasks to deliver it again");
                return Ok(Delivery::Answer(StatusCode::CONFLICT, "blocked"));
            }
            TurnClaim::Missing => {
                info!(turn_id, "turn already finished; nothing to run");
                return Ok(Delivery::Answer(StatusCode::OK, "finished"));
            }
            TurnClaim::Exhausted => {
                error!(
                    turn_id,
                    max_attempts = MAX_TURN_ATTEMPTS,
                    "turn delivered past its attempt cap; the sweep reaps it and the athlete was not told"
                );
                return Ok(Delivery::Answer(StatusCode::OK, "exhausted"));
            }
        }
    }
}

/// The status Cloud Tasks reads and the word an operator reads.
fn answer(status: StatusCode, outcome: &str) -> (StatusCode, Json<serde_json::Value>) {
    (status, Json(json!({ "status": outcome })))
}
