// ABOUTME: How a failed provider-context read (zones, gear, records, best efforts, weight, search) reaches the platform's error codes
// ABOUTME: A provider refusal that may pass, a 429 or a 5xx at the provider, is retryable and never an internal fault

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The session-read service answers a context read the provider refused with a `500` whose
//! marker is the service's own error text. A `429` or `5xx` the provider
//! answered (`network error: … answered HTTP 429`) is a refusal that may pass a
//! moment later, so it is reported as one: [`ErrorCode::ExternalRateLimited`]
//! for the `429`, [`ErrorCode::ExternalServiceUnavailable`] for the `5xx`. A
//! bare `429` from anything in front of the service is the same rate limit.
//!
//! Everything else on these operations keeps its ordinary mapping
//! ([`super::to_app_error_as`]): a dead session is an auth error, a shed is
//! backpressure, any other `500` is internal. An absent value (a `404`) is no
//! error and never gets here: the caller reads it as `None`.

use dravr_sciotte::client::{ClientError, Operation, StatusCode};
use pierre_core::errors::{AppError, ErrorCode};
use serde_json::Value;
use tracing::warn;

use super::answer_marker;

/// The service's text for a provider answer that may pass (`SessionReadError::Network`).
const NETWORK_MARKER: &str = "network error:";

/// The marker's own words for a provider rate limit.
const RATE_LIMITED_MARKER: &str = "HTTP 429";

/// Whether `operation` is a provider-context read.
const fn is_context_read(operation: Option<Operation>) -> bool {
    matches!(
        operation,
        Some(
            Operation::ActivitySearch
                | Operation::ActivityZones
                | Operation::Gear
                | Operation::PersonalRecords
                | Operation::BestEfforts
                | Operation::AthleteWeight
        )
    )
}

/// The retryable error a context read's answer is, or `None` when it is not
/// one and the ordinary mapping applies.
pub(super) fn retryable(error: &ClientError, brand: &str) -> Option<AppError> {
    if !is_context_read(error.operation()) {
        return None;
    }
    let ClientError::Unexpected { status, body, .. } = error else {
        return None;
    };
    let rate_limited =
        *status == StatusCode::TOO_MANY_REQUESTS || service_relayed_429(*status, body);
    if rate_limited {
        warn!(error = %error, "the provider rate limited a context read");
        return Some(AppError::new(
            ErrorCode::ExternalRateLimited,
            format!("{brand} is rate limiting this read; try again later"),
        ));
    }
    if status.is_server_error() && answer_marker(body).starts_with(NETWORK_MARKER) {
        warn!(error = %error, "the provider failed a context read");
        return Some(AppError::new(
            ErrorCode::ExternalServiceUnavailable,
            format!("{brand} did not answer this read; try again in a moment"),
        ));
    }
    None
}

/// Whether the service relayed a provider `429` as its own `500`.
fn service_relayed_429(status: StatusCode, body: &Value) -> bool {
    status.is_server_error() && {
        let marker = answer_marker(body);
        marker.starts_with(NETWORK_MARKER) && marker.contains(RATE_LIMITED_MARKER)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use dravr_sciotte::client::{Exchange, RequestId};
    use serde_json::json;

    use super::*;

    fn unexpected(operation: Operation, status: StatusCode, body: Value) -> ClientError {
        ClientError::Unexpected {
            exchange: Box::new(Exchange {
                service: "sciotte".to_owned(),
                operation: operation.as_str().to_owned(),
                request_id: RequestId::mint(),
                elapsed: Duration::from_millis(40),
                resent: false,
            }),
            status,
            body,
        }
    }

    #[test]
    fn a_provider_429_the_service_relayed_is_a_rate_limit() {
        let error = unexpected(
            Operation::Gear,
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({"error": "network error: https://example.test/x answered HTTP 429"}),
        );
        let mapped = retryable(&error, "Strava").expect("a retryable answer");
        assert_eq!(
            mapped.code,
            ErrorCode::ExternalRateLimited,
            "bug: a relayed 429 is not a rate limit"
        );
        assert!(
            !mapped.message.contains("example.test"),
            "bug: the provider URL leaked into the message: {mapped}"
        );
    }

    #[test]
    fn a_bare_429_is_a_rate_limit() {
        let error = unexpected(
            Operation::ActivityZones,
            StatusCode::TOO_MANY_REQUESTS,
            Value::Null,
        );
        assert_eq!(
            retryable(&error, "Strava").map(|e| e.code),
            Some(ErrorCode::ExternalRateLimited),
            "bug: a bare 429 is not a rate limit"
        );
    }

    #[test]
    fn a_provider_5xx_the_service_relayed_is_unavailable() {
        let error = unexpected(
            Operation::PersonalRecords,
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({"error": "network error: https://example.test/x answered HTTP 503"}),
        );
        assert_eq!(
            retryable(&error, "Strava").map(|e| e.code),
            Some(ErrorCode::ExternalServiceUnavailable),
            "bug: a relayed 5xx is not a retryable outage"
        );
    }

    #[test]
    fn any_other_failure_keeps_its_ordinary_mapping() {
        let plain_500 = unexpected(
            Operation::AthleteWeight,
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({"error": "session read error: a weight this build cannot read"}),
        );
        assert!(
            retryable(&plain_500, "Strava").is_none(),
            "bug: a service fault read as a retryable provider answer"
        );
        let other_operation = unexpected(
            Operation::Activities,
            StatusCode::TOO_MANY_REQUESTS,
            Value::Null,
        );
        assert!(
            retryable(&other_operation, "Strava").is_none(),
            "bug: a non-context read took the context mapping"
        );
    }
}
