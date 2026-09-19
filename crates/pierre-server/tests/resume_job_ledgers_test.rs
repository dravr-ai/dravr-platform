// ABOUTME: Pins the claim semantics of the memory-extraction and activity-backfill job ledgers
// ABOUTME: A fresh row is not stale, a lapsed lease is, a claim leases and counts, finish deletes, one backfill per (user, provider)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Both ledgers carry the same contract the messaging resume relies on:
//! the row is written before the work is spawned, a resume sweep may take
//! it only once it is old enough that its own spawn is presumed dead and
//! its lease has lapsed, a claim leases it and bumps `attempts`, the cap
//! stops a job that keeps dying, and success deletes it. The backfill
//! ledger also refuses a second row for the same `(user, provider)`.

#![allow(missing_docs, clippy::unwrap_used, clippy::expect_used)]

use chrono::Utc;
use pierre_core::models::TenantId;
use pierre_database::database::test_utils::create_test_db;
use pierre_database::repositories::{
    ActivityBackfillJobRow, BackfillJobClaim, ExtractionJobClaim, MemoryExtractionJobRow,
};
use uuid::Uuid;

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

#[tokio::test]
async fn extraction_jobs_are_claimed_only_once_stale_and_unleased() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let ledger = &repos.memory_extraction_jobs;
    let tenant = TenantId::generate();
    let now = now_ms();

    let fresh = MemoryExtractionJobRow {
        id: Uuid::new_v4().to_string(),
        tenant_id: tenant,
        payload: r#"{"user_message":"fresh"}"#.to_owned(),
        created_at_ms: now,
        leased_until_ms: 0,
        attempts: 0,
    };
    let stale = MemoryExtractionJobRow {
        id: Uuid::new_v4().to_string(),
        tenant_id: tenant,
        payload: r#"{"user_message":"stale"}"#.to_owned(),
        created_at_ms: now - 10 * 60_000,
        leased_until_ms: 0,
        attempts: 0,
    };
    let exhausted = MemoryExtractionJobRow {
        id: Uuid::new_v4().to_string(),
        tenant_id: tenant,
        payload: r#"{"user_message":"exhausted"}"#.to_owned(),
        created_at_ms: now - 10 * 60_000,
        leased_until_ms: 0,
        attempts: 3,
    };
    for row in [&fresh, &stale, &exhausted] {
        ledger.record_extraction_job(row).await.unwrap();
    }

    let claim = ExtractionJobClaim {
        now_ms: now,
        queued_older_than_ms: 2 * 60_000,
        lease_ms: 5 * 60_000,
        max_attempts: 3,
        limit: 10,
    };
    let taken = ledger.claim_stale_extraction_jobs(claim).await.unwrap();
    assert_eq!(
        taken.len(),
        1,
        "only the stale, unexhausted row is claimable"
    );
    assert_eq!(taken[0].id, stale.id);
    assert_eq!(taken[0].attempts, 1, "a claim counts an attempt");
    assert_eq!(
        taken[0].leased_until_ms,
        now + 5 * 60_000,
        "a claim leases the row"
    );
    assert_eq!(taken[0].payload, stale.payload, "the payload round-trips");
    assert_eq!(taken[0].tenant_id, tenant);

    // Leased: a second sweep in the same instant takes nothing.
    let again = ledger.claim_stale_extraction_jobs(claim).await.unwrap();
    assert!(again.is_empty(), "a leased row is not claimed twice");

    // Once the lease lapses it is claimable again, with the attempt counted.
    // The staleness window is widened past the fresh row's age, so only the
    // lapsed lease makes this row claimable.
    let later = ExtractionJobClaim {
        now_ms: now + 6 * 60_000,
        queued_older_than_ms: 7 * 60_000,
        ..claim
    };
    let retaken = ledger.claim_stale_extraction_jobs(later).await.unwrap();
    assert_eq!(
        retaken.len(),
        1,
        "only the lapsed row; the fresh one is still young"
    );
    assert_eq!(retaken[0].id, stale.id);
    assert_eq!(retaken[0].attempts, 2);

    ledger.finish_extraction_job(&stale.id).await.unwrap();
    let after_finish = ledger
        .claim_stale_extraction_jobs(ExtractionJobClaim {
            now_ms: now + 20 * 60_000,
            ..claim
        })
        .await
        .unwrap();
    assert!(
        after_finish.iter().all(|r| r.id != stale.id),
        "a finished job is gone"
    );
    assert_eq!(
        after_finish
            .iter()
            .map(|r| r.id.as_str())
            .collect::<Vec<_>>(),
        vec![fresh.id.as_str()],
        "by then the once-fresh row is stale; the exhausted one stays untaken"
    );

    // The exhausted row is never claimed, so a reaper is what ends it — but
    // only once its last lease has lapsed, never under a live runner.
    assert_eq!(
        ledger.reap_exhausted_extraction_jobs(now, 3).await.unwrap(),
        1,
        "the row past the cap with a lapsed lease is reaped"
    );
    let remaining = ledger
        .claim_stale_extraction_jobs(ExtractionJobClaim {
            now_ms: now + 40 * 60_000,
            max_attempts: 100,
            ..claim
        })
        .await
        .unwrap();
    assert!(
        remaining.iter().all(|r| r.id != exhausted.id),
        "the reaped row is gone even to a claim with no cap"
    );
}

#[tokio::test]
async fn backfill_jobs_are_one_per_user_and_provider_and_resume_after_a_lapsed_lease() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let ledger = &repos.activity_backfill_jobs;
    let tenant = TenantId::generate();
    let user = Uuid::new_v4();
    let now = now_ms();

    let row = ActivityBackfillJobRow {
        id: Uuid::new_v4().to_string(),
        tenant_id: tenant,
        user_id: user,
        provider: "strava".to_owned(),
        after_ts: Some(1_700_000_000),
        before_ts: None,
        fetch_limit: Some(200),
        conversation_id: Some("conv-1".to_owned()),
        created_at_ms: now - 10 * 60_000,
        leased_until_ms: now - 60_000,
        attempts: 1,
    };
    assert!(
        ledger.record_backfill_job(&row).await.unwrap(),
        "first record inserts"
    );

    let duplicate = ActivityBackfillJobRow {
        id: Uuid::new_v4().to_string(),
        created_at_ms: now,
        ..row.clone()
    };
    assert!(
        !ledger.record_backfill_job(&duplicate).await.unwrap(),
        "a second ask for the same (user, provider) is the job already on file"
    );

    let claim = BackfillJobClaim {
        now_ms: now,
        queued_older_than_ms: 2 * 60_000,
        lease_ms: 10 * 60_000,
        max_attempts: 3,
        limit: 10,
    };
    let taken = ledger.claim_stale_backfill_jobs(claim).await.unwrap();
    assert_eq!(taken.len(), 1);
    let job = &taken[0];
    assert_eq!(
        job.id, row.id,
        "the first record's row, not the duplicate's"
    );
    assert_eq!(job.user_id, user);
    assert_eq!(job.provider, "strava");
    assert_eq!(job.after_ts, Some(1_700_000_000));
    assert_eq!(job.fetch_limit, Some(200));
    assert_eq!(job.conversation_id.as_deref(), Some("conv-1"));
    assert_eq!(job.attempts, 2, "the lapsed run and this claim");
    assert_eq!(job.leased_until_ms, now + 10 * 60_000);

    assert!(
        ledger
            .renew_backfill_job_lease(&job.id, now + 20 * 60_000)
            .await
            .unwrap(),
        "the holder can extend its own lease"
    );
    assert!(
        !ledger
            .renew_backfill_job_lease(&job.id, now + 15 * 60_000)
            .await
            .unwrap(),
        "a renewal that would shorten the lease is refused — it is not this runner's any more"
    );

    ledger.finish_backfill_job(&job.id).await.unwrap();
    assert!(
        ledger.record_backfill_job(&duplicate).await.unwrap(),
        "once finished, a new ask for the pair records a new job"
    );
}
