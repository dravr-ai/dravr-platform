// ABOUTME: The one mapping from the sciotte client's ClientError to the platform's AppError, by variant and operation
// ABOUTME: Holds the details keys callers branch on: the athlete refusal marker and a load-shed's retry window
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! What a failed call to the sciotte scraper service is, in the platform's
//! error vocabulary.
//!
//! The scraper service is called through `dravr-sciotte`'s own client, whose
//! [`ClientError`] names what the service answered. [`to_app_error`] is the
//! only place that names it in the platform's [`ErrorCode`]s:
//!
//! - a dead or missing session is [`ErrorCode::ProviderAuthRequired`], which
//!   the chat pipeline's auth recovery turns into a reconnect link. The
//!   provider slug is the generic `sciotte`; the provider layer re-tags it
//!   with the backend that owns the session;
//! - a load-shed is [`ErrorCode::ResourceUnavailable`] carrying the service's
//!   own wait, read back by [`shed_retry_after_secs`];
//! - a request the service or its gateway could not finish, a service that
//!   could not be reached, and a session lost with its instance are
//!   [`ErrorCode::ExternalServiceUnavailable`]: the same request can succeed
//!   a moment later, and none of them is the athlete's doing;
//! - the two athlete refusals keep their own codes and carry their marker
//!   under the details key [`sciotte_refusal`] reads. Neither is an auth
//!   error: signing in again changes nothing about which account the athlete
//!   holds or whose roster an athlete is on;
//! - everything else is an internal fault.
//!
//! Nothing here logs, and no message carries a response body.

use dravr_sciotte::client::{ClientError, Operation, ServiceError, TransportFailure};
use dravr_sciotte::wire::{ATHLETE_NOT_ACCESSIBLE, ATHLETE_REQUIRED};
use pierre_core::errors::{AppError, ErrorCode};
use serde_json::{Map, Value};

/// The provider slug an auth-shaped sciotte error names. The provider layer
/// replaces it with the backend that owns the session.
const AUTH_PROVIDER: &str = "sciotte";

/// Key of the [`AppError::details`] entry naming which scraper refusal an
/// error carries, read back by [`sciotte_refusal`].
const SCIOTTE_REFUSAL_DETAIL: &str = "sciotte_refusal";

/// The reason a shed that names none is reported under.
const UNNAMED_SHED_REASON: &str = "scraper_busy";

/// What a login answer that names no marker is reported as carrying.
const NO_MARKER: &str = "none";

/// The platform error for a failed call to the sciotte scraper service.
///
/// Both matches are exhaustive on purpose: a variant added to the client
/// fails this build until it is given a code here.
#[must_use]
pub fn to_app_error(error: ClientError) -> AppError {
    let operation = error.operation();
    match error {
        ClientError::NotConfigured => AppError::internal(
            "DRAVR_SCIOTTE_REMOTE_URL must be set (with DRAVR_SCIOTTE_AUDIENCE \
             for a non-loopback service) — the in-process sciotte scrape path \
             was removed in the ADR-021 Phase 4 cutover",
        ),
        ClientError::Config(config) => AppError::internal(config.to_string()),
        ClientError::Service(service) => service_error(service, operation),
        ClientError::SessionNotFound { .. } | ClientError::SessionExpired { .. } => {
            AppError::provider_auth_required(AUTH_PROVIDER)
        }
        lost @ ClientError::SessionLostOnResend { .. } => {
            AppError::new(ErrorCode::ExternalServiceUnavailable, lost.to_string())
        }
        ClientError::AthleteRequired { .. } => with_refusal_marker(
            AppError::invalid_input(
                "This account is a coach account, and a coach account has no training \
                 calendar of its own. To read an athlete's workouts, link each athlete from a \
                 group you coach; the athlete confirms the link.",
            ),
            ATHLETE_REQUIRED,
        ),
        ClientError::AthleteNotAccessible { .. } => athlete_not_accessible(),
        refused @ ClientError::QueryRefused { .. } => AppError::internal(refused.to_string()),
        mismatch @ ClientError::CountMismatch { .. } => AppError::internal(mismatch.to_string()),
        // LIMITATION(registre#746): the login-step `ClientError::Unexpected` arm also receives a
        // login that ran out its budget, which the service answers `401 session_expired`.
        ClientError::Unexpected {
            exchange,
            status,
            body,
        } if is_login_step(operation) => AppError::internal(format!(
            "sciotte returned HTTP {status} with no recognizable login status \
             (x-request-id {}, marker: {}); check DRAVR_SCIOTTE_AUDIENCE / IAM / \
             service health",
            exchange.request_id,
            answer_marker(&body)
        )),
        unexpected @ ClientError::Unexpected { .. } => AppError::internal(unexpected.to_string()),
    }
}

/// The platform error for one of dravr-tronc's own outcomes.
///
/// A shed of a session transfer is an internal fault, not backpressure: the
/// export that follows a login and the import that precedes a read are the
/// platform's own steps, and a caller that waited and retried would repeat the
/// login or the read around them, not the transfer.
fn service_error(error: ServiceError, operation: Option<Operation>) -> AppError {
    let said = error.to_string();
    match error {
        ServiceError::Identity { .. } => AppError::new(ErrorCode::ExternalServiceError, said),
        ServiceError::Transport { failure, .. } => match failure {
            TransportFailure::TimedOut
            | TransportFailure::Unreachable
            | TransportFailure::ClosedBeforeResponse => {
                AppError::new(ErrorCode::ExternalServiceUnavailable, said)
            }
            TransportFailure::Other => AppError::internal(said),
        },
        ServiceError::Shed {
            exchange, status, ..
        } if is_session_transfer(operation) => AppError::internal(format!(
            "sciotte {} returned {status} (x-request-id {})",
            exchange.operation, exchange.request_id
        )),
        ServiceError::Shed {
            retry_after_secs,
            reason,
            ..
        } => AppError::resource_unavailable(format!(
            "sciotte shed the request ({}); retry after {retry_after_secs}s",
            reason.as_deref().unwrap_or(UNNAMED_SHED_REASON)
        ))
        .with_retry_after(retry_after_secs),
        ServiceError::Unfinished { .. } => {
            AppError::new(ErrorCode::ExternalServiceUnavailable, said)
        }
        ServiceError::Body { .. } | ServiceError::Decode { .. } => AppError::internal(said),
    }
}

/// Whether `operation` is one of the three steps of an interactive login.
const fn is_login_step(operation: Option<Operation>) -> bool {
    matches!(
        operation,
        Some(Operation::Login | Operation::SubmitOtp | Operation::Select2fa)
    )
}

/// Whether `operation` moves a session between the platform and the service.
const fn is_session_transfer(operation: Option<Operation>) -> bool {
    matches!(operation, Some(Operation::Export | Operation::Import))
}

/// The marker a login answer with no login status names: the service's own
/// `error` string, or the `type` of the request guard's and the identity-token
/// gate's nested error.
///
/// The marker tells a rejected identity token from an unknown provider or an
/// unknown flow, which is what an operator reads the failure for. The rest of
/// the body is left out: it is logged and sent in a business event.
fn answer_marker(body: &Value) -> &str {
    let error = body.get("error");
    error
        .and_then(Value::as_str)
        .or_else(|| error?.get("type")?.as_str())
        .unwrap_or(NO_MARKER)
}

/// The refusal of an athlete the session may not read.
///
/// It carries the [`ATHLETE_NOT_ACCESSIBLE`] marker: the scraper's `403` maps
/// to it, and a delegated read answers it when the coach's roster no longer
/// lists the athlete it names.
#[must_use]
pub(crate) fn athlete_not_accessible() -> AppError {
    with_refusal_marker(
        AppError::new(
            ErrorCode::PermissionDenied,
            "That athlete is not on this coach account's roster",
        ),
        ATHLETE_NOT_ACCESSIBLE,
    )
}

/// `error` with `marker` recorded under the refusal details key.
fn with_refusal_marker(mut error: AppError, marker: &str) -> AppError {
    let mut details = Map::new();
    details.insert(
        SCIOTTE_REFUSAL_DETAIL.to_owned(),
        Value::String(marker.to_owned()),
    );
    error.details = Some(Box::new(Value::Object(details)));
    error
}

/// The scraper refusal `error` carries ([`ATHLETE_REQUIRED`],
/// [`ATHLETE_NOT_ACCESSIBLE`]), or `None` for any other error.
#[must_use]
pub fn sciotte_refusal(error: &AppError) -> Option<&str> {
    error
        .details
        .as_ref()?
        .get(SCIOTTE_REFUSAL_DETAIL)?
        .as_str()
}

/// The wait window a load-shed advertises, or `None` when `error` is anything
/// else.
///
/// Callers branch on this to answer a shed with the service's own
/// `Retry-After` instead of routing it through the system-failure path.
#[must_use]
pub fn shed_retry_after_secs(error: &AppError) -> Option<u64> {
    if !matches!(error.code, ErrorCode::ResourceUnavailable) {
        return None;
    }
    error.retry_after_secs()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use dravr_sciotte::client::{ConfigError, Exchange, RequestId, StatusCode, Unfinished};
    use dravr_tronc::iam::IamError;
    use serde_json::json;

    use super::*;

    /// The attempt `exchange` describes, as a hand-built message names it.
    fn named(exchange: &Exchange) -> String {
        format!("x-request-id {}", exchange.request_id)
    }

    fn exchange(operation: Operation) -> Exchange {
        Exchange {
            service: "sciotte".to_owned(),
            operation: operation.as_str().to_owned(),
            request_id: RequestId::mint(),
            elapsed: Duration::from_millis(42),
            resent: false,
        }
    }

    fn shed(operation: Operation, retry_after_secs: u64, reason: Option<&str>) -> ClientError {
        ClientError::Service(ServiceError::Shed {
            exchange: exchange(operation).into(),
            status: StatusCode::SERVICE_UNAVAILABLE,
            retry_after_secs,
            reason: reason.map(str::to_owned),
        })
    }

    fn transport(failure: TransportFailure) -> ClientError {
        ClientError::Service(ServiceError::Transport {
            exchange: exchange(Operation::Activities).into(),
            failure,
            cause: "connection reset".to_owned(),
        })
    }

    fn unexpected(operation: Operation, status: StatusCode, body: Value) -> ClientError {
        ClientError::Unexpected {
            exchange: exchange(operation).into(),
            status,
            body,
        }
    }

    /// Assert the code and the exact details of `error`'s mapping, and hand
    /// the mapped error back for message checks.
    fn mapped(error: ClientError, code: ErrorCode, expected_details: Option<&Value>) -> AppError {
        let label = format!("{error:?}");
        let app_error = to_app_error(error);
        assert_eq!(app_error.code, code, "code of {label}");
        assert_eq!(
            app_error.details.as_deref(),
            expected_details,
            "details of {label}"
        );
        app_error
    }

    #[test]
    fn not_configured_is_internal_and_names_the_variable() {
        let error = mapped(ClientError::NotConfigured, ErrorCode::InternalError, None);
        assert!(error.message.contains("DRAVR_SCIOTTE_REMOTE_URL"));
        assert!(error.message.contains("DRAVR_SCIOTTE_AUDIENCE"));
    }

    #[test]
    fn every_config_error_is_internal_under_its_own_words() {
        let service = || "sciotte".to_owned();
        let cases = [
            ConfigError::InvalidBaseUrl {
                service: service(),
                detail: "http is only allowed on loopback".to_owned(),
            },
            ConfigError::AudienceRequired { service: service() },
            ConfigError::AudienceNotSet {
                service: service(),
                url_var: "DRAVR_SCIOTTE_REMOTE_URL".to_owned(),
                audience_var: "DRAVR_SCIOTTE_AUDIENCE".to_owned(),
            },
            ConfigError::HttpClient {
                service: service(),
                detail: "no tls backend".to_owned(),
            },
        ];
        for config in cases {
            let said = config.to_string();
            let error = mapped(ClientError::Config(config), ErrorCode::InternalError, None);
            assert_eq!(error.message, said);
        }
    }

    #[test]
    fn a_token_that_could_not_be_minted_is_an_external_service_error() {
        let error = mapped(
            ClientError::Service(ServiceError::Identity {
                service: "sciotte".to_owned(),
                operation: Operation::Activities.as_str().to_owned(),
                source: IamError::MetadataUnavailable("no route".to_owned()),
            }),
            ErrorCode::ExternalServiceError,
            None,
        );
        assert!(error
            .message
            .starts_with("could not mint an identity token for sciotte: "));
    }

    #[test]
    fn no_response_is_unavailable_and_an_unsendable_request_is_internal() {
        for failure in [
            TransportFailure::TimedOut,
            TransportFailure::Unreachable,
            TransportFailure::ClosedBeforeResponse,
        ] {
            let error = mapped(
                transport(failure),
                ErrorCode::ExternalServiceUnavailable,
                None,
            );
            assert!(error.message.contains("after 42 ms: connection reset"));
            assert_eq!(error.provider_auth_required_provider(), None);
        }
        let error = mapped(
            transport(TransportFailure::Other),
            ErrorCode::InternalError,
            None,
        );
        assert!(error.message.contains("connection reset"));
    }

    #[test]
    fn a_shed_of_a_login_step_or_a_read_carries_the_services_wait() {
        for operation in [
            Operation::Login,
            Operation::SubmitOtp,
            Operation::Select2fa,
            Operation::Activities,
            Operation::PlannedWorkouts,
            Operation::Athlete,
            Operation::Activity,
            Operation::DailySummary,
        ] {
            let error = mapped(
                shed(operation, 17, Some("chrome_budget")),
                ErrorCode::ResourceUnavailable,
                Some(&json!({"retry_after_secs": 17})),
            );
            assert_eq!(shed_retry_after_secs(&error), Some(17));
            assert_eq!(
                error.message,
                "sciotte shed the request (chrome_budget); retry after 17s"
            );
        }
    }

    #[test]
    fn a_shed_naming_no_wait_or_no_reason_is_still_a_shed() {
        let error = mapped(
            shed(Operation::Login, 0, None),
            ErrorCode::ResourceUnavailable,
            Some(&json!({"retry_after_secs": 1})),
        );
        assert_eq!(shed_retry_after_secs(&error), Some(1));
        assert_eq!(
            error.message,
            "sciotte shed the request (scraper_busy); retry after 0s"
        );
    }

    #[test]
    fn a_shed_of_a_session_transfer_is_internal_and_carries_no_wait() {
        for operation in [Operation::Export, Operation::Import] {
            let client_error = shed(operation, 17, Some("chrome_budget"));
            let request = client_error.exchange().map(named).unwrap_or_default();
            let error = mapped(client_error, ErrorCode::InternalError, None);
            assert_eq!(shed_retry_after_secs(&error), None);
            assert_eq!(
                error.message,
                format!(
                    "sciotte {} returned 503 Service Unavailable ({request})",
                    operation.as_str()
                )
            );
        }
    }

    #[test]
    fn a_request_that_was_not_finished_is_unavailable_never_auth() {
        let cases = [
            (Unfinished::ServiceDeadline, StatusCode::GATEWAY_TIMEOUT),
            (Unfinished::GatewayDeadline, StatusCode::GATEWAY_TIMEOUT),
            (Unfinished::BadGateway, StatusCode::BAD_GATEWAY),
            (Unfinished::HandlerPanic, StatusCode::INTERNAL_SERVER_ERROR),
        ];
        for (kind, status) in cases {
            let error = mapped(
                ClientError::Service(ServiceError::Unfinished {
                    exchange: exchange(Operation::Activities).into(),
                    status,
                    kind,
                    detail: None,
                }),
                ErrorCode::ExternalServiceUnavailable,
                None,
            );
            assert!(error.message.contains(kind.describe()));
            assert!(error.message.ends_with("no detail"));
            assert_eq!(error.provider_auth_required_provider(), None);
        }
    }

    #[test]
    fn a_body_that_did_not_arrive_or_decode_is_internal() {
        let body = ClientError::Service(ServiceError::Body {
            exchange: exchange(Operation::Athlete).into(),
            status: StatusCode::OK,
            cause: "connection reset".to_owned(),
        });
        let decode = ClientError::Service(ServiceError::Decode {
            exchange: exchange(Operation::Activities).into(),
            status: StatusCode::OK,
            detail: "missing field `head_complete`".to_owned(),
        });
        let error = mapped(body, ErrorCode::InternalError, None);
        assert!(error.message.contains("its body could not be read"));
        let error = mapped(decode, ErrorCode::InternalError, None);
        assert!(error.message.contains("missing field `head_complete`"));
    }

    #[test]
    fn a_session_the_service_does_not_hold_or_the_provider_refused_asks_for_auth() {
        let auth_details = json!({"provider": "sciotte"});
        let not_found = mapped(
            ClientError::SessionNotFound {
                exchange: exchange(Operation::Activities).into(),
            },
            ErrorCode::ProviderAuthRequired,
            Some(&auth_details),
        );
        let expired = mapped(
            ClientError::SessionExpired {
                exchange: exchange(Operation::Activities).into(),
                message: Some("cookies rejected".to_owned()),
            },
            ErrorCode::ProviderAuthRequired,
            Some(&auth_details),
        );
        for error in [not_found, expired] {
            assert_eq!(error.message, "Provider sciotte requires authentication");
            assert_eq!(
                error.provider_auth_required_provider().as_deref(),
                Some("sciotte")
            );
        }
    }

    #[test]
    fn a_session_lost_with_its_instance_is_unavailable_never_auth() {
        let error = mapped(
            ClientError::SessionLostOnResend {
                exchange: exchange(Operation::Activities).into(),
            },
            ErrorCode::ExternalServiceUnavailable,
            None,
        );
        assert!(error
            .message
            .starts_with("sciotte no longer held the session when the read was re-sent"));
        assert_eq!(error.provider_auth_required_provider(), None);
    }

    #[test]
    fn a_coach_account_read_naming_no_athlete_is_invalid_input_with_its_marker() {
        let error = mapped(
            ClientError::AthleteRequired {
                exchange: exchange(Operation::PlannedWorkouts).into(),
                message: Some("name an athlete".to_owned()),
            },
            ErrorCode::InvalidInput,
            Some(&json!({"sciotte_refusal": "athlete_required"})),
        );
        assert_eq!(
            error.message,
            "This account is a coach account, and a coach account has no training calendar of \
             its own. To read an athlete's workouts, link each athlete from a group you coach; \
             the athlete confirms the link."
        );
        assert_eq!(sciotte_refusal(&error), Some(ATHLETE_REQUIRED));
        assert_eq!(error.provider_auth_required_provider(), None);
    }

    #[test]
    fn an_athlete_off_the_roster_is_permission_denied_with_its_marker() {
        let error = to_app_error(ClientError::AthleteNotAccessible {
            exchange: exchange(Operation::Activities).into(),
            athlete: Some("900001".to_owned()),
            message: Some("not on the roster".to_owned()),
        });
        // Compared with `==`: the PermissionDenied message review reads every
        // other use of the code in `src` as a refusal being built.
        assert!(error.code == ErrorCode::PermissionDenied, "{error:?}");
        assert_eq!(
            error.details.as_deref(),
            Some(&json!({"sciotte_refusal": "athlete_not_accessible"}))
        );
        assert_eq!(
            error.message,
            "That athlete is not on this coach account's roster"
        );
        assert_eq!(sciotte_refusal(&error), Some(ATHLETE_NOT_ACCESSIBLE));

        let built = athlete_not_accessible();
        assert_eq!(built.code, error.code);
        assert_eq!(built.message, error.message);
        assert_eq!(built.details, error.details);
    }

    #[test]
    fn a_query_the_service_refused_is_internal_and_no_athlete_refusal() {
        for marker in ["invalid_athlete", "invalid_window", "invalid_date"] {
            let error = mapped(
                ClientError::QueryRefused {
                    exchange: exchange(Operation::PlannedWorkouts).into(),
                    marker,
                    message: "after is later than before".to_owned(),
                },
                ErrorCode::InternalError,
                None,
            );
            assert_eq!(sciotte_refusal(&error), None);
            assert_eq!(
                error.message,
                format!(
                    "sciotte refused the planned-workouts query as {marker}: after is later \
                     than before"
                )
            );
        }
    }

    #[test]
    fn a_list_that_miscounts_its_rows_is_internal() {
        let error = mapped(
            ClientError::CountMismatch {
                exchange: exchange(Operation::Activities).into(),
                announced: 3,
                carried: 2,
            },
            ErrorCode::InternalError,
            None,
        );
        assert_eq!(
            error.message,
            "sciotte activities announced 3 activities and carried 2"
        );
    }

    #[test]
    fn a_login_answer_with_no_status_names_its_marker_and_none_of_its_body() {
        let body = json!({
            "error": "no_pending_login",
            "session": {"session_id": "s-1", "cookies": [{"name": "auth", "value": "hunter2"}]},
        });
        for operation in [Operation::Login, Operation::SubmitOtp, Operation::Select2fa] {
            let client_error = unexpected(operation, StatusCode::BAD_REQUEST, body.clone());
            let request = client_error.exchange().map(named).unwrap_or_default();
            let error = mapped(client_error, ErrorCode::InternalError, None);
            assert_eq!(
                error.message,
                format!(
                    "sciotte returned HTTP 400 Bad Request with no recognizable login status \
                     ({request}, marker: no_pending_login); check DRAVR_SCIOTTE_AUDIENCE / IAM \
                     / service health"
                )
            );
            assert!(!error.message.contains("hunter2"));
            assert!(!error.message.contains("s-1"));
        }
    }

    #[test]
    fn a_login_answer_from_the_token_gate_names_its_nested_type_or_none() {
        let gate = json!({"error": {"type": "unauthorized", "message": "bad audience"}});
        let error = mapped(
            unexpected(Operation::Login, StatusCode::UNAUTHORIZED, gate),
            ErrorCode::InternalError,
            None,
        );
        assert!(error.message.contains("marker: unauthorized)"));
        assert!(!error.message.contains("bad audience"));

        let error = mapped(
            unexpected(Operation::Login, StatusCode::BAD_GATEWAY, Value::Null),
            ErrorCode::InternalError,
            None,
        );
        assert!(error.message.contains("marker: none)"));
    }

    #[test]
    fn any_other_unexpected_answer_is_internal_under_the_clients_words() {
        for operation in [
            Operation::Export,
            Operation::Import,
            Operation::DeleteSession,
            Operation::Activities,
            Operation::Athlete,
        ] {
            let client_error = unexpected(
                operation,
                StatusCode::NOT_FOUND,
                json!({"error": "session_not_found"}),
            );
            let said = client_error.to_string();
            let error = mapped(client_error, ErrorCode::InternalError, None);
            assert_eq!(error.message, said);
            assert!(!error.message.contains("login status"));
            assert_eq!(error.provider_auth_required_provider(), None);
        }
    }

    #[test]
    fn the_details_readers_answer_none_for_an_error_of_another_kind() {
        assert_eq!(sciotte_refusal(&AppError::internal("plain")), None);
        assert_eq!(
            shed_retry_after_secs(&AppError::internal("chrome crashed")),
            None
        );
        // A wait on another code is that code's, not a shed's.
        let limited = AppError::new(ErrorCode::RateLimitExceeded, "slow down").with_retry_after(9);
        assert_eq!(shed_retry_after_secs(&limited), None);
    }
}
