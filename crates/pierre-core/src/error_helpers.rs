// ABOUTME: Standardized error handling utilities for consistent error management
// ABOUTME: Provides helper functions and patterns for creating and handling errors consistently
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::any::Any;

use crate::errors::{AppError, ErrorCode};

/// Extract a human-readable message from a caught panic payload.
///
/// `std` panics carry either a `&'static str` or a `String`; anything else is
/// reported generically so the caller's log still fires with a correlation id
/// rather than swallowing the failure.
///
/// Shared by every `catch_unwind` boundary in the workspace — the turn-level
/// one in messaging dispatch and the stage-level one around claim
/// verification — so a caught panic reads the same way in the logs wherever
/// it was contained.
#[must_use]
pub fn panic_payload_str(payload: &(dyn Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown panic payload".to_owned())
}

/// Create a validation error with context
#[must_use]
pub fn validation_error(message: &str) -> AppError {
    AppError::new(
        ErrorCode::InvalidInput,
        format!("Validation failed: {message}"),
    )
}

/// Create a user state error with context
#[must_use]
pub fn user_state_error(message: &str) -> AppError {
    AppError::new(
        ErrorCode::PermissionDenied,
        format!("User state error: {message}"),
    )
}
