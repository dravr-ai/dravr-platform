// ABOUTME: Unit tests for FollowupStatus string round-trips
// ABOUTME: Pins the DB strings agent follow-ups are stored under
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(missing_docs)]

use pierre_memory::followups::FollowupStatus;

#[test]
fn status_roundtrip() {
    for status in [
        FollowupStatus::Pending,
        FollowupStatus::Delivered,
        FollowupStatus::Cancelled,
    ] {
        assert_eq!(FollowupStatus::parse(status.as_str()), Some(status));
    }
}
