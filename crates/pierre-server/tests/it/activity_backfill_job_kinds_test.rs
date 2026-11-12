// ABOUTME: Pins the job ledger's kind model: one row per (user, provider, kind), a live read's re-ask rebinds its turn
// ABOUTME: A backfill re-asked while owed changes nothing, and a step's progress is written only under the lease that holds the row
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A history walk, a live read and a backfill for the same athlete and
//! provider are three different jobs, so the ledger keeps each as its own
//! row. A live read asked again while one is owed answers the newest ask:
//! its follow-up moves to the new conversation, turn and message. Any other
//! kind asked again while owed is the same job and nothing changes. A walk
//! reports its progress one step at a time, and only the runner whose lease
//! holds the row may write it.

#![allow(missing_docs, clippy::unwrap_used, clippy::expect_used)]

use chrono::Utc;
use pierre_core::models::TenantId;
use pierre_database::repositories::{
    ActivityBackfillJobRow, BackfillJobClaim, BackfillJobKind, BackfillJobState, JobLease,
    JobProgress, JobProgressStep,
};
use pierre_test_support::db::create_test_db;
use uuid::Uuid;

const PROVIDER: &str = "strava";

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

/// A row owed for `user` in `tenant`, recorded ten minutes ago and leased
/// until a minute ago, so a claim made now takes it.
fn owed_row(tenant: TenantId, user: Uuid, kind: BackfillJobKind) -> ActivityBackfillJobRow {
    let now = now_ms();
    ActivityBackfillJobRow {
        id: Uuid::new_v4().to_string(),
        tenant_id: tenant,
        user_id: user,
        provider: PROVIDER.to_owned(),
        kind,
        after_ts: Some(1_700_000_000),
        before_ts: Some(1_710_000_000),
        fetch_limit: Some(200),
        conversation_id: Some("conv-1".to_owned()),
        created_at_ms: now - 10 * 60_000,
        leased_until_ms: now - 60_000,
        attempts: 1,
        lease_token: Some(Uuid::new_v4().to_string()),
        progress: JobProgress::default(),
        turn_id: Some("turn-1".to_owned()),
        asked_message_id: Some("message-1".to_owned()),
    }
}

/// Every row a claim made now takes.
fn claim_now() -> BackfillJobClaim {
    BackfillJobClaim {
        now_ms: now_ms(),
        queued_older_than_ms: 0,
        lease_ms: 10 * 60_000,
        max_attempts: 10,
        limit: 10,
    }
}

#[tokio::test]
async fn a_walk_a_live_read_and_a_backfill_are_three_rows_for_one_provider() {
    let db = create_test_db().await.unwrap();
    let ledger = &db.repositories().activity_backfill_jobs;
    let tenant = TenantId::generate();
    let user = Uuid::new_v4();

    for kind in [
        BackfillJobKind::Backfill,
        BackfillJobKind::HistoryWalk,
        BackfillJobKind::LiveRead,
    ] {
        assert!(
            ledger
                .record_backfill_job(&owed_row(tenant, user, kind))
                .await
                .unwrap(),
            "bug: a {kind} job is recorded beside the other kinds owed for the provider"
        );
        assert_eq!(
            ledger
                .backfill_job_state(user, tenant, PROVIDER, kind)
                .await
                .unwrap(),
            Some(BackfillJobState::Owed),
            "bug: the {kind} job reads as owed"
        );
    }

    let mut kinds: Vec<&str> = ledger
        .claim_stale_backfill_jobs(claim_now())
        .await
        .unwrap()
        .iter()
        .map(|row| row.kind.as_str())
        .collect();
    kinds.sort_unstable();
    assert_eq!(
        kinds,
        ["backfill", "history_walk", "live_read"],
        "bug: each kind is its own row, claimed on its own"
    );
}

#[tokio::test]
async fn a_live_read_asked_again_while_owed_answers_the_newest_ask() {
    let db = create_test_db().await.unwrap();
    let ledger = &db.repositories().activity_backfill_jobs;
    let tenant = TenantId::generate();
    let user = Uuid::new_v4();
    let first = owed_row(tenant, user, BackfillJobKind::LiveRead);
    assert!(
        ledger.record_backfill_job(&first).await.unwrap(),
        "bug: the first live read is recorded"
    );

    let reask = ActivityBackfillJobRow {
        id: Uuid::new_v4().to_string(),
        after_ts: Some(1_600_000_000),
        conversation_id: Some("conv-2".to_owned()),
        turn_id: Some("turn-2".to_owned()),
        asked_message_id: Some("message-2".to_owned()),
        lease_token: Some(Uuid::new_v4().to_string()),
        ..first.clone()
    };
    assert!(
        !ledger.record_backfill_job(&reask).await.unwrap(),
        "bug: a re-ask while one is owed is the job already on file, never a second run"
    );
    let elsewhere = ActivityBackfillJobRow {
        id: Uuid::new_v4().to_string(),
        tenant_id: TenantId::generate(),
        conversation_id: Some("conv-other-tenant".to_owned()),
        turn_id: Some("turn-other-tenant".to_owned()),
        asked_message_id: Some("message-other-tenant".to_owned()),
        ..first.clone()
    };
    assert!(
        !ledger.record_backfill_job(&elsewhere).await.unwrap(),
        "bug: an ask from another tenant is refused"
    );

    let claimed = ledger.claim_stale_backfill_jobs(claim_now()).await.unwrap();
    assert_eq!(claimed.len(), 1, "bug: one live read is owed, not two");
    let row = &claimed[0];
    assert_eq!(row.id, first.id, "bug: the owed row is the first ask's");
    assert_eq!(
        row.after_ts, first.after_ts,
        "bug: a re-ask moves where the follow-up goes, never the window"
    );
    assert_eq!(
        row.conversation_id.as_deref(),
        Some("conv-2"),
        "bug: the follow-up goes to the newest ask's conversation"
    );
    assert_eq!(
        row.turn_id.as_deref(),
        Some("turn-2"),
        "bug: the follow-up answers the newest ask's turn"
    );
    assert_eq!(
        row.asked_message_id.as_deref(),
        Some("message-2"),
        "bug: the follow-up answers the newest ask's message, and no other tenant's"
    );
}

#[tokio::test]
async fn a_backfill_asked_again_while_owed_changes_nothing() {
    let db = create_test_db().await.unwrap();
    let ledger = &db.repositories().activity_backfill_jobs;
    let tenant = TenantId::generate();
    let user = Uuid::new_v4();
    let first = owed_row(tenant, user, BackfillJobKind::Backfill);
    assert!(
        ledger.record_backfill_job(&first).await.unwrap(),
        "bug: the first backfill is recorded"
    );
    let reask = ActivityBackfillJobRow {
        id: Uuid::new_v4().to_string(),
        conversation_id: Some("conv-2".to_owned()),
        turn_id: Some("turn-2".to_owned()),
        asked_message_id: Some("message-2".to_owned()),
        ..first.clone()
    };
    assert!(
        !ledger.record_backfill_job(&reask).await.unwrap(),
        "bug: a re-ask while owed is the job already on file"
    );

    let claimed = ledger.claim_stale_backfill_jobs(claim_now()).await.unwrap();
    assert_eq!(claimed.len(), 1, "bug: one backfill is owed");
    assert_eq!(
        claimed[0].conversation_id.as_deref(),
        Some("conv-1"),
        "bug: only a live read is rebound; a backfill keeps the ask that recorded it"
    );
    assert_eq!(
        claimed[0].asked_message_id.as_deref(),
        Some("message-1"),
        "bug: a backfill keeps the message that recorded it"
    );
}

#[tokio::test]
async fn a_walks_progress_is_written_only_under_the_lease_that_holds_it() {
    let db = create_test_db().await.unwrap();
    let ledger = &db.repositories().activity_backfill_jobs;
    let tenant = TenantId::generate();
    let user = Uuid::new_v4();
    let mut walk = owed_row(tenant, user, BackfillJobKind::HistoryWalk);
    walk.leased_until_ms = now_ms() + 10 * 60_000;
    walk.progress.months_total = Some(120);
    assert!(
        ledger.record_backfill_job(&walk).await.unwrap(),
        "bug: the walk is recorded"
    );
    assert_eq!(
        ledger
            .job_progress(user, tenant, PROVIDER, BackfillJobKind::Backfill)
            .await
            .unwrap(),
        None,
        "bug: no backfill is on file, so it has no progress"
    );
    let holder = JobLease {
        id: walk.id.clone(),
        token: walk.lease_token.clone().unwrap(),
    };
    let stranger = JobLease {
        id: walk.id.clone(),
        token: Uuid::new_v4().to_string(),
    };
    let first_step = JobProgressStep {
        before_ts: 1_600_000_000,
        months_walked: 36,
        rows_landed: 10,
        secs_per_unit: Some(4.5),
        floor: None,
    };

    assert!(
        !ledger
            .record_job_progress(&stranger, first_step)
            .await
            .unwrap(),
        "bug: a runner whose lease does not hold the row cannot write its progress"
    );
    assert_eq!(
        ledger
            .job_progress(user, tenant, PROVIDER, BackfillJobKind::HistoryWalk)
            .await
            .unwrap(),
        Some(JobProgress {
            months_total: Some(120),
            months_done: 0,
            rows_landed: 0,
            secs_per_unit: None,
        }),
        "bug: a refused write changes nothing"
    );

    assert!(
        ledger
            .record_job_progress(&holder, first_step)
            .await
            .unwrap(),
        "bug: the lease holder writes its step"
    );
    assert_eq!(
        ledger
            .job_progress(user, tenant, PROVIDER, BackfillJobKind::HistoryWalk)
            .await
            .unwrap(),
        Some(JobProgress {
            months_total: Some(120),
            months_done: 36,
            rows_landed: 10,
            secs_per_unit: Some(4.5),
        }),
        "bug: the step is added to the walk's progress"
    );
    assert!(
        !ledger
            .record_job_progress(&holder, first_step)
            .await
            .unwrap(),
        "bug: a written step releases the row, so its old lease writes nothing more"
    );

    let claimed = ledger.claim_stale_backfill_jobs(claim_now()).await.unwrap();
    assert_eq!(
        claimed.len(),
        1,
        "bug: the walk stays owed for its next step"
    );
    let next = &claimed[0];
    assert_eq!(
        next.before_ts,
        Some(1_600_000_000),
        "bug: the next step reads below where this one reached"
    );
    assert_eq!(
        next.attempts, 1,
        "bug: a landed step starts the next one's attempts over"
    );
    let next_holder = JobLease {
        id: next.id.clone(),
        token: next.lease_token.clone().unwrap(),
    };
    assert!(
        ledger
            .record_job_progress(
                &next_holder,
                JobProgressStep {
                    before_ts: 1_500_000_000,
                    months_walked: 36,
                    rows_landed: 5,
                    secs_per_unit: None,
                    floor: None,
                },
            )
            .await
            .unwrap(),
        "bug: the next claim's holder writes its step"
    );
    assert_eq!(
        ledger
            .job_progress(user, tenant, PROVIDER, BackfillJobKind::HistoryWalk)
            .await
            .unwrap(),
        Some(JobProgress {
            months_total: Some(120),
            months_done: 72,
            rows_landed: 15,
            secs_per_unit: Some(4.5),
        }),
        "bug: steps add up, and a step that measured nothing keeps the measured pace"
    );
}

/// A live read nobody waits on — a stale head's top-up — asked while a
/// chat's read is owed never takes that chat's follow-up away.
#[tokio::test]
async fn a_live_read_asked_with_no_conversation_keeps_the_owed_follow_up() {
    let db = create_test_db().await.unwrap();
    let ledger = &db.repositories().activity_backfill_jobs;
    let tenant = TenantId::generate();
    let user = Uuid::new_v4();
    let chat = owed_row(tenant, user, BackfillJobKind::LiveRead);
    assert!(
        ledger.record_backfill_job(&chat).await.unwrap(),
        "bug: the chat's live read is recorded"
    );
    let top_up = ActivityBackfillJobRow {
        id: Uuid::new_v4().to_string(),
        conversation_id: None,
        turn_id: None,
        asked_message_id: None,
        ..chat.clone()
    };
    assert!(
        !ledger.record_backfill_job(&top_up).await.unwrap(),
        "bug: a top-up asked while a read is owed is that read, never a second one"
    );
    assert!(
        !ledger.rebind_live_read(&top_up).await.unwrap(),
        "bug: an ask with no conversation rebinds nothing"
    );

    let claimed = ledger.claim_stale_backfill_jobs(claim_now()).await.unwrap();
    assert_eq!(claimed.len(), 1, "bug: one live read is owed");
    assert_eq!(
        claimed[0].conversation_id.as_deref(),
        Some("conv-1"),
        "bug: the follow-up still goes to the chat that asked"
    );
    assert_eq!(
        claimed[0].asked_message_id.as_deref(),
        Some("message-1"),
        "bug: the follow-up still answers the chat's message"
    );
}

/// A re-ask that arrives while this instance runs the read rebinds the owed
/// row and never records one of its own; with nothing owed it records
/// nothing at all, and the row it finishes hands back the ask it was bound to.
#[tokio::test]
async fn a_rebind_only_moves_an_owed_live_read_and_finishing_returns_its_ask() {
    let db = create_test_db().await.unwrap();
    let ledger = &db.repositories().activity_backfill_jobs;
    let tenant = TenantId::generate();
    let user = Uuid::new_v4();
    let first = owed_row(tenant, user, BackfillJobKind::LiveRead);
    let reask = ActivityBackfillJobRow {
        id: Uuid::new_v4().to_string(),
        conversation_id: Some("conv-2".to_owned()),
        turn_id: Some("turn-2".to_owned()),
        asked_message_id: Some("message-2".to_owned()),
        ..first.clone()
    };
    assert!(
        !ledger.rebind_live_read(&reask).await.unwrap(),
        "bug: with nothing owed a rebind finds no row"
    );
    assert_eq!(
        ledger
            .backfill_job_state(user, tenant, PROVIDER, BackfillJobKind::LiveRead)
            .await
            .unwrap(),
        None,
        "bug: a rebind never records a live read of its own"
    );

    assert!(
        ledger.record_backfill_job(&first).await.unwrap(),
        "bug: the first live read is recorded"
    );
    let elsewhere = ActivityBackfillJobRow {
        tenant_id: TenantId::generate(),
        conversation_id: Some("conv-other-tenant".to_owned()),
        ..reask.clone()
    };
    assert!(
        !ledger.rebind_live_read(&elsewhere).await.unwrap(),
        "bug: another tenant's ask never rebinds this tenant's read"
    );
    assert!(
        ledger.rebind_live_read(&reask).await.unwrap(),
        "bug: a re-ask rebinds the owed read"
    );

    let lease = JobLease {
        id: first.id.clone(),
        token: first.lease_token.clone().unwrap(),
    };
    let stolen = JobLease {
        id: first.id.clone(),
        token: Uuid::new_v4().to_string(),
    };
    assert_eq!(
        ledger.finish_live_read(&stolen).await.unwrap(),
        None,
        "bug: only the lease that holds the row finishes it"
    );
    let ask = ledger
        .finish_live_read(&lease)
        .await
        .unwrap()
        .expect("bug: the holding lease finishes the row");
    assert_eq!(
        (
            ask.conversation_id.as_deref(),
            ask.turn_id.as_deref(),
            ask.asked_message_id.as_deref()
        ),
        (Some("conv-2"), Some("turn-2"), Some("message-2")),
        "bug: finishing hands back the newest ask the row was bound to"
    );
    assert_eq!(
        ledger
            .backfill_job_state(user, tenant, PROVIDER, BackfillJobKind::LiveRead)
            .await
            .unwrap(),
        None,
        "bug: a finished live read leaves no row"
    );
}

/// A walk asked again after one ended (a new connection) starts from the
/// top: the ended row's cursor and progress are replaced, so the sweep that
/// resumes the new walk never reads on from the old walk's floor and calls
/// the history walked without listing it.
#[tokio::test]
async fn a_walk_asked_after_one_ended_starts_from_the_top() {
    let db = create_test_db().await.unwrap();
    let ledger = &db.repositories().activity_backfill_jobs;
    let tenant = TenantId::generate();
    let user = Uuid::new_v4();
    let mut ended = owed_row(tenant, user, BackfillJobKind::HistoryWalk);
    ended.leased_until_ms = now_ms() + 10 * 60_000;
    assert!(
        ledger.record_backfill_job(&ended).await.unwrap(),
        "bug: the first walk is recorded"
    );
    let holder = JobLease {
        id: ended.id.clone(),
        token: ended.lease_token.clone().unwrap(),
    };
    let last_step = JobProgressStep {
        before_ts: 1_400_000_000,
        months_walked: 48,
        rows_landed: 30,
        secs_per_unit: Some(2.0),
        floor: None,
    };
    assert!(
        ledger
            .end_history_walk(&holder, last_step)
            .await
            .unwrap()
            .is_some(),
        "bug: the lease holder ends the walk"
    );
    assert_eq!(
        ledger
            .backfill_job_state(user, tenant, PROVIDER, BackfillJobKind::HistoryWalk)
            .await
            .unwrap(),
        Some(BackfillJobState::Walked),
        "bug: an ended walk is kept as walked"
    );

    let mut again = owed_row(tenant, user, BackfillJobKind::HistoryWalk);
    again.before_ts = None;
    again.after_ts = Some(946_684_800);
    again.attempts = 0;
    assert!(
        ledger.record_backfill_job(&again).await.unwrap(),
        "bug: a new walk replaces the ended row"
    );
    let claimed = ledger.claim_stale_backfill_jobs(claim_now()).await.unwrap();
    assert_eq!(claimed.len(), 1, "bug: the new walk is owed");
    assert_eq!(
        claimed[0].before_ts, None,
        "bug: the new walk reads from the top, not below the ended walk's floor"
    );
    assert_eq!(
        claimed[0].after_ts,
        Some(946_684_800),
        "bug: the new walk takes its own floor"
    );
    assert_eq!(
        ledger
            .job_progress(user, tenant, PROVIDER, BackfillJobKind::HistoryWalk)
            .await
            .unwrap(),
        Some(JobProgress::default()),
        "bug: the new walk's progress starts at zero"
    );
}
