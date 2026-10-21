// ABOUTME: A failed tool call shows its caller the sanitized message, never the server's internal error text
// ABOUTME: Pins that database and internal failures stay in the log while validation messages still reach the caller
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A failed tool call's reply carries the sanitized message, never the
//! internal error text the server logs.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_core::errors::AppError;
use pierre_tool_runtime::conversions::tool_result_to_response;

fn reply_text(error: AppError) -> (String, String) {
    let response = tool_result_to_response(Err(error));
    assert!(response.is_error);
    let text = response.content[0].as_text().unwrap().to_owned();
    let structured = response.structured_content.unwrap()["error"]
        .as_str()
        .unwrap()
        .to_owned();
    (text, structured)
}

#[test]
fn a_database_failure_reaches_the_caller_without_its_sql() {
    let (text, structured) = reply_text(AppError::database(
        "error returned from database: relation \"user_oauth_tokens\" violates constraint fk_tenant",
    ));
    for shown in [&text, &structured] {
        assert!(!shown.contains("user_oauth_tokens"), "{shown}");
        assert!(!shown.contains("constraint"), "{shown}");
    }
}

#[test]
fn an_internal_failure_reaches_the_caller_without_its_detail() {
    let (text, _) = reply_text(AppError::internal(
        "panic in /srv/app/src/secret_path.rs:42",
    ));
    assert!(!text.contains("secret_path"), "{text}");
}

#[test]
fn a_validation_message_still_tells_the_caller_what_to_fix() {
    let (text, structured) = reply_text(AppError::invalid_input("limit must be between 1 and 100"));
    assert!(text.contains("limit must be between 1 and 100"), "{text}");
    assert_eq!(structured, text);
}

#[test]
fn a_missing_resource_and_a_provider_failure_keep_their_message() {
    let (missing, _) = reply_text(AppError::not_found("A saved training plan"));
    assert!(missing.contains("saved training plan"), "{missing}");
    let (provider, _) = reply_text(AppError::external_service(
        "intervals.icu",
        "answered 503 Service Unavailable",
    ));
    assert!(provider.contains("503"), "{provider}");
}
