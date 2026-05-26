// ABOUTME: Account-status authorization gate — Pending/Suspended blocked, Active passes
// ABOUTME: Shared by HTTP/JWT middleware and messaging ingress so policy lives in one place
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # User status gate
//!
//! Centralizes the [`UserStatus::Pending`] / [`UserStatus::Suspended`] checks
//! that authenticated callers need to pass before any user-action runs. Without
//! a shared gate, an unapproved account could chat freely via a messaging
//! channel (which bypasses JWT middleware) while being blocked on the HTTP API.
//!
//! Callers map the returned [`AppError`] to their transport's reply: HTTP
//! middleware bubbles it up to the standard error envelope, messaging ingress
//! looks the error code up via a transport-specific string registry. Keeping
//! the policy in one function means flipping `Pending → Active` in the admin
//! console takes effect instantly across every transport.
//!
//! ## Behavior
//!
//! - [`UserStatus::Active`] — `Ok(())`, request proceeds.
//! - [`UserStatus::Pending`] — [`AppError::account_pending`] (`ErrorCode::AccountPending`).
//! - [`UserStatus::Suspended`] — [`AppError::account_suspended`] (`ErrorCode::AccountSuspended`).

use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::UserStatus;

/// Enforce the global account-status policy.
///
/// Returns `Ok(())` for active accounts and a structured [`AppError`] for
/// pending or suspended accounts so callers can surface a transport-appropriate
/// response (HTTP error envelope, translated messaging string, …).
///
/// # Errors
///
/// - [`AppError::account_pending`] — account is awaiting admin approval.
/// - [`AppError::account_suspended`] — account has been suspended.
pub fn enforce_user_status(status: UserStatus) -> AppResult<()> {
    match status {
        UserStatus::Active => Ok(()),
        UserStatus::Pending => Err(AppError::account_pending(
            "Your account is pending admin approval",
        )),
        UserStatus::Suspended => Err(AppError::account_suspended(
            "Your account has been suspended",
        )),
    }
}
