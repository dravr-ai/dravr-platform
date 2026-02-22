// ABOUTME: URL redaction utility for safe logging of connection strings
// ABOUTME: Strips credentials from URLs to prevent secret leakage in logs
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/// Redact credentials from a URL for safe logging
///
/// Replaces the `user:password` portion of a URL with `***:***` when
/// credentials are found between the scheme and an `@` symbol.
///
/// # Arguments
///
/// * `url` - URL that may contain embedded credentials
///
/// # Returns
///
/// URL with credentials redacted, showing only host and port
///
/// # Examples
///
/// ```
/// use pierre_core::redaction::redact_url;
///
/// // Redacts credentials from Redis URL
/// assert_eq!(
///     redact_url("redis://user:secret@localhost:6379"),
///     "redis://***:***@localhost:6379"
/// );
///
/// // Leaves URLs without credentials unchanged
/// assert_eq!(
///     redact_url("redis://localhost:6379"),
///     "redis://localhost:6379"
/// );
/// ```
#[must_use]
pub fn redact_url(url: &str) -> String {
    // Match pattern: scheme://user:password@host... or scheme://:password@host...
    if let Some(at_pos) = url.find('@') {
        if let Some(scheme_end) = url.find("://") {
            let credentials_start = scheme_end + 3;
            if at_pos > credentials_start {
                return format!(
                    "{}***:***@{}",
                    &url[..credentials_start],
                    &url[at_pos + 1..]
                );
            }
        }
    }
    url.to_owned()
}
