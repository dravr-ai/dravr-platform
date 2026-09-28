// ABOUTME: Bearer challenges and transport refusals for MCP authentication failures
// ABOUTME: Builds RFC 9728 / RFC 6750 WWW-Authenticate values and maps an auth AppError to its AuthError
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use dravr_tronc::mcp::auth::AuthError;
use pierre_core::errors::{AppError, ErrorCode};
use pierre_core::permissions::scopes::OAuthScope;
use tracing::{debug, error};

/// The RFC 9728 protected resource metadata document for the resource
/// `resource_url`: the well-known path on the origin clients dial for MCP.
fn resource_metadata_url(resource_url: &str) -> String {
    format!("{resource_url}/.well-known/oauth-protected-resource")
}

/// Build the RFC 9728 `WWW-Authenticate` challenge value pointing clients at the
/// protected resource metadata document of `resource_url`, the MCP resource
/// identifier (`MCP_RESOURCE_URL`). `error` carries an optional RFC 6750 error
/// code (e.g. `invalid_token`).
pub(super) fn www_authenticate_challenge(resource_url: &str, error: Option<&str>) -> String {
    let metadata_url = resource_metadata_url(resource_url);
    error.map_or_else(
        || format!("Bearer resource_metadata=\"{metadata_url}\""),
        |err| format!("Bearer resource_metadata=\"{metadata_url}\", error=\"{err}\""),
    )
}

/// Build the RFC 6750 §3.1 `insufficient_scope` challenge naming the grant the
/// caller is missing.
///
/// The `scope` parameter is the whole point of this error code: a client that
/// reads it knows exactly which grant to re-request and can recover, where a
/// bare 403 tells it only that it lost. `resource_metadata` rides along so the
/// client also knows *which* authorization server to ask.
pub(super) fn insufficient_scope_challenge(resource_url: &str, missing: OAuthScope) -> String {
    let metadata_url = resource_metadata_url(resource_url);
    format!(
        "Bearer resource_metadata=\"{metadata_url}\", error=\"insufficient_scope\", scope=\"{missing}\""
    )
}

/// The transport refusal for an authentication failure.
///
/// Only a refused credential is a 401 `invalid_token`, which tells an `OAuth`
/// client its token is dead and sends it to refresh and re-authorize. A spent
/// request budget is a 429 carrying the refusal's own retry window, and a
/// server-side failure (a database error reading the user or the usage
/// counter) is a 500: neither says anything about the credential, and a 401
/// for either loops the client through re-authorization only to be refused
/// again. Mirrors [`AppError::into_auth_refusal`] on the REST routes.
pub(super) fn auth_refusal(error: &AppError, resource_url: &str) -> AuthError {
    if error.code == ErrorCode::RateLimitExceeded {
        debug!(error = %error, "MCP request refused: the credential's request budget is spent");
        return AuthError::RateLimited {
            retry_after_secs: error.retry_after_secs().unwrap_or(1),
            reason: error.sanitized_message(),
        };
    }
    if error.is_server_fault() {
        error!(error = %error, "MCP request failed: authentication could not complete");
        return AuthError::Internal {
            reason: error.sanitized_message(),
        };
    }
    debug!(error = %error, "MCP request rejected: bearer token failed validation");
    AuthError::Unauthorized {
        www_authenticate: www_authenticate_challenge(resource_url, Some("invalid_token")),
    }
}
