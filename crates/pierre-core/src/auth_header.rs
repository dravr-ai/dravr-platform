// ABOUTME: Generic authentication utilities for bearer token extraction and validation
// ABOUTME: Eliminates duplication in Authorization header parsing across routes and middleware
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::constants::key_prefixes;
use crate::errors::{AppError, AppResult};

/// Extract bearer token from Authorization header string
///
/// # Errors
///
/// Returns an error if:
/// - Authorization header doesn't start with "Bearer "
/// - Token is empty after extraction and trimming
/// - Header format is invalid
pub fn extract_bearer_token(auth_header: &str) -> AppResult<&str> {
    if !auth_header.starts_with("Bearer ") {
        return Err(AppError::auth_invalid(
            "Invalid authorization header format",
        ));
    }

    let token = auth_header
        .strip_prefix("Bearer ")
        .ok_or_else(|| AppError::auth_invalid("Failed to extract bearer token"))?
        .trim();

    if token.is_empty() {
        return Err(AppError::auth_invalid("Empty bearer token"));
    }

    Ok(token)
}

/// Extract bearer token and return it as owned String
///
/// # Errors
///
/// Returns an error if:
/// - Authorization header doesn't start with "Bearer "  
/// - Token is empty after extraction and trimming
/// - Header format is invalid
pub fn extract_bearer_token_owned(auth_header: &str) -> AppResult<String> {
    extract_bearer_token(auth_header).map(str::to_owned)
}

/// Extract bearer token from optional Authorization header
///
/// # Errors
///
/// Returns an error if:
/// - Authorization header is missing (None)
/// - Header format is invalid
/// - Token is empty
pub fn extract_bearer_token_from_option(auth_header: Option<&str>) -> AppResult<&str> {
    let header = auth_header.ok_or_else(AppError::auth_required)?;
    extract_bearer_token(header)
}

/// Extract bearer token from optional Authorization header as owned String
///
/// # Errors
///
/// Returns an error if:
/// - Authorization header is missing (None)
/// - Header format is invalid  
/// - Token is empty
pub fn extract_bearer_token_from_option_owned(auth_header: Option<&str>) -> AppResult<String> {
    extract_bearer_token_from_option(auth_header).map(str::to_owned)
}

/// Whether an Authorization header value, or the bearer token a transport
/// stripped from one, is an API key this server issues (`pk_live_` or
/// `pk_trial_`).
///
/// The one classifier REST authentication and the `/mcp` transport share, so
/// both accept exactly the same key formats.
#[must_use]
pub fn is_api_key_format(auth_header: &str) -> bool {
    auth_header.starts_with(key_prefixes::LIVE) || auth_header.starts_with(key_prefixes::TRIAL)
}
