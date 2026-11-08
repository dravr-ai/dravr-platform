// ABOUTME: Covers the super-admin impersonation audit trail against whichever backend DATABASE_URL names
// ABOUTME: Round-trips a session, finds it by either party, pages the whole log, and ends it one way or all at once
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `impersonation_sessions` records which operator acted as which user and
//! when. Every read the admin handlers make — the active session for a
//! caller, one session by id, a page of the log — has to work on both
//! backends, and the ids that key those reads are `uuid` columns on
//! `PostgreSQL` and `TEXT` on `SQLite`.
//!
//! These run on `SQLite` and on `PostgreSQL`: `create_test_db` opens whichever
//! `DATABASE_URL` names, so the same assertions cover both.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use chrono::{Duration, TimeZone, Utc};
use pierre_core::models::User;
use pierre_core::permissions::impersonation::ImpersonationSession;
use pierre_database::repositories::SessionCursor;
use pierre_database::RepositoryRegistry;
use pierre_test_support::db::create_test_db;
use uuid::Uuid;

/// A distinct user per call; the table's foreign keys need the rows to exist.
async fn fresh_user(repos: &RepositoryRegistry) -> Uuid {
    let user = User::new(
        format!("impersonation-{}@example.com", Uuid::new_v4()),
        "argon2-hash-placeholder".to_owned(),
        Some("Impersonation Tester".to_owned()),
    );
    repos.users.create(&user).await.unwrap()
}

#[tokio::test]
async fn a_session_reads_back_by_id_and_by_either_party() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let operator = fresh_user(repos).await;
    let target = fresh_user(repos).await;

    let session = ImpersonationSession::new(operator, target, Some("support ticket 42".to_owned()));
    repos.impersonation.create_session(&session).await.unwrap();

    let by_id = repos
        .impersonation
        .get_session(&session.id)
        .await
        .unwrap()
        .expect("the session just created must read back by id");
    assert_eq!(by_id.impersonator_id, operator);
    assert_eq!(by_id.target_user_id, target);
    assert_eq!(by_id.reason.as_deref(), Some("support ticket 42"));
    assert!(by_id.is_active);
    assert_eq!(by_id.ended_at, None);
    assert_eq!(
        by_id.started_at.timestamp_millis(),
        session.started_at.timestamp_millis(),
        "started_at survives the round-trip to the millisecond"
    );

    let for_operator = repos
        .impersonation
        .get_active_session(operator)
        .await
        .unwrap()
        .expect("the operator's active session must be found by their id");
    assert_eq!(for_operator.id, session.id);

    let for_target = repos
        .impersonation
        .get_active_session(target)
        .await
        .unwrap()
        .expect("the impersonated user's active session must be found by their id");
    assert_eq!(for_target.id, session.id);

    let bystander = fresh_user(repos).await;
    assert!(
        repos
            .impersonation
            .get_active_session(bystander)
            .await
            .unwrap()
            .is_none(),
        "a user in no session reads as None"
    );
}

#[tokio::test]
async fn ending_a_session_clears_it_from_the_active_lookup() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let operator = fresh_user(repos).await;
    let target = fresh_user(repos).await;

    let session = ImpersonationSession::new(operator, target, None);
    repos.impersonation.create_session(&session).await.unwrap();
    repos.impersonation.end_session(&session.id).await.unwrap();

    let ended = repos
        .impersonation
        .get_session(&session.id)
        .await
        .unwrap()
        .unwrap();
    assert!(!ended.is_active, "end_session flips the flag");
    assert!(ended.ended_at.is_some(), "end_session stamps the end");
    assert!(
        repos
            .impersonation
            .get_active_session(operator)
            .await
            .unwrap()
            .is_none(),
        "an ended session is no longer the operator's active one"
    );
}

#[tokio::test]
async fn end_all_sessions_counts_only_the_operators_open_ones() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let operator = fresh_user(repos).await;
    let other_operator = fresh_user(repos).await;
    let first_target = fresh_user(repos).await;
    let second_target = fresh_user(repos).await;

    for target in [first_target, second_target] {
        repos
            .impersonation
            .create_session(&ImpersonationSession::new(operator, target, None))
            .await
            .unwrap();
    }
    let untouched = ImpersonationSession::new(other_operator, first_target, None);
    repos
        .impersonation
        .create_session(&untouched)
        .await
        .unwrap();

    assert_eq!(
        repos
            .impersonation
            .end_all_sessions(operator)
            .await
            .unwrap(),
        2,
        "both of the operator's open sessions end, the other operator's does not"
    );
    assert_eq!(
        repos
            .impersonation
            .end_all_sessions(operator)
            .await
            .unwrap(),
        0,
        "a second sweep finds nothing open"
    );
    assert!(
        repos
            .impersonation
            .get_active_session(other_operator)
            .await
            .unwrap()
            .is_some(),
        "the other operator's session is still active"
    );
}

/// The log pages newest first by `(started_at, id)`, so two sessions that
/// started in the same instant on either side of a page boundary both appear,
/// and every session is reachable, however many there are.
#[tokio::test]
async fn the_session_log_pages_through_every_session_with_no_gap_at_a_tied_start() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let operator = fresh_user(repos).await;
    let target = fresh_user(repos).await;

    let tied_at = Utc.with_ymd_and_hms(2026, 9, 27, 9, 0, 0).unwrap();
    let mut written = Vec::new();
    for (n, started_at) in [
        tied_at - Duration::minutes(5),
        tied_at,
        tied_at,
        tied_at,
        tied_at + Duration::minutes(5),
    ]
    .into_iter()
    .enumerate()
    {
        let session = ImpersonationSession {
            id: format!("imp-page-{n}-{}", Uuid::new_v4()),
            started_at,
            created_at: started_at,
            ..ImpersonationSession::new(operator, target, None)
        };
        repos.impersonation.create_session(&session).await.unwrap();
        written.push(session.id);
    }

    let mut seen = Vec::new();
    let mut cursor: Option<SessionCursor> = None;
    loop {
        let page = repos
            .impersonation
            .list_sessions_page(cursor.as_ref(), 2)
            .await
            .unwrap();
        let mine: Vec<_> = page
            .iter()
            .filter(|s| s.impersonator_id == operator)
            .map(|s| s.id.clone())
            .collect();
        seen.extend(mine);
        let Some(last) = page.last() else { break };
        cursor = Some(SessionCursor {
            started_at: last.started_at,
            id: last.id.clone(),
        });
        if page.len() < 2 {
            break;
        }
    }

    let mut expected = written.clone();
    expected.sort_unstable();
    let mut got = seen.clone();
    got.sort_unstable();
    assert_eq!(
        got, expected,
        "bug: every session must be reachable, the tied ones included"
    );
    assert_eq!(seen.first(), written.last(), "newest first");
    assert_eq!(seen.last(), written.first(), "oldest last");
}
