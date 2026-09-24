// ABOUTME: Unit tests for Commitment — status and outcome string round-trips and window length
// ABOUTME: Pins that a same-day commitment window still counts as one day
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(missing_docs)]

use chrono::{Duration, Utc};
use pierre_memory::commitments::{Commitment, CommitmentOutcome, CommitmentStatus};

fn sample() -> Commitment {
    let now = Utc::now();
    Commitment {
        id: "c1".to_owned(),
        tenant_id: "t1".to_owned(),
        user_id: "u1".to_owned(),
        agent_id: None,
        conversation_id: None,
        statement: "three easy runs this week".to_owned(),
        sport: Some("run".to_owned()),
        target_sessions: 3,
        window_start: now,
        window_end: now + Duration::days(7),
        status: CommitmentStatus::Open,
        outcome: None,
        completed_sessions: None,
        swept_at: None,
        reported_at: None,
        created_at: now,
        updated_at: now,
    }
}

#[test]
fn status_roundtrip() {
    for status in [
        CommitmentStatus::Open,
        CommitmentStatus::Labeled,
        CommitmentStatus::Reported,
        CommitmentStatus::Expired,
        CommitmentStatus::Cancelled,
    ] {
        assert_eq!(CommitmentStatus::parse(status.as_str()), Some(status));
    }
    assert_eq!(CommitmentStatus::parse("nope"), None);
}

#[test]
fn outcome_roundtrip() {
    for outcome in [
        CommitmentOutcome::Met,
        CommitmentOutcome::Partial,
        CommitmentOutcome::Missed,
    ] {
        assert_eq!(CommitmentOutcome::parse(outcome.as_str()), Some(outcome));
    }
    assert_eq!(CommitmentOutcome::parse("nope"), None);
}

#[test]
fn window_days_floors_at_one() {
    let mut c = sample();
    assert_eq!(c.window_days(), 7);
    c.window_end = c.window_start;
    assert_eq!(c.window_days(), 1, "a same-day window still reads as a day");
}
