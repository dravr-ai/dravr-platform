// ABOUTME: Construction coverage for the Resend-backed transactional email service
// ABOUTME: Asserts an empty API key is refused up front instead of failing every later send
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The service sends through dravr-tronc's `ResendClient`, which refuses an
//! empty key at construction. Before, the service accepted any key and every
//! send then failed at Resend with a 401, one password-reset request at a time.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_email::ResendEmailService;

#[test]
fn an_empty_api_key_is_refused_at_construction() {
    let Err(error) = ResendEmailService::new(String::new(), "Dravr <no-reply@dravr.ai>".to_owned())
    else {
        panic!("an empty Resend key must not build a service");
    };
    assert!(
        error.message.contains("Resend API key is empty"),
        "the refusal must name the missing key, got: {}",
        error.message
    );
}

#[test]
fn a_non_empty_api_key_builds_the_service() {
    assert!(
        ResendEmailService::new("re_key".to_owned(), "Dravr <no-reply@dravr.ai>".to_owned())
            .is_ok()
    );
}
