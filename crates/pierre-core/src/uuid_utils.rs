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

/// The numeric id the dev fixture serves a seeded row under: the first 16
/// hex digits of its UUID text, read as one `u64` (`1` when it holds none).
///
/// Strava ids are integers, so the fixture's Strava stand-in hands out this
/// one for a `synthetic_activities` row and for a seeded athlete. A seeder
/// that warms the activity cache with the row must key it by the same id:
/// a later sync through the fixture then rewrites that cache row instead of
/// adding a second copy of the session beside it.
#[must_use]
pub fn fixture_numeric_id(uuid: &str) -> u64 {
    let hex: String = uuid
        .chars()
        .filter(char::is_ascii_hexdigit)
        .take(16)
        .collect();
    u64::from_str_radix(&hex, 16).unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::fixture_numeric_id;

    #[test]
    fn the_fixture_id_is_the_first_sixteen_hex_digits() {
        assert_eq!(
            fixture_numeric_id("0a1b2c3d-4e5f-4a6b-8c7d-9e0f1a2b3c4d"),
            0x0a1b_2c3d_4e5f_4a6b,
            "the first sixteen hex digits, read as one u64"
        );
        assert_eq!(
            fixture_numeric_id("zz-zz"),
            1,
            "a text with no hex digit reads as 1"
        );
    }
}
