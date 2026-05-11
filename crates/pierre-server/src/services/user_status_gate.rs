// ABOUTME: Single source of truth for UserStatus authorization — used by both HTTP/JWT middleware and messaging ingress
// ABOUTME: One enforce function returns AppError::account_pending / account_suspended; callers map to transport-appropriate replies
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # User status gate
//!
//! Centralizes the `UserStatus::Pending` / `UserStatus::Suspended` checks that
//! previously lived inline in [`crate::middleware::auth`]. Messaging ingress
//! (Telegram, `WhatsApp`, Discord, Slack, Messenger) does not flow through that
//! middleware — channel-link lookups bypass JWT validation entirely — so
//! without this shared gate, an unapproved account could chat freely via any
//! messaging channel while being blocked on the HTTP API.
//!
//! Both code paths now call [`enforce_user_status`] after resolving the user:
//! HTTP middleware bubbles the [`AppError`] up to the standard error response,
//! messaging ingress maps it to a localized template via
//! [`messaging_key_for_status`] and renders it through the strings registry.
//!
//! Keeping the policy in one function means flipping `Pending → Active` in the
//! admin console takes effect instantly across every transport without a
//! second deploy.
//!
//! ## Behavior
//!
//! - [`UserStatus::Active`] — `Ok(())`, request proceeds.
//! - [`UserStatus::Pending`] — [`AppError::account_pending`] (`ErrorCode::AccountPending`).
//! - [`UserStatus::Suspended`] — [`AppError::account_suspended`] (`ErrorCode::AccountSuspended`).

use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::UserStatus;

use crate::contremaitre::messaging_strings::{KEY_ACCOUNT_PENDING, KEY_ACCOUNT_SUSPENDED};

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

/// Resolve the messaging-strings key for an account-status error.
///
/// Returns the translation key that messaging-ingress callers should render
/// through [`crate::contremaitre::messaging_strings::MessagingStringsRegistry`]
/// when [`enforce_user_status`] returns the corresponding [`AppError`].
/// Returns `None` for error codes unrelated to account status so callers can
/// fall through to the generic-error channel reply.
#[must_use]
pub const fn messaging_key_for_status(code: ErrorCode) -> Option<&'static str> {
    match code {
        ErrorCode::AccountPending => Some(KEY_ACCOUNT_PENDING),
        ErrorCode::AccountSuspended => Some(KEY_ACCOUNT_SUSPENDED),
        _ => None,
    }
}
