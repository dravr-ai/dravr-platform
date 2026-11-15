// ABOUTME: Explicit cancel of a sciotte sign-in a closed window left running, by the attempt id its steps named
// ABOUTME: Binds the attempt to its caller, so one user never stops another's login, and refuses a cancelled attempt on every pod
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
//! Two things stop a cancelled attempt, and only the first is a guarantee:
//!
//! - **The platform refuses it.** The cancel files the attempt as cancelled in
//!   the shared cache, which every API pod reads. A step naming it is refused
//!   before it reaches the service, and a step already running when the
//!   cancel landed is refused when the service answers, before anything it
//!   brought back is stored or remembered. Whatever the service did, the
//!   attempt links no account.
//! - **The service stops it.** The platform derives a cancel key from the
//!   caller and the attempt — the same on every pod — names it to the service
//!   on each step, and asks the service to cancel it. The service ends the
//!   running step (its browser and permit drop) and releases a flow parked on
//!   a 2FA pick or a code. Its registry is in memory, though, and the service
//!   is not always one instance: a request that arrives while an instance is
//!   still starting can start a second one, so a cancel can land on an
//!   instance that never saw the step (dev, 2026-11-02). That step then runs
//!   to its own budget, and the first guarantee is what keeps its result out.

use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use axum::Json;
use dravr_sciotte::client::SciotteClient;
use dravr_sciotte::wire::LOGIN_CANCELLED;
use pierre_cache::{Cache, CacheKey, CacheResource};
use pierre_core::errors::{AppError, ErrorCode};
use pierre_core::models::TenantId;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::time::Duration;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::sciotte::{
    forget_remote_flow, recall_remote_flow, REMOTE_FLOW_KEY_PROVIDER, REMOTE_FLOW_TTL_SECS,
};
use crate::sciotte_caller::authenticate;
use crate::AuthRoutesContext;

/// Fewest characters an attempt id carries.
const ATTEMPT_ID_MIN_LEN: usize = 8;

/// Most characters an attempt id carries. A UUID, the id the login UIs mint,
/// is 36.
const ATTEMPT_ID_MAX_LEN: usize = 64;

/// How long a cancelled attempt stays refused: the lifetime of the flow it
/// could have parked, past which no step of it can still succeed.
const CANCELLED_ATTEMPT_TTL: Duration = Duration::from_secs(REMOTE_FLOW_TTL_SECS);

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

/// The attempt a sign-in step names, bound to the caller that sent it.
pub(crate) struct StepAttempt {
    tenant_id: Uuid,
    user_id: Uuid,
    attempt_id: String,
    cancel_key: String,
}

impl StepAttempt {
    /// The attempt a step names, or `None` for a step that names none — which
    /// behaves exactly as a step always did.
    ///
    /// # Errors
    ///
    /// [`AppError::invalid_input`] when the step names a malformed attempt id.
    pub(crate) fn of_step(
        tenant_id: Uuid,
        user_id: Uuid,
        attempt_id: Option<&str>,
    ) -> Result<Option<Self>, AppError> {
        attempt_id
            .map(|attempt_id| {
                Ok(Self {
                    tenant_id,
                    user_id,
                    attempt_id: attempt_id.to_owned(),
                    cancel_key: cancel_key_for(tenant_id, user_id, attempt_id)?,
                })
            })
            .transpose()
    }

    /// The key the step names to the service.
    pub(crate) fn cancel_key(&self) -> &str {
        &self.cancel_key
    }

    /// The attempt id the window named.
    pub(crate) fn attempt_id(&self) -> &str {
        &self.attempt_id
    }
}

/// Where a cancelled attempt is filed: under the caller's own tenant and
/// user, so one caller's cancel never refuses another's step.
fn cancelled_attempt_key(tenant_id: Uuid, user_id: Uuid, attempt_id: &str) -> CacheKey {
    CacheKey::new(
        TenantId::from_uuid(tenant_id),
        user_id,
        REMOTE_FLOW_KEY_PROVIDER.to_owned(),
        CacheResource::SciotteLoginCancel {
            attempt_id: attempt_id.to_owned(),
        },
    )
}

/// Whether `attempt`'s window cancelled it.
///
/// A cache read that fails counts as not cancelled: refusing every sign-in
/// while the cache is down would cost more than a closed window's login
/// finishing, which is what happened before cancels existed.
async fn was_cancelled(cache: &Cache, attempt: &StepAttempt) -> bool {
    let key = cancelled_attempt_key(attempt.tenant_id, attempt.user_id, &attempt.attempt_id);
    match cache.get::<bool>(&key).await {
        Ok(found) => found.is_some(),
        Err(e) => {
            warn!(
                user_id = %attempt.user_id,
                error = %e,
                "Failed to read whether the sign-in attempt was cancelled — letting the step run"
            );
            false
        }
    }
}

/// Refuse a step whose attempt its window has already cancelled, before the
/// step reaches the service: no browser starts for it.
///
/// # Errors
///
/// [`login_cancelled_refusal`] when the attempt was cancelled.
pub(crate) async fn refuse_cancelled_attempt(
    cache: &Cache,
    attempt: Option<&StepAttempt>,
) -> Result<(), AppError> {
    let Some(attempt) = attempt else {
        return Ok(());
    };
    if was_cancelled(cache, attempt).await {
        info!(
            user_id = %attempt.user_id,
            "Sciotte sign-in step refused: its window cancelled the attempt"
        );
        return Err(login_cancelled_refusal());
    }
    Ok(())
}

/// Refuse what a step brought back when its window cancelled the attempt
/// while the step ran, before any of it is stored or remembered.
///
/// This is the cancel that reached a service instance the step was not on:
/// the step ran to its end there. The service is asked once more to release
/// whatever the step parked, and the flow the step continued, when it is
/// still the one remembered, is forgotten.
///
/// # Errors
///
/// [`login_cancelled_refusal`] when the attempt was cancelled.
pub(crate) async fn refuse_cancelled_outcome(
    cache: &Cache,
    remote: &SciotteClient,
    attempt: Option<&StepAttempt>,
    continued_flow: Option<&str>,
) -> Result<(), AppError> {
    let Some(attempt) = attempt else {
        return Ok(());
    };
    if !was_cancelled(cache, attempt).await {
        return Ok(());
    }
    if let Err(e) = remote.cancel_login(attempt.cancel_key()).await {
        debug!(error = %e, "Sciotte release of a cancelled step's flow did not arrive");
    }
    if let Some(flow_id) = continued_flow {
        let remembered = recall_remote_flow(cache, attempt.tenant_id, attempt.user_id).await;
        if remembered.is_some_and(|flow| flow.flow_id == flow_id) {
            forget_remote_flow(cache, attempt.tenant_id, attempt.user_id).await;
        }
    }
    info!(
        user_id = %attempt.user_id,
        "Sciotte sign-in step outlived its cancel: its result is dropped"
    );
    Err(login_cancelled_refusal())
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

/// File `attempt_id` as cancelled for its caller; whether the cache took it.
async fn file_cancelled_attempt(
    cache: &Cache,
    tenant_id: Uuid,
    user_id: Uuid,
    attempt_id: &str,
) -> bool {
    let marker = cancelled_attempt_key(tenant_id, user_id, attempt_id);
    match cache.set(&marker, &true, CANCELLED_ATTEMPT_TTL).await {
        Ok(()) => true,
        Err(e) => {
            warn!(
                %user_id,
                error = %e,
                "Failed to file the sign-in attempt as cancelled — only the service can stop it"
            );
            false
        }
    }
}

/// Ask the service to stop the steps under `cancel_key`; whether it ended a
/// running step or released a parked flow. A released flow is forgotten.
async fn stop_on_service(
    cache: &Cache,
    remote: &SciotteClient,
    tenant_id: Uuid,
    user_id: Uuid,
    cancel_key: &str,
) -> bool {
    match remote.cancel_login(cancel_key).await {
        Ok(outcome) => {
            if outcome.released_parked {
                forget_remote_flow(cache, tenant_id, user_id).await;
            }
            info!(
                %user_id,
                cancelled_running = outcome.cancelled_running,
                released_parked = outcome.released_parked,
                "Sciotte answered the sign-in cancel"
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
    }
}

/// `POST /api/providers/sciotte/cancel`: stop the caller's sign-in attempt.
///
/// Authenticated as the login is, a hosted page's link-token included. The
/// attempt is filed as cancelled first, so no step of it links an account
/// from then on, then the service is asked to stop it. Answers
/// `{"cancelled": true}` when either took hold, `false` when neither did. A
/// flow the service released is also forgotten here, so the next
/// continuation does not name a flow the service let go.
///
/// Best effort: a cache or a service that cannot be reached is logged at
/// `WARN`, never answered as a failure, since nobody waits on the answer and
/// no operator has anything to do about a cancel that did not arrive (the
/// login ends on its own budget).
pub(crate) async fn handle_sciotte_cancel(
    State(resources): State<AuthRoutesContext>,
    headers: HeaderMap,
    Json(request): Json<SciotteCancelRequest>,
) -> Result<Response, AppError> {
    let (user_id, tenant_id, _link_context) = authenticate(&resources, &headers).await?;
    let cancelled = cancel_attempt(&resources, tenant_id, user_id, &request.attempt_id).await?;
    Ok(Json(serde_json::json!({ "cancelled": cancelled })).into_response())
}

/// Cancel the caller's sign-in attempt `attempt_id`: file it as cancelled,
/// so no step of it links an account from then on, then ask the service to
/// stop it. Whether either took hold.
///
/// # Errors
///
/// [`AppError::invalid_input`] for a malformed attempt id, and the error of
/// an unconfigured scraper service.
pub(crate) async fn cancel_attempt(
    resources: &AuthRoutesContext,
    tenant_id: Uuid,
    user_id: Uuid,
    attempt_id: &str,
) -> Result<bool, AppError> {
    let cancel_key = cancel_key_for(tenant_id, user_id, attempt_id)?;
    let remote = resources.provider_registry.require_sciotte_client()?;
    let filed = file_cancelled_attempt(&resources.cache, tenant_id, user_id, attempt_id).await;
    let stopped = stop_on_service(&resources.cache, remote, tenant_id, user_id, &cancel_key).await;
    info!(%user_id, filed, stopped, "Sciotte sign-in cancelled by its caller");
    Ok(filed || stopped)
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
        assert!(
            StepAttempt::of_step(Uuid::from_u128(1), Uuid::from_u128(2), None)
                .unwrap()
                .is_none(),
            "a step naming no attempt behaves as a step always did"
        );
        assert!(
            StepAttempt::of_step(Uuid::from_u128(1), Uuid::from_u128(2), Some("bad")).is_err(),
            "a malformed attempt id is refused"
        );
    }

    #[test]
    fn a_step_names_the_key_its_attempt_derives() {
        let (tenant, user) = (Uuid::from_u128(1), Uuid::from_u128(2));
        let attempt = StepAttempt::of_step(tenant, user, Some("attempt-1"))
            .unwrap()
            .unwrap();
        assert_eq!(
            attempt.cancel_key(),
            sciotte_cancel_key(tenant, user, "attempt-1"),
            "a step names the same key its window's cancel derives, else the cancel misses it"
        );
    }

    #[test]
    fn a_cancelled_attempt_is_filed_under_its_caller() {
        let tenant = Uuid::from_u128(1);
        let user = Uuid::from_u128(2);
        assert_eq!(
            cancelled_attempt_key(tenant, user, "attempt-1").to_string(),
            format!("tenant:{tenant}:user:{user}:provider:sciotte:sciotte_login_cancel:attempt-1"),
            "the marker is keyed by the caller's tenant and user"
        );
        assert_ne!(
            cancelled_attempt_key(tenant, Uuid::from_u128(3), "attempt-1"),
            cancelled_attempt_key(tenant, user, "attempt-1"),
            "another caller's same attempt id is another entry"
        );
    }
}
