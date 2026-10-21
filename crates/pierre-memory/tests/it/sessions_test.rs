// ABOUTME: Unit tests for SessionStatus string round-trips
// ABOUTME: Pins the DB strings agent sessions are stored under
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(missing_docs)]

use pierre_memory::sessions::SessionStatus;

#[test]
fn status_roundtrip() {
    for status in [SessionStatus::Active, SessionStatus::Archived] {
        assert_eq!(SessionStatus::parse(status.as_str()), Some(status));
    }
}
