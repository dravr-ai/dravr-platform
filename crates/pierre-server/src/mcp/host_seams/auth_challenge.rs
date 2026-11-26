// ABOUTME: Bearer challenges and transport refusals for MCP authentication failures
// ABOUTME: Builds RFC 9728 / RFC 6750 WWW-Authenticate values and maps an auth AppError to its AuthError
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use dravr_tronc::mcp::auth::AuthError;
use pierre_auth::config::{protected_resource_metadata_url, www_authenticate_challenge};
use pierre_auth::dpop::dpop_challenge;
use pierre_core::errors::ErrorCode;
use pierre_core::permissions::scopes::OAuthScope;
use pierre_middleware::auth::AuthRefusal;
use tracing::{debug, error};

/// Build the RFC 6750 §3.1 `insufficient_scope` challenge naming the grant the
/// caller is missing.
///
/// The `scope` parameter is the whole point of this error code: a client that
/// reads it knows exactly which grant to re-request and can recover, where a
/// bare 403 tells it only that it lost. `resource_metadata` rides along so the
/// client also knows *which* authorization server to ask.
pub(super) fn insufficient_scope_challenge(resource_url: &str, missing: OAuthScope) -> String {
    let metadata_url = protected_resource_metadata_url(resource_url);
    format!(
        "Bearer resource_metadata=\"{metadata_url}\", error=\"insufficient_scope\", scope=\"{missing}\""
    )
}

/// The `insufficient_scope` challenge of a call a step-up would cure (owl §7.6).
///
/// Its `scope` is the step-up scope
/// ([`pierre_auth::oauth2_server::step_up_scope::challenge_scope`]), the
/// token's own grant and the one-off question over `streams`, which a host
/// following MCP's step-up flow re-authorizes with.
pub(super) fn step_up_challenge(resource_url: &str, scope: &str) -> String {
    let metadata_url = protected_resource_metadata_url(resource_url);
    format!(
        "Bearer resource_metadata=\"{metadata_url}\", error=\"insufficient_scope\", scope=\"{scope}\""
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
/// again. Mirrors `AppError::into_auth_refusal` on the REST routes.
///
/// A refusal of the `DPoP` binding rules adds the `DPoP` challenge naming its
/// code and the accepted algorithms beside the Bearer one (RFC 9449 §7.1).
pub(super) fn auth_refusal(refusal: &AuthRefusal, resource_url: &str) -> AuthError {
    let error = &refusal.error;
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
    let bearer = www_authenticate_challenge(resource_url, Some("invalid_token"));
    let www_authenticate = refusal.dpop_error.map_or_else(
        || bearer.clone(),
        |code| format!("{}, {bearer}", dpop_challenge(Some(code))),
    );
    AuthError::Unauthorized { www_authenticate }
}
