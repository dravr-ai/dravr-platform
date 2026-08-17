// ABOUTME: CommitmentRepository round-trip — insert dedupe, due scan, verdict + report transitions, tenant isolation
// ABOUTME: Proves the storage layer a commitment sweep depends on, including the racing-tick no-ops
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Integration tests for the athlete-commitment repository.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::sync::Arc;

use chrono::{Duration, Utc};
use pierre_database::repositories::SweptVerdict;
use pierre_database::RepositoryRegistry;
use pierre_memory::commitments::{Commitment, CommitmentOutcome, CommitmentStatus};

#[path = "helpers/db_fixtures.rs"]
mod db_fixtures;
use db_fixtures::create_test_db;

/// Build an open commitment due `days_out` days from now.
fn commitment(
    tenant: &str,
    user: &str,
    sport: Option<&str>,
    target: u32,
    days_out: i64,
) -> Commitment {
    let now = Utc::now();
    Commitment {
        id: uuid::Uuid::new_v4().to_string(),
        tenant_id: tenant.to_owned(),
        user_id: user.to_owned(),
        coach_id: Some("marathon-coach".to_owned()),
        conversation_id: Some("conv-1".to_owned()),
        statement: "three easy runs this week".to_owned(),
        sport: sport.map(str::to_owned),
        target_sessions: target,
        window_start: now,
        window_end: now + Duration::days(days_out),
        status: CommitmentStatus::Open,
        outcome: None,
        completed_sessions: None,
        swept_at: None,
        reported_at: None,
        created_at: now,
        updated_at: now,
    }
}

#[tokio::test]
async fn insert_round_trips_every_field() {
    let db = create_test_db().await;
    let repos: Arc<RepositoryRegistry> = Arc::new(db.repositories());

    let c = commitment("t1", "u1", Some("run"), 3, 7);
    assert!(repos.commitments.insert_commitment(&c).await.unwrap());

    let open = repos
        .commitments
        .list_open_commitments("t1", "u1", 10)
        .await
        .unwrap();
    assert_eq!(open.len(), 1);
    let got = &open[0];
    assert_eq!(got.id, c.id);
    assert_eq!(got.target_sessions, 3);
    assert_eq!(got.sport.as_deref(), Some("run"));
    assert_eq!(got.coach_id.as_deref(), Some("marathon-coach"));
    assert_eq!(got.conversation_id.as_deref(), Some("conv-1"));
    assert_eq!(got.statement, "three easy runs this week");
    assert_eq!(got.status, CommitmentStatus::Open);
    assert_eq!(got.outcome, None);
    assert_eq!(got.completed_sessions, None);
    assert_eq!(
        got.window_end.timestamp(),
        c.window_end.timestamp(),
        "the window survives the epoch-second round trip"
    );
}

#[tokio::test]
async fn absent_optional_fields_round_trip_as_none() {
    let db = create_test_db().await;
    let repos: Arc<RepositoryRegistry> = Arc::new(db.repositories());

    let mut c = commitment("t1", "u1", None, 2, 5);
    c.coach_id = None;
    c.conversation_id = None;
    assert!(repos.commitments.insert_commitment(&c).await.unwrap());

    let got = repos
        .commitments
        .list_open_commitments("t1", "u1", 10)
        .await
        .unwrap()
        .remove(0);
    // Stored as '' so the duplicate guard can compare them; they must not come
    // back as empty strings.
    assert_eq!(got.coach_id, None);
    assert_eq!(got.conversation_id, None);
    assert_eq!(got.sport, None);
}

#[tokio::test]
async fn reaffirming_the_same_promise_does_not_stack() {
    let db = create_test_db().await;
    let repos: Arc<RepositoryRegistry> = Arc::new(db.repositories());

    let first = commitment("t1", "u1", Some("run"), 3, 7);
    let mut same = first.clone();
    same.id = uuid::Uuid::new_v4().to_string();
    same.statement = "3 easy runs, this week".to_owned();

    assert!(repos.commitments.insert_commitment(&first).await.unwrap());
    assert!(
        !repos.commitments.insert_commitment(&same).await.unwrap(),
        "an identical open commitment is dropped, and says so"
    );
    assert_eq!(
        repos
            .commitments
            .list_open_commitments("t1", "u1", 10)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn a_different_target_is_a_different_promise() {
    let db = create_test_db().await;
    let repos: Arc<RepositoryRegistry> = Arc::new(db.repositories());

    let three_runs = commitment("t1", "u1", Some("run"), 3, 7);
    let two_swims = commitment("t1", "u1", Some("swim"), 2, 7);
    assert!(repos
        .commitments
        .insert_commitment(&three_runs)
        .await
        .unwrap());
    assert!(repos
        .commitments
        .insert_commitment(&two_swims)
        .await
        .unwrap());
    assert_eq!(
        repos
            .commitments
            .list_open_commitments("t1", "u1", 10)
            .await
            .unwrap()
            .len(),
        2
    );
}

#[tokio::test]
async fn due_scan_returns_only_closed_windows_oldest_first() {
    let db = create_test_db().await;
    let repos: Arc<RepositoryRegistry> = Arc::new(db.repositories());

    let overdue_older = commitment("t1", "u1", Some("run"), 3, -5);
    let overdue_newer = commitment("t1", "u1", Some("ride"), 2, -1);
    let still_open = commitment("t1", "u1", Some("swim"), 1, 3);
    for c in [&overdue_older, &overdue_newer, &still_open] {
        repos.commitments.insert_commitment(c).await.unwrap();
    }

    let due = repos
        .commitments
        .due_commitments(Utc::now().timestamp(), 50)
        .await
        .unwrap();
    assert_eq!(due.len(), 2, "a future window is not due");
    assert_eq!(due[0].id, overdue_older.id, "oldest first");
    assert_eq!(due[1].id, overdue_newer.id);
}

#[tokio::test]
async fn recording_a_verdict_moves_it_out_of_the_due_scan() {
    let db = create_test_db().await;
    let repos: Arc<RepositoryRegistry> = Arc::new(db.repositories());

    let c = commitment("t1", "u1", Some("run"), 3, -1);
    repos.commitments.insert_commitment(&c).await.unwrap();

    let at = Utc::now();
    let verdict = SweptVerdict {
        tenant_id: "t1",
        commitment_id: &c.id,
        outcome: CommitmentOutcome::Partial,
        completed_sessions: 2,
        at,
    };
    assert!(repos
        .commitments
        .record_commitment_verdict(&verdict)
        .await
        .unwrap());

    assert!(repos
        .commitments
        .due_commitments(Utc::now().timestamp(), 50)
        .await
        .unwrap()
        .is_empty());

    let pending = repos.commitments.unreported_commitments(50).await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].outcome, Some(CommitmentOutcome::Partial));
    assert_eq!(pending[0].completed_sessions, Some(2));
    assert_eq!(pending[0].status, CommitmentStatus::Labeled);
    assert_eq!(
        pending[0].swept_at.map(|t| t.timestamp()),
        Some(at.timestamp())
    );
}

#[tokio::test]
async fn a_racing_second_sweep_writes_nothing() {
    let db = create_test_db().await;
    let repos: Arc<RepositoryRegistry> = Arc::new(db.repositories());

    let c = commitment("t1", "u1", Some("run"), 3, -1);
    repos.commitments.insert_commitment(&c).await.unwrap();

    let met = SweptVerdict {
        tenant_id: "t1",
        commitment_id: &c.id,
        outcome: CommitmentOutcome::Met,
        completed_sessions: 3,
        at: Utc::now(),
    };
    let missed = SweptVerdict {
        outcome: CommitmentOutcome::Missed,
        completed_sessions: 0,
        ..met
    };
    assert!(repos
        .commitments
        .record_commitment_verdict(&met)
        .await
        .unwrap());
    assert!(
        !repos
            .commitments
            .record_commitment_verdict(&missed)
            .await
            .unwrap(),
        "the status predicate makes the second write a no-op, not an overwrite"
    );

    let pending = repos.commitments.unreported_commitments(50).await.unwrap();
    assert_eq!(pending[0].outcome, Some(CommitmentOutcome::Met));
    assert_eq!(pending[0].completed_sessions, Some(3));
}

#[tokio::test]
async fn reporting_is_single_shot_and_recorded() {
    let db = create_test_db().await;
    let repos: Arc<RepositoryRegistry> = Arc::new(db.repositories());

    let c = commitment("t1", "u1", Some("run"), 3, -1);
    repos.commitments.insert_commitment(&c).await.unwrap();
    repos
        .commitments
        .record_commitment_verdict(&SweptVerdict {
            tenant_id: "t1",
            commitment_id: &c.id,
            outcome: CommitmentOutcome::Met,
            completed_sessions: 3,
            at: Utc::now(),
        })
        .await
        .unwrap();

    assert!(repos
        .commitments
        .last_commitment_report("t1", "u1")
        .await
        .unwrap()
        .is_none());

    let at = Utc::now();
    assert!(repos
        .commitments
        .mark_commitment_reported("t1", &c.id, at)
        .await
        .unwrap());
    assert!(
        !repos
            .commitments
            .mark_commitment_reported("t1", &c.id, at)
            .await
            .unwrap(),
        "a second reporter pass over the same row delivers nothing"
    );

    assert!(repos
        .commitments
        .unreported_commitments(50)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(
        repos
            .commitments
            .last_commitment_report("t1", "u1")
            .await
            .unwrap()
            .map(|t| t.timestamp()),
        Some(at.timestamp()),
        "the cadence cap can see when this athlete last heard from us"
    );
}

#[tokio::test]
async fn cancelling_removes_it_from_every_scan() {
    let db = create_test_db().await;
    let repos: Arc<RepositoryRegistry> = Arc::new(db.repositories());

    let c = commitment("t1", "u1", Some("run"), 3, -1);
    repos.commitments.insert_commitment(&c).await.unwrap();

    assert!(repos
        .commitments
        .cancel_commitment("t1", &c.id)
        .await
        .unwrap());
    assert!(
        !repos
            .commitments
            .cancel_commitment("t1", &c.id)
            .await
            .unwrap(),
        "double-cancel returns false"
    );

    assert!(repos
        .commitments
        .due_commitments(Utc::now().timestamp(), 50)
        .await
        .unwrap()
        .is_empty());
    assert!(repos
        .commitments
        .list_open_commitments("t1", "u1", 10)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn expiring_closes_a_verdict_that_never_landed() {
    let db = create_test_db().await;
    let repos: Arc<RepositoryRegistry> = Arc::new(db.repositories());

    let c = commitment("t1", "u1", Some("run"), 3, -1);
    repos.commitments.insert_commitment(&c).await.unwrap();
    repos
        .commitments
        .record_commitment_verdict(&SweptVerdict {
            tenant_id: "t1",
            commitment_id: &c.id,
            outcome: CommitmentOutcome::Missed,
            completed_sessions: 0,
            at: Utc::now(),
        })
        .await
        .unwrap();

    assert!(repos
        .commitments
        .expire_commitment("t1", &c.id)
        .await
        .unwrap());
    assert!(repos
        .commitments
        .unreported_commitments(50)
        .await
        .unwrap()
        .is_empty());
    assert!(
        repos
            .commitments
            .last_commitment_report("t1", "u1")
            .await
            .unwrap()
            .is_none(),
        "an expired verdict never reached the athlete, so it must not spend their cadence"
    );
}

#[tokio::test]
async fn every_write_is_tenant_scoped() {
    let db = create_test_db().await;
    let repos: Arc<RepositoryRegistry> = Arc::new(db.repositories());

    let mine = commitment("t1", "u1", Some("run"), 3, -1);
    repos.commitments.insert_commitment(&mine).await.unwrap();

    // Another tenant knows the id but must not be able to touch the row.
    assert!(!repos
        .commitments
        .cancel_commitment("t2", &mine.id)
        .await
        .unwrap());
    assert!(!repos
        .commitments
        .expire_commitment("t2", &mine.id)
        .await
        .unwrap());
    assert!(!repos
        .commitments
        .record_commitment_verdict(&SweptVerdict {
            tenant_id: "t2",
            commitment_id: &mine.id,
            outcome: CommitmentOutcome::Missed,
            completed_sessions: 0,
            at: Utc::now(),
        })
        .await
        .unwrap());

    assert!(repos
        .commitments
        .list_open_commitments("t2", "u1", 10)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(
        repos
            .commitments
            .list_open_commitments("t1", "u1", 10)
            .await
            .unwrap()
            .len(),
        1,
        "the row survived every cross-tenant attempt"
    );
}

#[tokio::test]
async fn open_list_is_scoped_to_the_athlete() {
    let db = create_test_db().await;
    let repos: Arc<RepositoryRegistry> = Arc::new(db.repositories());

    repos
        .commitments
        .insert_commitment(&commitment("t1", "u1", Some("run"), 3, 7))
        .await
        .unwrap();
    repos
        .commitments
        .insert_commitment(&commitment("t1", "u2", Some("ride"), 2, 7))
        .await
        .unwrap();

    let mine = repos
        .commitments
        .list_open_commitments("t1", "u1", 10)
        .await
        .unwrap();
    assert_eq!(mine.len(), 1);
    assert_eq!(mine[0].sport.as_deref(), Some("run"));
}
