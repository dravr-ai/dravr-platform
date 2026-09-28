// ABOUTME: Checks on the text an OAuth 2.0 client sends before any of it reaches a query
// ABOUTME: Refuses control characters and an oversized state as the caller's mistake, never as a database failure
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::constants::oauth2_authorization::MAX_STATE_BYTES;
use tracing::warn;

use super::models::OAuth2Error;

/// Refuse a request parameter carrying a control character.
///
/// Every value a client sends here is printable: RFC 6749 Appendix A defines
/// `state` and `code` as visible ASCII, RFC 7636 the PKCE values as unreserved
/// characters. `PostgreSQL` rejects a NUL byte in a text parameter, so one let
/// through to a query would report the caller's malformed request as a
/// database failure, and page the operators for it.
pub(super) fn refuse_control_characters(values: &[Option<&str>]) -> Result<(), OAuth2Error> {
    if values
        .iter()
        .flatten()
        .any(|value| value.chars().any(char::is_control))
    {
        warn!("OAuth2 request refused: a parameter carries a control character");
        return Err(OAuth2Error::invalid_request(
            "Request parameters must not contain control characters",
        ));
    }
    Ok(())
}

/// Refuse a `state` longer than [`MAX_STATE_BYTES`].
///
/// The server stores `state` as the primary key of `oauth2_states`, and a
/// `PostgreSQL` index entry has a size limit: past it the insert fails, which
/// would read as a database failure rather than the caller's oversized value.
pub(super) fn refuse_oversized_state(state: Option<&str>) -> Result<(), OAuth2Error> {
    if state.is_some_and(|s| s.len() > MAX_STATE_BYTES) {
        warn!("OAuth2 request refused: state exceeds {MAX_STATE_BYTES} bytes");
        return Err(OAuth2Error::invalid_request(&format!(
            "state must not exceed {MAX_STATE_BYTES} bytes"
        )));
    }
    Ok(())
}
