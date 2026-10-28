// ABOUTME: Garmin push webhook — checks the path token and client id, stores the raw batch, answers 200, then ingests it
// ABOUTME: One route per summary type under /webhooks/garmin/{token}/; the batch is durable before Garmin hears back

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Garmin push webhook
//!
//! The Garmin Connect Developer Program pushes each enabled summary type to
//! its own URL, configured in Garmin's developer portal as
//! `https://<host>/webhooks/garmin/<token>/<summaryType>` (`activities`,
//! `sleeps`, …, `deregistrations`, `userPermissionsChange`). The path is a
//! sub-path because the billing router claims the single segment
//! `/webhooks/{provider}`.
//!
//! Garmin signs nothing, so a push must present three things a forger has to
//! guess: the path token (`GARMIN_WEBHOOK_TOKEN`, compared in constant time),
//! the `garmin-client-id` header naming Dravr's Garmin client, and a `userId`
//! on each record that a stored Garmin connection captured (checked when the
//! batch is processed: a record naming nobody is dropped).
//!
//! Garmin wants a 200 within thirty seconds, before any processing, and does
//! not resend a batch it saw answered. So the raw body is written to
//! `garmin_push_inbox` before the answer, leased to this instance, and
//! ingested afterwards on the drain-tracked spawner; when this instance dies
//! first, the inbox sweep runs it elsewhere (`process_garmin_inbox`). A batch that cannot
//! be stored is answered 503, which Garmin retries.

mod worker;

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, FromRequestParts, Path, State};
use axum::http::request::Parts;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use axum::Router;
use chrono::Utc;
use secrecy::{ExposeSecret, SecretString};
use tracing::{error, info, warn};
use uuid::Uuid;

use pierre_core::constant_time::matches_configured_secret;
use pierre_database::repositories::{GarminPushInboxRow, JobLease, INBOX_PENDING};
use pierre_services::garmin_push::{parse_envelope, EnvelopeError, SummaryType};

use crate::mcp::resources::ServerContext;

pub use worker::{process_garmin_inbox, start_garmin_push_worker};

/// The header Garmin names the pushing client in.
const CLIENT_ID_HEADER: &str = "garmin-client-id";

/// The largest push body accepted, 32 MiB.
///
/// `activityDetails` carries every sample an activity recorded, up to one per
/// second, for activities up to 24 hours long (longer ones arrive as files).
/// A sample is about a dozen numeric fields, roughly 300–400 bytes of JSON,
/// so a 24-hour activity is about 86,400 × 350 B ≈ 30 MB. Axum's 2 MiB
/// default would refuse anything past an hour and a half of samples. 32 MiB
/// also matches Cloud Run's own ceiling on an HTTP/1 request body, past which
/// the request never reaches the server anyway.
const GARMIN_PUSH_BODY_LIMIT: usize = 32 * 1024 * 1024;

/// The Garmin push routes, on the webhook router's state.
pub(super) fn routes() -> Router<Arc<ServerContext>> {
    Router::new().route(
        "/webhooks/garmin/{token}/{summary_type}",
        post(receive_push).layer(DefaultBodyLimit::max(GARMIN_PUSH_BODY_LIMIT)),
    )
}

/// Receive one pushed batch: authenticate it, store it, answer, ingest it.
///
/// Refusals: 503 while the path token or Dravr's Garmin client id is not
/// configured (nothing could authenticate a push), 401 for a wrong path
/// token, 403 for a push naming another client, 400 for a body that is not
/// one summary list or whose list is not the path's type, 404 for a summary
/// type Dravr does not ingest, 503 when the batch could not be stored.
///
/// The push is authenticated by [`AuthenticatedPush`] before the body is
/// read: axum runs extractors in order and stops at the first refusal, so a
/// caller without the token never gets a 32 MiB body buffered.
async fn receive_push(
    State(resources): State<Arc<ServerContext>>,
    AuthenticatedPush { summary_type }: AuthenticatedPush,
    body: Bytes,
) -> StatusCode {
    let kind = match summary_of(&body, &summary_type) {
        Ok(kind) => kind,
        Err(refusal) => return refusal,
    };
    let (row, lease) = match store(&resources, kind, &body).await {
        Ok(stored) => stored,
        Err(refusal) => return refusal,
    };
    let spawned = Arc::clone(&resources);
    resources.common.turns.clone().spawn(Box::pin(async move {
        worker::process_push(&spawned, row, lease).await;
    }));
    StatusCode::OK
}

/// A push whose path token and `garmin-client-id` header both check out,
/// with the summary type its URL names. Extracted from the request's head
/// alone, so a refusal never reads the body.
struct AuthenticatedPush {
    summary_type: String,
}

impl FromRequestParts<Arc<ServerContext>> for AuthenticatedPush {
    type Rejection = StatusCode;

    async fn from_request_parts(
        parts: &mut Parts,
        resources: &Arc<ServerContext>,
    ) -> Result<Self, Self::Rejection> {
        let Path((token, summary_type)) =
            Path::<(String, String)>::from_request_parts(parts, resources)
                .await
                .map_err(|_| StatusCode::NOT_FOUND)?;
        authenticate(resources, &token, &parts.headers)?;
        Ok(Self { summary_type })
    }
}

/// Check the push's path token and the client it names.
fn authenticate(
    resources: &ServerContext,
    token: &str,
    headers: &HeaderMap,
) -> Result<(), StatusCode> {
    let (expected_token, client_id) = configured(resources)?;
    if !matches_configured_secret(expected_token.expose_secret().as_bytes(), token.as_bytes()) {
        warn!("Garmin push refused: the path token does not match");
        return Err(StatusCode::UNAUTHORIZED);
    }
    let presented_client = headers
        .get(CLIENT_ID_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::trim);
    if presented_client != Some(client_id) {
        warn!("Garmin push refused: the garmin-client-id header does not name Dravr's client");
        return Err(StatusCode::FORBIDDEN);
    }
    Ok(())
}

/// The path token and Dravr's Garmin client id, or the 503 a push gets while
/// either is unset: nothing could authenticate it.
fn configured(resources: &ServerContext) -> Result<(&SecretString, &str), StatusCode> {
    let config = &resources.common.config;
    let Some(expected_token) = config.webhooks.garmin_token.as_ref() else {
        warn!("Garmin push refused: GARMIN_WEBHOOK_TOKEN is unset; nothing stored");
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    };
    let Some(client_id) = config
        .oauth
        .garmin
        .client_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
    else {
        warn!("Garmin push refused: GARMIN_CLIENT_ID is unset; nothing stored");
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    };
    Ok((expected_token, client_id))
}

/// The summary type a body carries, when it is the one its URL names.
fn summary_of(body: &[u8], summary_type: &str) -> Result<SummaryType, StatusCode> {
    let kind = match parse_envelope(body) {
        Ok((kind, _)) => kind,
        Err(EnvelopeError::UnknownType) => {
            warn!(summary_type = %summary_type, "Garmin push of a summary type Dravr does not ingest");
            return Err(StatusCode::NOT_FOUND);
        }
        Err(EnvelopeError::Malformed(why)) => {
            warn!(reason = why, "Garmin push body is not a summary list");
            return Err(StatusCode::BAD_REQUEST);
        }
    };
    if kind.key() != summary_type {
        warn!(
            summary_type = %summary_type,
            body_type = kind.key(),
            "Garmin push to one summary type's URL carries another type"
        );
        return Err(StatusCode::BAD_REQUEST);
    }
    Ok(kind)
}

/// Write the batch to the inbox, leased to this instance for the run the
/// route starts.
async fn store(
    resources: &ServerContext,
    kind: SummaryType,
    body: &[u8],
) -> Result<(GarminPushInboxRow, JobLease), StatusCode> {
    // The envelope parsed as JSON, so the body is UTF-8.
    let payload = String::from_utf8(body.to_vec()).map_err(|_| StatusCode::BAD_REQUEST)?;
    let now_ms = Utc::now().timestamp_millis();
    let lease = JobLease {
        id: Uuid::new_v4().to_string(),
        token: Uuid::new_v4().to_string(),
    };
    let row = GarminPushInboxRow {
        id: lease.id.clone(),
        summary_type: kind.key().to_owned(),
        received_at_ms: now_ms,
        payload,
        status: INBOX_PENDING.to_owned(),
        attempts: 1,
        last_error: None,
        processed_at_ms: None,
        leased_until_ms: now_ms.saturating_add(worker::lease_ms()),
        lease_token: Some(lease.token.clone()),
    };
    if let Err(e) = resources
        .common
        .repos
        .garmin_push_inbox
        .record_push(&row)
        .await
    {
        error!(
            summary_type = kind.key(),
            error = %e,
            "Garmin push could not be stored; answered 503 so Garmin sends it again"
        );
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    info!(
        summary_type = kind.key(),
        inbox_id = %row.id,
        body_len = body.len(),
        "Garmin push stored"
    );
    Ok((row, lease))
}
