// ABOUTME: The status strings a user listing may ask for, and the one that selects every account
// ABOUTME: Split from users.rs so it stays under the size ceiling; the repository body there validates with them
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::{AppError, AppResult};

/// The status strings a listing may ask for.
///
/// # Errors
/// Returns an invalid-input error for any other string.
pub(crate) fn user_status_filter(status: &str) -> AppResult<&str> {
    match status {
        "active" | "pending" | "suspended" => Ok(status),
        _ => Err(AppError::invalid_input(format!(
            "Invalid user status: {status}"
        ))),
    }
}

/// The listing status that selects every account, whatever its status. The
/// paged statements of `user_sql` spell the same literal.
pub const ALL_USER_STATUSES: &str = "all";

/// The status strings a paged listing may ask for: one status, or
/// [`ALL_USER_STATUSES`].
///
/// # Errors
/// Returns an invalid-input error for any other string.
pub(crate) fn listing_status_filter(status: &str) -> AppResult<&str> {
    if status == ALL_USER_STATUSES {
        return Ok(status);
    }
    user_status_filter(status)
}
