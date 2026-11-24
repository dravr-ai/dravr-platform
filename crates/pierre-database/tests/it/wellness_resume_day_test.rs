// ABOUTME: Covers the day a history walk's paused wellness copy resumes from: kept under the lease, returned with the attempt
// ABOUTME: Read only by the lease and tenant that hold the row; a completed first fill outlives the walk and a new ask (carnet#1661)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A session-read cap that refuses a day of a walk's wellness copy pauses the
//! copy until the refusing window ends, with no attempt spent, and keeps the
//! day it stopped at so the next run copies from there. `create_test_db`
//! opens whichever `DATABASE_URL` names, so these run on `SQLite` and
//! `PostgreSQL` alike.

use dravr_tronc::dates::{CivilDate, Timestamp};
use pierre_core::models::TenantId;
use pierre_database::backends::factory::{Database, DatabaseBackend};
use pierre_database::repositories::{
    ActivityBackfillJobRow, BackfillJobClaim, BackfillJobKind, JobLease, JobProgress,
    JobProgressStep,
};
use pierre_test_support::db::create_test_db;
use sqlx::Row;
use uuid::Uuid;

fn now_ms() -> i64 {
    Timestamp::now().epoch_millis()
}

fn day(month: u32, day: u32) -> CivilDate {
    CivilDate::from_ymd(2026, month, day).unwrap()
}

/// A Garmin history walk owed for `user` in `tenant`, recorded ten minutes
/// ago and leased to the recording until an hour from now.
fn walk_row(tenant: TenantId, user: Uuid) -> ActivityBackfillJobRow {
    let now = now_ms();
    ActivityBackfillJobRow {
        id: Uuid::new_v4().to_string(),
        tenant_id: tenant,
        user_id: user,
        provider: "sciotte_garmin".to_owned(),
        kind: BackfillJobKind::HistoryWalk,
        after_ts: None,
        before_ts: Some(1_790_000_000),
        fetch_limit: Some(200),
        conversation_id: None,
        created_at_ms: now - 10 * 60_000,
        leased_until_ms: now + 60 * 60_000,
        attempts: 1,
        lease_token: Some(Uuid::new_v4().to_string()),
        progress: JobProgress::default(),
        turn_id: None,
        asked_message_id: None,
        walk_backend: None,
        walk_sport: None,
    }
}

fn lease_of(row: &ActivityBackfillJobRow) -> JobLease {
    JobLease {
        id: row.id.clone(),
        token: row.lease_token.clone().unwrap(),
    }
}

/// Every row whose retry is due at `now_ms`.
const fn claim_at(now_ms: i64) -> BackfillJobClaim {
    BackfillJobClaim {
        now_ms,
        queued_older_than_ms: 0,
        lease_ms: 10 * 60_000,
        max_attempts: 10,
        limit: 10,
    }
}

/// What row `id`'s `wellness_resume_day` column holds, read past any lease.
async fn stored_day(db: &Database, id: &str) -> Option<String> {
    let sql = "SELECT wellness_resume_day FROM activity_backfill_jobs WHERE id = $1";
    match db.backend() {
        DatabaseBackend::SQLite(sqlite) => sqlx::query(sql)
            .bind(id)
            .fetch_one(sqlite.pool())
            .await
            .unwrap()
            .get("wellness_resume_day"),
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(pg) => sqlx::query(sql)
            .bind(id)
            .fetch_one(pg.pool())
            .await
            .unwrap()
            .get("wellness_resume_day"),
    }
}

#[tokio::test]
async fn a_paused_copy_keeps_its_day_returns_its_attempt_and_waits_out_its_window() {
    let db = create_test_db().await.unwrap();
    let ledger = &db.repositories().activity_backfill_jobs;
    let (tenant, user) = (TenantId::generate(), Uuid::new_v4());
    let row = walk_row(tenant, user);
    assert!(
        ledger.record_backfill_job(&row).await.unwrap(),
        "the fixture's walk is recorded"
    );
    let lease = lease_of(&row);

    assert_eq!(
        ledger.wellness_resume_day(&lease, tenant).await.unwrap(),
        None,
        "bug: a walk that never paused resumes its copy from a day"
    );

    let until = now_ms() + 15 * 60_000;
    assert!(
        !ledger
            .pause_wellness_copy(&lease, TenantId::generate(), day(3, 7), until)
            .await
            .unwrap(),
        "bug: another tenant paused the walk's copy"
    );
    assert!(
        ledger
            .pause_wellness_copy(&lease, tenant, day(3, 7), until)
            .await
            .unwrap(),
        "bug: the lease that holds the walk could not pause its copy"
    );
    assert!(
        !ledger
            .pause_wellness_copy(&lease, tenant, day(3, 8), until)
            .await
            .unwrap(),
        "bug: a lease the pause released still writes the row"
    );

    // Not due before its window ends.
    let early = ledger
        .claim_stale_backfill_jobs(claim_at(now_ms()))
        .await
        .unwrap();
    assert!(
        early.iter().all(|claimed| claimed.id != row.id),
        "bug: a paused copy was claimed before its window ended"
    );

    // Due once it ends, with the attempt the pause returned: the claim counts
    // one, as the first run did.
    let claimed = ledger
        .claim_stale_backfill_jobs(claim_at(until + 1))
        .await
        .unwrap();
    let resumed = claimed
        .iter()
        .find(|claimed| claimed.id == row.id)
        .expect("the paused walk is claimed once its window ends");
    assert_eq!(
        resumed.attempts, 1,
        "bug: a pause spent one of the walk's attempts"
    );
    let resumed_lease = lease_of(resumed);
    assert_eq!(
        ledger
            .wellness_resume_day(&resumed_lease, tenant)
            .await
            .unwrap(),
        Some(day(3, 7)),
        "bug: the next run does not resume the copy from the day it stopped at"
    );
    assert_eq!(
        ledger
            .wellness_resume_day(&resumed_lease, TenantId::generate())
            .await
            .unwrap(),
        None,
        "bug: another tenant read the walk's resume day"
    );
    assert_eq!(
        ledger.wellness_resume_day(&lease, tenant).await.unwrap(),
        None,
        "bug: a lease no longer holding the row read its resume day"
    );

    // The walk's end: the copy is done, so no day is kept.
    let ended = ledger
        .end_history_walk(
            &resumed_lease,
            JobProgressStep {
                before_ts: 1_700_000_000,
                units_done: 1,
                rows_landed: 0,
                secs_per_unit: None,
                floor: None,
            },
        )
        .await
        .unwrap();
    assert!(ended.is_some(), "the lease that holds the walk ends it");
    assert_eq!(
        stored_day(&db, &row.id).await,
        None,
        "bug: an ended walk keeps a day to resume a copy that is done"
    );
}

#[tokio::test]
async fn a_new_ask_of_an_ended_walk_copies_its_whole_window() {
    let db = create_test_db().await.unwrap();
    let ledger = &db.repositories().activity_backfill_jobs;
    let (tenant, user) = (TenantId::generate(), Uuid::new_v4());
    let first = walk_row(tenant, user);
    assert!(
        ledger.record_backfill_job(&first).await.unwrap(),
        "the fixture's walk is recorded"
    );
    let until = now_ms() - 1;
    assert!(
        ledger
            .pause_wellness_copy(&lease_of(&first), tenant, day(5, 1), until)
            .await
            .unwrap(),
        "bug: the lease that holds the walk could not pause its copy"
    );
    let claimed = ledger
        .claim_stale_backfill_jobs(claim_at(now_ms()))
        .await
        .unwrap();
    let held = claimed
        .iter()
        .find(|claimed| claimed.id == first.id)
        .expect("the paused walk is claimed once its window ended");
    // Ended without its copy done, the day still on the row.
    assert!(
        ledger
            .end_backfill_job_incomplete(&lease_of(held))
            .await
            .unwrap(),
        "the lease that holds the walk ends it incomplete"
    );
    assert_eq!(
        stored_day(&db, &first.id).await.as_deref(),
        Some("2026-05-01"),
        "the incomplete end leaves the row as it was"
    );

    let again = walk_row(tenant, user);
    assert!(
        ledger.record_backfill_job(&again).await.unwrap(),
        "a new ask replaces an ended walk"
    );
    assert_eq!(
        ledger
            .wellness_resume_day(&lease_of(&again), tenant)
            .await
            .unwrap(),
        None,
        "bug: a new walk inherits the day an ended one stopped at and skips history"
    );
}

/// A walk's first wellness fill, once complete, is remembered on its row
/// across the walk's end and a new ask, so a later copy counts in the
/// ordinary background caps; only the lease and tenant that hold the row
/// read or write it, and the first time it completed is kept.
#[tokio::test]
async fn a_completed_first_fill_is_kept_across_the_walks_end_and_a_new_ask() {
    let db = create_test_db().await.unwrap();
    let ledger = &db.repositories().activity_backfill_jobs;
    let (tenant, user) = (TenantId::generate(), Uuid::new_v4());
    let first = walk_row(tenant, user);
    assert!(
        ledger.record_backfill_job(&first).await.unwrap(),
        "the fixture's walk is recorded"
    );
    let lease = lease_of(&first);
    assert!(
        !ledger.wellness_first_filled(&lease, tenant).await.unwrap(),
        "bug: a walk that never copied reads as first-filled"
    );
    assert!(
        !ledger
            .mark_wellness_first_filled(&lease, TenantId::generate(), now_ms())
            .await
            .unwrap(),
        "bug: another tenant marked the walk's first fill"
    );
    let filled_at = now_ms();
    assert!(
        ledger
            .mark_wellness_first_filled(&lease, tenant, filled_at)
            .await
            .unwrap(),
        "bug: the lease that holds the walk could not mark its first fill"
    );
    assert!(
        ledger
            .mark_wellness_first_filled(&lease, tenant, filled_at + 60_000)
            .await
            .unwrap(),
        "bug: marking a first fill twice is refused"
    );
    assert!(
        ledger.wellness_first_filled(&lease, tenant).await.unwrap(),
        "bug: a marked first fill is not read back"
    );
    assert!(
        !ledger
            .wellness_first_filled(&lease, TenantId::generate())
            .await
            .unwrap(),
        "bug: another tenant read the walk's first fill"
    );

    let ended = ledger
        .end_history_walk(
            &lease,
            JobProgressStep {
                before_ts: 1_700_000_000,
                units_done: 1,
                rows_landed: 0,
                secs_per_unit: None,
                floor: None,
            },
        )
        .await
        .unwrap();
    assert!(ended.is_some(), "the lease that holds the walk ends it");

    let again = walk_row(tenant, user);
    assert!(
        ledger.record_backfill_job(&again).await.unwrap(),
        "a new ask replaces an ended walk"
    );
    assert!(
        ledger
            .wellness_first_filled(&lease_of(&again), tenant)
            .await
            .unwrap(),
        "bug: a new ask forgot the completed first fill, and copies under its allowance again"
    );
    assert_eq!(
        stored_filled_at(&db, &again.id).await,
        Some(filled_at),
        "bug: a second mark moved the time the first fill completed"
    );
}

/// What row `id`'s `wellness_filled_at_ms` column holds, read past any lease.
async fn stored_filled_at(db: &Database, id: &str) -> Option<i64> {
    let sql = "SELECT wellness_filled_at_ms FROM activity_backfill_jobs WHERE id = $1";
    match db.backend() {
        DatabaseBackend::SQLite(sqlite) => sqlx::query(sql)
            .bind(id)
            .fetch_one(sqlite.pool())
            .await
            .unwrap()
            .get("wellness_filled_at_ms"),
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(pg) => sqlx::query(sql)
            .bind(id)
            .fetch_one(pg.pool())
            .await
            .unwrap()
            .get("wellness_filled_at_ms"),
    }
}
