// ABOUTME: Standardized error handling utilities for consistent error management
// ABOUTME: Provides helper functions and patterns for creating and handling errors consistently
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::errors::{AppError, ErrorCode};

/// Create a validation error with context
#[must_use]
pub fn validation_error(message: &str) -> AppError {
    AppError::new(
        ErrorCode::InvalidInput,
        format!("Validation failed: {message}"),
    )
}

/// Create an authentication error with context
#[must_use]
pub fn auth_error(message: &str) -> AppError {
    AppError::new(
        ErrorCode::AuthInvalid,
        format!("Authentication failed: {message}"),
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
