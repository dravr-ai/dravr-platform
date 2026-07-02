// ABOUTME: Unit tests for archetype aggregation — sport bucketing + k-anonymity distinct-user counting
// ABOUTME: Proves priors group by (archetype, trigger, intervention), sum counts, and gate on K distinct users (P6)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Unit tests for archetype aggregation: sport bucketing and k-anonymity counting.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pierre_database::repositories::PlaybookAggInput;
use pierre_services::archetype_aggregation::{archetype_key_of, build_priors};

fn agg(user: &str, sport: &str, success: i64, failure: i64) -> PlaybookAggInput {
    PlaybookAggInput {
        user_id: user.to_owned(),
        trigger_hash: format!("hrv_drop:{sport}:high"),
        intervention_hash: "easy_block:*".to_owned(),
        trigger_json: format!(r#"{{"kind":"hrv_drop","sport":"{sport}","magnitude":"high"}}"#),
        intervention_json: r#"{"kind":"easy_block","magnitude":null}"#.to_owned(),
        success_count: success,
        failure_count: failure,
    }
}

#[test]
fn archetype_key_derives_sport_or_any() {
    assert_eq!(
        archetype_key_of(r#"{"kind":"hrv_drop","sport":"run","magnitude":"high"}"#),
        "run"
    );
    assert_eq!(
        archetype_key_of(r#"{"kind":"hrv_drop","sport":null,"magnitude":"high"}"#),
        "any"
    );
    // Unparseable -> the safe "any" bucket.
    assert_eq!(archetype_key_of("garbage"), "any");
}

#[test]
fn build_priors_groups_sums_and_enforces_k_anonymity() {
    let rows = vec![
        agg("u1", "run", 4, 1),
        agg("u2", "run", 3, 0),
        agg("u3", "run", 2, 2),
        // A second row from u1 — must NOT inflate the distinct-user count.
        agg("u1", "run", 1, 0),
    ];

    // k=3: 3 distinct users qualifies.
    let result = build_priors(rows.clone(), 3);
    assert_eq!(result.priors.len(), 1);
    assert!(result.prune.is_empty(), "at-floor bucket is not pruned");
    let p = &result.priors[0];
    assert_eq!(p.archetype_key, "run");
    assert_eq!(p.distinct_user_count, 3, "distinct users, not rows");
    assert_eq!(p.success_count, 10, "4+3+2+1 summed across the 4 rows");
    assert_eq!(p.failure_count, 3, "1+0+2+0 summed across the 4 rows");

    // k=4: only 3 distinct users -> below the floor -> not a prior, and the
    // bucket is queued for pruning so a previously-materialized row is deleted.
    let below = build_priors(rows, 4);
    assert!(below.priors.is_empty());
    assert_eq!(below.prune.len(), 1, "sub-K bucket is queued for prune");
}

#[test]
fn build_priors_separates_buckets_by_sport() {
    let rows = vec![
        agg("u1", "run", 5, 0),
        agg("u2", "run", 5, 0),
        agg("u3", "ride", 5, 0),
    ];
    // k=2: the "run" bucket has 2 users; "ride" has only 1 and is dropped.
    let result = build_priors(rows, 2);
    assert_eq!(result.priors.len(), 1);
    assert_eq!(result.priors[0].archetype_key, "run");
    // The single-user "ride" bucket falls below k=2 and is queued for prune.
    assert_eq!(result.prune.len(), 1);
}
