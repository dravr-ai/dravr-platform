// ABOUTME: Smoke test for pierre-notifications' bridge layer to dravr-commere
// ABOUTME: Exercises cron validation + AppError mapping that pierre-server depends on
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Smoke integration tests for the crate public API.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp
)]

use pierre_notifications::{to_app_error, validate_cron_expression, CommereError};

#[test]
fn validate_cron_expression_accepts_a_valid_schedule() {
    // "minute hour dom month dow" — every day at 09:00 UTC
    validate_cron_expression("0 9 * * *").expect("valid cron must parse");
}

#[test]
fn validate_cron_expression_rejects_garbage() {
    let err = validate_cron_expression("not a real cron").err();
    assert!(err.is_some(), "garbage cron expressions must fail");
}

#[test]
fn to_app_error_maps_validation_to_invalid_input() {
    let err = CommereError::Validation {
        field: "cron".to_owned(),
        reason: "syntactically wrong".to_owned(),
    };
    let app = to_app_error(err);
    let msg = format!("{app}");
    assert!(
        msg.contains("cron") && msg.contains("syntactically wrong"),
        "AppError must preserve field + reason: {msg}"
    );
}
