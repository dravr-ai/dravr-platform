// ABOUTME: UUID parsing and validation utilities for consistent error handling across the platform
// ABOUTME: Provides safe UUID parsing, formatting, and generation functions used by database and API layers
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::errors::protocol::ProtocolError;
use crate::errors::{AppError, AppResult};
use uuid::Uuid;

/// Parse a UUID from a string with consistent error handling
///
/// # Errors
///
/// Returns an error if the string is not a valid UUID format
pub fn parse_uuid(uuid_str: &str) -> AppResult<Uuid> {
    Uuid::parse_str(uuid_str)
        .map_err(|e| AppError::invalid_input(format!("Invalid UUID format '{uuid_str}': {e}")))
}

/// Parse a user ID for protocol requests with `ProtocolError`
///
/// # Errors
///
/// Returns a `ProtocolError::InvalidParameters` if the user ID is not a valid UUID
pub fn parse_user_id_for_protocol(user_id_str: &str) -> Result<Uuid, ProtocolError> {
    Uuid::parse_str(user_id_str)
        .map_err(|_| ProtocolError::InvalidParameters("Invalid user ID format".into()))
}
