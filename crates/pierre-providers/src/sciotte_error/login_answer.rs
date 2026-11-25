// ABOUTME: The platform error for a session-read service answer that is no login outcome: a cancelled step, a lost flow, a login step with no status
// ABOUTME: A code step the service no longer holds a browser for is a lapsed sign-in, never a system failure (carnet#1504)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use dravr_sciotte::client::{ClientError, Operation, StatusCode};
use dravr_sciotte::wire::{LOGIN_CANCELLED, NO_PENDING_LOGIN};
use pierre_core::errors::{AppError, ErrorCode};
use tracing::warn;

use super::{answer_marker, sciotte_refusal, with_refusal_marker};

/// The platform error for an answer the client could not read as a login
/// outcome or a read: a cancelled step, a code step whose sign-in the
/// service no longer holds, a login step with no status, or anything else.
pub(super) fn unexpected_error(error: ClientError, operation: Option<Operation>) -> AppError {
    match error {
        cancelled @ ClientError::Unexpected { .. } if cancelled.is_login_cancelled() => {
            with_refusal_marker(
                AppError::new(ErrorCode::ResourceLocked, "The sign-in was cancelled"),
                LOGIN_CANCELLED,
            )
        }
        lost @ ClientError::Unexpected { .. } if is_lost_flow(&lost, operation) => {
            warn!(
                error = %lost,
                "the session-read service no longer holds the sign-in a code step continues"
            );
            with_refusal_marker(
                AppError::invalid_input(
                    "The sign-in service no longer holds this sign-in; start the sign-in again",
                ),
                NO_PENDING_LOGIN,
            )
        }
        // LIMITATION(registre#746): the login-step `ClientError::Unexpected` arm also receives a
        // login that ran out its budget, which the service answers `401 session_expired`.
        ClientError::Unexpected {
            exchange,
            status,
            body,
        } if is_login_step(operation) => AppError::internal(format!(
            "sciotte returned HTTP {status} with no recognizable login status \
             (x-request-id {}, marker: {}); check DRAVR_SCIOTTE_AUDIENCE / IAM / \
             service health",
            exchange.request_id,
            answer_marker(&body)
        )),
        unexpected => AppError::internal(unexpected.to_string()),
    }
}

/// Whether `operation` is one of the three steps of an interactive login.
const fn is_login_step(operation: Option<Operation>) -> bool {
    matches!(
        operation,
        Some(Operation::Login | Operation::SubmitOtp | Operation::Select2fa)
    )
}

/// Whether `error` is the service's `404 no_pending_login` to a code step:
/// the instance that parked the sign-in's browser is gone (scaled down,
/// redeployed) or reaped it, so the step continues nothing (carnet#1504).
///
/// A `no_pending_login` marker under any other status stays a system failure:
/// the service only ever answers it as a `404`.
fn is_lost_flow(error: &ClientError, operation: Option<Operation>) -> bool {
    let ClientError::Unexpected { status, body, .. } = error else {
        return false;
    };
    matches!(operation, Some(Operation::SubmitOtp | Operation::Select2fa))
        && *status == StatusCode::NOT_FOUND
        && answer_marker(body) == NO_PENDING_LOGIN
}

/// Whether `error` is a code step whose sign-in the service no longer holds.
///
/// Callers answer it as a lapsed sign-in the athlete starts again, never
/// through the system-failure path: nothing is broken, the browser that
/// waited for the code is gone.
#[must_use]
pub fn is_login_flow_lost(error: &AppError) -> bool {
    matches!(error.code, ErrorCode::InvalidInput)
        && sciotte_refusal(error) == Some(NO_PENDING_LOGIN)
}
