// ABOUTME: Explicit cancel of a sciotte sign-in a closed window left running, by the attempt id its steps named
// ABOUTME: Binds the attempt to its caller in a cancel key, so one user can never stop another user's login
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Closing a sign-in window aborts its request, but that disconnect does not
//! reach the scraper service: the frontend proxy, the API and the service are
//! separate hops, and none of them carries a client's dropped connection on
//! to the next. The login kept its browser and its scrape permit until it
//! finished on its own. So a window names each opening with an attempt id,
//! sends it on every step, and on close asks for that attempt to be cancelled.
//!
//! The platform keeps no registry of attempts: the cancel can land on another
//! API instance than the step it cancels. It derives a cancel key from the
//! caller and the attempt instead — the same on every instance — and names it
//! to the service on each step and on the cancel. The service, which runs as
//! one instance, holds the registry: a cancel ends the running step (its
//! browser and permit drop) and releases a flow parked on a 2FA pick or a
//! code, and the step it ended answers `login_cancelled`.

use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use axum::Json;
use dravr_sciotte::wire::LOGIN_CANCELLED;
use pierre_core::errors::{AppError, ErrorCode};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tracing::{info, warn};
use uuid::Uuid;

use crate::sciotte::{authenticate, forget_remote_flow};
use crate::AuthRoutesContext;

/// Fewest characters an attempt id carries.
const ATTEMPT_ID_MIN_LEN: usize = 8;

/// Most characters an attempt id carries. A UUID, the id the login UIs mint,
/// is 36.
const ATTEMPT_ID_MAX_LEN: usize = 64;

/// Body of `POST /api/providers/sciotte/cancel`.
#[derive(Debug, Deserialize)]
pub(crate) struct SciotteCancelRequest {
    /// The attempt id the window's steps named.
    pub attempt_id: String,
}

/// The key the service files `attempt_id`'s steps under: lowercase hex
/// SHA-256 of `"{tenant_id}:{user_id}:{attempt_id}"`.
///
/// The caller's tenant and user are part of it, so the same attempt id sent
/// by anyone else names another key, and a cancel can only reach the
/// caller's own login. Every API instance derives the same key, so a cancel
/// served by one instance reaches a step another one is running.
#[must_use]
fn sciotte_cancel_key(tenant_id: Uuid, user_id: Uuid, attempt_id: &str) -> String {
    hex::encode(Sha256::digest(format!(
        "{tenant_id}:{user_id}:{attempt_id}"
    )))
}

/// Whether `attempt_id` is one the platform accepts: 8 to 64 ASCII letters,
/// digits or hyphens.
fn is_valid_attempt_id(attempt_id: &str) -> bool {
    (ATTEMPT_ID_MIN_LEN..=ATTEMPT_ID_MAX_LEN).contains(&attempt_id.len())
        && attempt_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

/// The caller's cancel key for `attempt_id`.
///
/// # Errors
///
/// [`AppError::invalid_input`] (a `400`) when `attempt_id` is not 8 to 64
/// letters, digits or hyphens.
fn cancel_key_for(tenant_id: Uuid, user_id: Uuid, attempt_id: &str) -> Result<String, AppError> {
    if !is_valid_attempt_id(attempt_id) {
        return Err(AppError::invalid_input(
            "attempt_id must be 8 to 64 letters, digits or hyphens",
        ));
    }
    Ok(sciotte_cancel_key(tenant_id, user_id, attempt_id))
}

/// The cancel key a sign-in step names to the service, or `None` for a step
/// that names no attempt — which behaves exactly as a step always did.
///
/// # Errors
///
/// [`AppError::invalid_input`] when the step names a malformed attempt id.
pub(crate) fn step_cancel_key(
    tenant_id: Uuid,
    user_id: Uuid,
    attempt_id: Option<&str>,
) -> Result<Option<String>, AppError> {
    attempt_id
        .map(|attempt_id| cancel_key_for(tenant_id, user_id, attempt_id))
        .transpose()
}

/// The answer to a step the service ended because its caller cancelled it.
///
/// Its `details.reason` is the service's own [`LOGIN_CANCELLED`] marker.
/// Nobody reads it in practice — the window that sent the step is gone — but
/// a client still running names it this way.
///
/// A `409`, which `response_failure_log_middleware` records below `ERROR`, and
/// routed around the system-failure path: the window that sent the step
/// closed and asked for exactly this, so there is nothing for an operator to
/// see and no failed sync to count.
pub(crate) fn login_cancelled_refusal() -> AppError {
    let mut refusal = AppError::new(ErrorCode::ResourceLocked, "This sign-in was cancelled.");
    refusal.details = Some(Box::new(serde_json::json!({ "reason": LOGIN_CANCELLED })));
    refusal
}

/// `POST /api/providers/sciotte/cancel`: stop the caller's sign-in attempt.
///
/// Authenticated as the login is, a hosted page's link-token included.
/// Answers `{"cancelled": true}` when the service ended a running step or
/// released a parked flow, `false` when there was nothing under the key —
/// a late cancel is harmless. A released flow is also forgotten here, so the
/// next continuation does not name a flow the service let go.
///
/// Best effort: a service that cannot be reached is logged at `WARN` and
/// answered `{"cancelled": false}`, never as a failure, since nobody waits on
/// the answer and no operator has anything to do about a cancel that did not
/// arrive (the login ends on its own budget).
pub(crate) async fn handle_sciotte_cancel(
    State(resources): State<AuthRoutesContext>,
    headers: HeaderMap,
    Json(request): Json<SciotteCancelRequest>,
) -> Result<Response, AppError> {
    let (user_id, tenant_id, _link_context) = authenticate(&resources, &headers).await?;
    let cancel_key = cancel_key_for(tenant_id, user_id, &request.attempt_id)?;
    let remote = resources.provider_registry.require_sciotte_client()?;

    let cancelled = match remote.cancel_login(&cancel_key).await {
        Ok(outcome) => {
            if outcome.released_parked {
                forget_remote_flow(&resources.cache, tenant_id, user_id).await;
            }
            info!(
                %user_id,
                cancelled_running = outcome.cancelled_running,
                released_parked = outcome.released_parked,
                "Sciotte sign-in cancelled by its caller"
            );
            outcome.cancelled_running || outcome.released_parked
        }
        Err(e) => {
            warn!(
                %user_id,
                error = %e,
                "Sciotte sign-in cancel did not reach the service — the login ends on its own"
            );
            false
        }
    };
    Ok(Json(serde_json::json!({ "cancelled": cancelled })).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_attempt_id_is_eight_to_sixty_four_letters_digits_or_hyphens() {
        for accepted in [
            "0b7e3c1a-5f2d-4e8b-9a61-2c4d8e0f1a3b",
            "abcdefgh",
            &"A".repeat(ATTEMPT_ID_MAX_LEN),
        ] {
            assert!(is_valid_attempt_id(accepted), "{accepted}");
        }
        for refused in [
            "",
            "short-7",
            &"a".repeat(ATTEMPT_ID_MAX_LEN + 1),
            "has space here",
            "under_score_",
            "dot.ted.attempt",
            "ünïcødé-attempt",
        ] {
            assert!(!is_valid_attempt_id(refused), "{refused}");
        }
    }

    #[test]
    fn the_key_is_the_hex_sha256_of_tenant_user_and_attempt() {
        let tenant = Uuid::from_u128(1);
        let user = Uuid::from_u128(2);
        let key = sciotte_cancel_key(tenant, user, "attempt-1");
        let expected = hex::encode(Sha256::digest(
            "00000000-0000-0000-0000-000000000001:00000000-0000-0000-0000-000000000002:attempt-1",
        ));
        assert_eq!(key, expected, "the key is SHA-256 over tenant:user:attempt");
        assert_eq!(key.len(), 64, "a hex SHA-256 is 64 characters");
        assert!(
            key.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
            "the key is lowercase hex: {key}"
        );
    }

    #[test]
    fn a_step_without_an_attempt_names_no_key() {
        assert_eq!(
            step_cancel_key(Uuid::from_u128(1), Uuid::from_u128(2), None).unwrap(),
            None,
            "a step naming no attempt behaves as a step always did"
        );
        assert!(
            step_cancel_key(Uuid::from_u128(1), Uuid::from_u128(2), Some("bad")).is_err(),
            "a malformed attempt id is refused"
        );
    }
}
