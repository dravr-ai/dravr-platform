// ABOUTME: Maps account-status error codes to per-channel messaging-string keys
// ABOUTME: The policy gate itself lives in pierre_auth::user_status::enforce_user_status
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # User status messaging-key mapper
//!
//! Companion to [`pierre_auth::user_status::enforce_user_status`] (which returns
//! the structured [`AppError`] for non-active accounts). This module owns the
//! translation from [`ErrorCode::AccountPending`] / [`ErrorCode::AccountSuspended`]
//! to the contremaitre [`pierre_contremaitre::messaging_strings`] registry keys,
//! so messaging-ingress callers (Telegram, WhatsApp, Discord, Slack, Messenger)
//! can render a localized template instead of the bare HTTP error envelope.
//!
//! Kept in `pierre-server` because the messaging-strings registry itself lives
//! here. HTTP middleware never touches this mapper — it lets the [`AppError`]
//! flow through to the standard error response.

use pierre_core::errors::ErrorCode;

use pierre_contremaitre::messaging_strings::{
    KEY_ACCOUNT_PENDING, KEY_ACCOUNT_SUSPENDED, KEY_NO_PROVIDER_CONNECTED, KEY_QUOTA_EXCEEDED,
    KEY_RATE_LIMITED,
};

/// Resolve the messaging-strings key for an account-status error.
///
/// Returns the translation key that messaging-ingress callers should render
/// through [`pierre_contremaitre::messaging_strings::MessagingStringsRegistry`]
/// when [`pierre_auth::user_status::enforce_user_status`] returns the
/// corresponding [`pierre_core::errors::AppError`]. Returns `None` for error
/// codes unrelated to account status so callers can fall through to the
/// generic-error channel reply.
#[must_use]
pub const fn messaging_key_for_status(code: ErrorCode) -> Option<&'static str> {
    match code {
        ErrorCode::AccountPending => Some(KEY_ACCOUNT_PENDING),
        ErrorCode::AccountSuspended => Some(KEY_ACCOUNT_SUSPENDED),
        ErrorCode::NoProviderConnected => Some(KEY_NO_PROVIDER_CONNECTED),
        ErrorCode::RateLimitExceeded => Some(KEY_RATE_LIMITED),
        ErrorCode::QuotaExceeded => Some(KEY_QUOTA_EXCEEDED),
        _ => None,
    }
}
