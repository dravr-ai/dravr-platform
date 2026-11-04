// ABOUTME: Core types for universal protocol system
// ABOUTME: Request, response, and executor types used across the universal protocol
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::protocol::executor::UniversalExecutor;
use crate::protocols::ProtocolError;
use pierre_core::errors::AppError;
use pierre_providers::sciotte_error::{coach_account_error, COACH_ACCOUNT_REFUSAL};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::fmt::Debug;

/// Universal request structure for protocol-agnostic tool execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UniversalRequest {
    /// Name of the tool to execute
    pub tool_name: String,
    /// Tool-specific parameters as JSON
    pub parameters: Value,
    /// User ID making the request
    pub user_id: String,
    /// Protocol identifier (e.g., "mcp", "a2a")
    pub protocol: String,
    /// Optional tenant ID for multi-tenant isolation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tenant_id: Option<String>,
}

/// Universal response structure for tool execution results
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UniversalResponse {
    /// Whether the tool execution succeeded
    pub success: bool,
    /// Tool execution result as JSON
    pub result: Option<Value>,
    /// Error message if execution failed
    pub error: Option<String>,
    /// Additional metadata about the execution
    pub metadata: Option<HashMap<String, Value>>,
}

/// Metadata key set on a tool's `UniversalResponse` when the underlying
/// `AppError` was `ProviderAuthRequired`. The tool loop scans this key to
/// know it should exit early without continuing iteration.
///
/// Lives here (rather than in the `client-chat`-gated `tool_execution`
/// module) so it can be shared with always-compiled callers such as
/// [`provider_helpers`](super::provider_helpers).
pub const META_AUTH_REQUIRED_PROVIDER: &str = "auth_required_provider";

/// Metadata key set (to `true`) on the refusal a tool returns when the athlete
/// has no fitness provider connected at all.
///
/// Distinct from [`META_AUTH_REQUIRED_PROVIDER`] on purpose: an account that
/// never connected anything has no connection to RE-connect, and the re-auth
/// signal replaces the turn with "your connection expired". This key does not
/// stop the tool loop: the model reads the refusal text and answers what it
/// can, with a nudge to connect.
pub const META_NO_PROVIDER_CONNECTED: &str = "no_provider_connected";

/// Whether a failed response is the no-provider-connected refusal.
#[must_use]
pub fn is_no_provider_refusal(response: &UniversalResponse) -> bool {
    response
        .metadata
        .as_ref()
        .and_then(|m| m.get(META_NO_PROVIDER_CONNECTED))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// Metadata key set (to `true`) on the refusal of a read of a coach account's
/// own calendar, on a platform whose coach accounts keep none.
///
/// The refusal is answered before any read, once the account is recorded as a
/// coach's, so its words alone would reach the caller as a plain provider
/// failure. The key keeps it typed: it is the account's standing state, not
/// an outage, and signing in again leaves a coach account a coach account, so
/// it is never the re-auth signal either.
pub(crate) const META_OWN_CALENDAR_REFUSED: &str = "own_calendar_refused";

/// Whether a failed response is the refusal of a coach account's own calendar
/// ([`META_OWN_CALENDAR_REFUSED`]).
#[must_use]
pub(crate) fn is_own_calendar_refusal(response: &UniversalResponse) -> bool {
    response
        .metadata
        .as_ref()
        .and_then(|m| m.get(META_OWN_CALENDAR_REFUSED))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// The provider slug a failed response says the athlete must reconnect, if any.
///
/// Every caller that re-raises the typed auth error, drops a cached page, or
/// decides a recovery turn reads [`META_AUTH_REQUIRED_PROVIDER`] the same way:
/// the key must be present *and* hold a string. Reading it by hand once
/// admitted a non-string value as a reconnect signal, so the extraction lives
/// here beside the key it reads.
#[must_use]
pub fn auth_required_provider(response: &UniversalResponse) -> Option<String> {
    response
        .metadata
        .as_ref()?
        .get(META_AUTH_REQUIRED_PROVIDER)?
        .as_str()
        .map(ToOwned::to_owned)
}

/// Type a failed `create_authenticated_provider`, preserving the auth shape.
///
/// That call tags a non-recoverably dead connection with
/// the auth-required provider slug — the one signal that says the athlete must
/// reconnect. It tags the refusal of a coach account's own calendar
/// ([`is_own_calendar_refusal`]) too, which becomes the same coach-account
/// error the scraper's refusal maps to ([`coach_account_error`]): the
/// account's standing state, judged the same whichever step refused it.
/// Everything else it can fail with (an unsupported provider, a tenant
/// missing OAuth credentials, a transport blip) is not a dead session and
/// must never flag a connection, so it degrades to a plain external-service
/// error the sweep treats as transient.
///
/// The auth error names the backend the tag names, the connection that
/// failed, not `provider_slug`: a read asked for `garmin` and served by the
/// athlete's username-and-password session (`sciotte_garmin`) names that
/// session, so the flag, the notice and the reconnect link reach it rather
/// than the Garmin grant beside it, which was fine.
pub(crate) fn provider_auth_failure(provider_slug: &str, response: &UniversalResponse) -> AppError {
    if let Some(failed) = auth_required_provider(response) {
        return AppError::provider_auth_required(failed);
    }
    if is_own_calendar_refusal(response) {
        return coach_account_error(
            response
                .error
                .clone()
                .unwrap_or_else(|| COACH_ACCOUNT_REFUSAL.to_owned()),
        );
    }
    AppError::external_service(
        provider_slug,
        response
            .error
            .clone()
            .unwrap_or_else(|| "provider authentication failed".to_owned()),
    )
}

/// Universal tool definition with handler function
#[derive(Debug, Clone)]
pub struct UniversalTool {
    /// Tool name identifier
    pub name: String,
    /// Human-readable tool description
    pub description: String,
    /// Handler function for tool execution
    pub handler:
        fn(&UniversalToolExecutor, UniversalRequest) -> Result<UniversalResponse, ProtocolError>,
}

/// Type alias for `UniversalExecutor` used in tool handler signatures
pub type UniversalToolExecutor = UniversalExecutor;

#[cfg(test)]
mod tests {
    use pierre_core::errors::ErrorCode;
    use pierre_providers::sciotte_error::is_coach_account_refusal;

    use super::*;

    fn refused(metadata: Option<HashMap<String, Value>>) -> UniversalResponse {
        UniversalResponse {
            success: false,
            result: None,
            error: Some("This TrainingPeaks account is a coach account.".to_owned()),
            metadata,
        }
    }

    #[test]
    fn a_refused_own_calendar_keeps_its_coach_account_marker() {
        let error = provider_auth_failure(
            "sciotte_trainingpeaks",
            &refused(Some(HashMap::from([(
                META_OWN_CALENDAR_REFUSED.to_owned(),
                Value::Bool(true),
            )]))),
        );
        assert!(
            is_coach_account_refusal(&error),
            "bug: the marker was lost: {error:?}"
        );
        assert_eq!(
            error.code,
            ErrorCode::InvalidInput,
            "bug: the refusal must keep the coach-account code"
        );
        assert_eq!(
            error.message, "This TrainingPeaks account is a coach account.",
            "bug: the refusal must keep its own words"
        );
        assert_eq!(
            error.provider_auth_required_provider(),
            None,
            "bug: a coach account's refusal is never the re-auth signal"
        );
    }

    #[test]
    fn an_auth_failure_names_the_backend_that_failed_not_the_one_asked_for() {
        let response = UniversalResponse {
            success: false,
            result: None,
            error: Some("No valid garmin token found.".to_owned()),
            metadata: Some(HashMap::from([(
                META_AUTH_REQUIRED_PROVIDER.to_owned(),
                Value::String("sciotte_garmin".to_owned()),
            )])),
        };
        let error = provider_auth_failure("garmin", &response);
        assert_eq!(
            error.provider_auth_required_provider().as_deref(),
            Some("sciotte_garmin"),
            "bug: the auth error must name the session that failed, not the Garmin grant"
        );
    }

    #[test]
    fn an_untagged_refusal_stays_a_plain_external_service_error() {
        let error = provider_auth_failure("sciotte_trainingpeaks", &refused(None));
        assert!(
            !is_coach_account_refusal(&error),
            "bug: an untagged failure must not read as a coach-account refusal: {error:?}"
        );
        assert_eq!(
            error.code,
            ErrorCode::ExternalServiceError,
            "bug: an untagged failure must degrade to a plain external-service error"
        );
    }
}
