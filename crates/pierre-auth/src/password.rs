// ABOUTME: Checks a sign-in password against a stored bcrypt hash, off the async executor
// ABOUTME: A federated-only account matches no password; an unreadable stored hash is a server fault
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::FEDERATED_ONLY_PASSWORD_HASH;
use tokio::task::spawn_blocking;
use tracing::error;

/// Whether `password` matches the stored `hash`.
///
/// bcrypt is CPU-bound, so it runs on a blocking thread. A federated-only
/// account's [`FEDERATED_ONLY_PASSWORD_HASH`] matches no password and is
/// answered `false` before bcrypt, which would refuse the marker as malformed:
/// a password tried on a Google or Apple account is the caller's mistake.
///
/// # Errors
/// Returns an internal error, already logged at ERROR, when the blocking task
/// fails or bcrypt cannot read the stored hash: a corrupt row, not a wrong
/// password. bcrypt's own message quotes the hash, so it is never logged.
pub async fn verify_password(password: String, hash: String) -> AppResult<bool> {
    if hash == FEDERATED_ONLY_PASSWORD_HASH {
        return Ok(false);
    }
    spawn_blocking(move || bcrypt::verify(&password, &hash))
        .await
        .map_err(|e| {
            error!("Password verification task failed: {e}");
            AppError::internal("Password verification failed")
        })?
        .map_err(|_| {
            error!("bcrypt could not read a stored password hash");
            AppError::internal("Password verification failed")
        })
}
