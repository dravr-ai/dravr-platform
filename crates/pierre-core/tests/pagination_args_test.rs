// ABOUTME: parse_limit_offset is the one clamp offset-paged tools share — pinned at its edges
// ABOUTME: A missing, zero, oversized or non-numeric limit must never reach a query unclamped
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The input-domain rule says every pagination limit carries min/max bounds.
//! Enforced per call site it is one forgotten `.clamp()` from an unbounded
//! query, so the clamp is shared — and these cases are the shapes a hand-written
//! copy tends to miss: `0` (a query for nothing), a number far past the cap, and
//! a string where a number belongs.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_core::pagination::parse_limit_offset;
use serde_json::json;

#[test]
fn absent_arguments_take_the_defaults() {
    assert_eq!(parse_limit_offset(&json!({}), 50, 100), (50, 0));
}

#[test]
fn a_limit_of_zero_is_raised_to_one() {
    assert_eq!(
        parse_limit_offset(&json!({"limit": 0}), 50, 100).0,
        1,
        "a zero limit would ask the database for nothing at all"
    );
}

#[test]
fn an_oversized_limit_is_capped() {
    assert_eq!(
        parse_limit_offset(&json!({"limit": 10_000}), 50, 100).0,
        100,
        "the cap is the whole point of the guard"
    );
}

#[test]
fn a_non_numeric_limit_falls_back_to_the_default() {
    assert_eq!(parse_limit_offset(&json!({"limit": "x"}), 50, 100).0, 50);
    assert_eq!(parse_limit_offset(&json!({"limit": -3}), 50, 100).0, 50);
    assert_eq!(parse_limit_offset(&json!({"limit": null}), 50, 100).0, 50);
}

#[test]
fn a_valid_window_passes_through_untouched() {
    assert_eq!(
        parse_limit_offset(&json!({"limit": 25, "offset": 7}), 50, 100),
        (25, 7)
    );
}

#[test]
fn a_non_numeric_offset_starts_at_the_beginning() {
    assert_eq!(
        parse_limit_offset(&json!({"offset": "later"}), 50, 100).1,
        0
    );
    assert_eq!(parse_limit_offset(&json!({"offset": -1}), 50, 100).1, 0);
}
