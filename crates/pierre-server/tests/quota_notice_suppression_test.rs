// ABOUTME: Pins that the quota notice is claimed once per level per budget window
// ABOUTME: Regression for 2026-09-02 — five consecutive replies carried the same billing line
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! The envelope pushed the notice on every turn the athlete sat at or above the
//! warning threshold. There was no state anywhere in the path saying they had
//! already been told.
//!
//! Live 2026-09-02, five consecutive turns:
//!
//! ```text
//! Petite note : tu as utilisé 408342 de 500000 sur ton forfait.
//! Petite note : tu as utilisé 460584 de 500000 sur ton forfait.
//! Petite note : tu as utilisé 512953 de 500000 sur ton forfait.
//! Petite note : tu as utilisé 565369 de 500000 sur ton forfait.
//! Petite note : tu as utilisé 670828 de 500000 sur ton forfait.
//! ```
//!
//! They landed under the replies where the athlete was disputing the coach's
//! facts about his own training, so the last thing the conversation did before
//! he left was interleave billing telemetry with an argument (registre#251).
//!
//! The slot is keyed on `resets_at` rather than on a date: that string is the
//! window's identity, constant for the life of the budget period and different
//! the moment it rolls.

use pierre_chat_pipeline::quota_policy::claim_notice_slot;
use pierre_chat_pipeline::QuotaLevel;
use pierre_core::models::TenantId;
use pierre_database::database::test_utils::create_test_db;
use uuid::Uuid;

const WINDOW: &str = "2026-09-03T00:00:00Z";
const NEXT_WINDOW: &str = "2026-09-04T00:00:00Z";

#[tokio::test]
async fn the_notice_is_claimed_once_per_window() {
    let db = create_test_db().await.expect("test db");
    let counters = db.repositories().usage_counters;
    let tenant = TenantId::generate();
    let user = Uuid::new_v4();

    assert!(
        claim_notice_slot(
            counters.as_ref(),
            tenant,
            user,
            QuotaLevel::Approaching,
            WINDOW
        )
        .await,
        "the first turn over the threshold tells the athlete"
    );

    for turn in 2..=5 {
        assert!(
            !claim_notice_slot(
                counters.as_ref(),
                tenant,
                user,
                QuotaLevel::Approaching,
                WINDOW
            )
            .await,
            "turn {turn} must stay silent — five in a row is what the athlete got"
        );
    }
}

/// Crossing into the burst zone is news, even after the approaching notice.
/// One slot per window would have told him at 80% and never again, including
/// when he went over.
#[tokio::test]
async fn crossing_into_burst_earns_its_own_notice() {
    let db = create_test_db().await.expect("test db");
    let counters = db.repositories().usage_counters;
    let tenant = TenantId::generate();
    let user = Uuid::new_v4();

    assert!(
        claim_notice_slot(
            counters.as_ref(),
            tenant,
            user,
            QuotaLevel::Approaching,
            WINDOW
        )
        .await
    );
    assert!(
        claim_notice_slot(counters.as_ref(), tenant, user, QuotaLevel::Burst, WINDOW).await,
        "passing the cap is a different fact and must be said once"
    );
    assert!(
        !claim_notice_slot(counters.as_ref(), tenant, user, QuotaLevel::Burst, WINDOW).await,
        "but only once"
    );
}

/// A new budget window is a new conversation about the budget.
#[tokio::test]
async fn the_next_window_tells_them_again() {
    let db = create_test_db().await.expect("test db");
    let counters = db.repositories().usage_counters;
    let tenant = TenantId::generate();
    let user = Uuid::new_v4();

    assert!(
        claim_notice_slot(
            counters.as_ref(),
            tenant,
            user,
            QuotaLevel::Approaching,
            WINDOW
        )
        .await
    );
    assert!(
        claim_notice_slot(
            counters.as_ref(),
            tenant,
            user,
            QuotaLevel::Approaching,
            NEXT_WINDOW
        )
        .await,
        "the counter reset, so the athlete hears about the new budget"
    );
}

/// One athlete's notice does not consume another's.
#[tokio::test]
async fn the_slot_is_per_athlete() {
    let db = create_test_db().await.expect("test db");
    let counters = db.repositories().usage_counters;
    let tenant = TenantId::generate();

    let raph = Uuid::new_v4();
    let phil = Uuid::new_v4();

    assert!(
        claim_notice_slot(
            counters.as_ref(),
            tenant,
            raph,
            QuotaLevel::Approaching,
            WINDOW
        )
        .await
    );
    assert!(
        claim_notice_slot(
            counters.as_ref(),
            tenant,
            phil,
            QuotaLevel::Approaching,
            WINDOW
        )
        .await,
        "a shared room must not spend one member's notice on another"
    );
}
