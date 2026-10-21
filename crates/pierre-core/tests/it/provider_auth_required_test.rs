// ABOUTME: AppError::provider_auth_required + ProtocolError::ProviderAuthRequired round-trip tests
// ABOUTME: Pins the contract the chat pipeline relies on to detect re-auth signals across protocol boundary
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Unit tests for the `ProviderAuthRequired` error path.
//!
//! These tests pin the contract the chat pipeline's auth-recovery stage
//! relies on: a `ProviderAuthRequired` minted at the provider layer must
//! survive the `AppError → ProtocolError → AppError` round-trip used by
//! `UniversalExecutor` so the tool loop can detect it and short-circuit a
//! deterministic hosted-login reply.

use pierre_core::errors::{protocol::ProtocolError as PE, AppError, ErrorCode};

#[test]
fn provider_auth_required_carries_provider_in_details() {
    let err = AppError::provider_auth_required("sciotte_garmin");
    assert_eq!(err.code, ErrorCode::ProviderAuthRequired);
    assert_eq!(
        err.provider_auth_required_provider(),
        Some("sciotte_garmin".to_owned())
    );
}

#[test]
fn provider_auth_required_provider_returns_none_for_other_codes() {
    let err = AppError::auth_expired();
    assert!(err.provider_auth_required_provider().is_none());

    let err = AppError::not_found("activity 42");
    assert!(err.provider_auth_required_provider().is_none());
}

#[test]
fn protocol_error_provider_auth_required_extracts_slug() {
    let err = PE::ProviderAuthRequired {
        provider: "sciotte".to_owned(),
    };
    assert_eq!(err.provider_auth_required_provider(), Some("sciotte"));
}

#[test]
fn protocol_error_other_variants_do_not_extract_slug() {
    let err = PE::InvalidRequest("nope".to_owned());
    assert_eq!(err.provider_auth_required_provider(), None);

    let err = PE::InternalError("kaboom".to_owned());
    assert_eq!(err.provider_auth_required_provider(), None);
}

#[test]
fn provider_auth_required_round_trips_via_protocol_error() {
    // Path the chat tool loop relies on: provider returns AppError, executor
    // converts to ProtocolError, then `From<ProtocolError> for AppError`
    // converts back. The provider slug must survive both hops.
    let original = AppError::provider_auth_required("sciotte_garmin");
    assert_eq!(original.code, ErrorCode::ProviderAuthRequired);

    // Provider layer → executor: simulate the executor's
    // `app_error_to_protocol_error` mapping for this code.
    let provider = original
        .provider_auth_required_provider()
        .unwrap_or_else(|| "unknown".to_owned());
    let protocol_err = PE::ProviderAuthRequired { provider };

    // Executor → tool loop: From<ProtocolError> for AppError
    let recovered: AppError = protocol_err.into();
    assert_eq!(recovered.code, ErrorCode::ProviderAuthRequired);
    assert_eq!(
        recovered.provider_auth_required_provider(),
        Some("sciotte_garmin".to_owned())
    );
}

#[test]
fn provider_auth_required_serialises_error_code_as_string() {
    // The wire-format serializer for ErrorCode used by api responses must
    // know about the new variant — round-trip through serde to confirm.
    let json = serde_json::to_string(&ErrorCode::ProviderAuthRequired).ok();
    assert_eq!(json.as_deref(), Some("\"ProviderAuthRequired\""));

    // Round-trip through the documented wire shape so a future serializer
    // refactor that drops the variant breaks loud.
    let back = serde_json::from_str::<ErrorCode>("\"ProviderAuthRequired\"").ok();
    assert_eq!(back, Some(ErrorCode::ProviderAuthRequired));
}

#[test]
fn provider_auth_required_returns_403_forbidden() {
    // Sanity: this is an authorization issue (provider auth state), not a
    // missing-credentials issue, so HTTP 403 (matches AuthExpired neighbours).
    use pierre_core::constants::http_status::FORBIDDEN;
    assert_eq!(ErrorCode::ProviderAuthRequired.http_status(), FORBIDDEN);
}
