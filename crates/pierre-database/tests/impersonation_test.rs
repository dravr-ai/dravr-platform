// ABOUTME: Covers the super-admin impersonation audit trail against whichever backend DATABASE_URL names
// ABOUTME: Round-trips a session, finds it by either party, lists with every filter, and ends it one way or all at once
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `impersonation_sessions` records which operator acted as which user and
//! when. Every read the admin handlers make — the active session for a
//! caller, one session by id, the filtered list — has to work on both
//! backends, and the ids that key those reads are `uuid` columns on
//! `PostgreSQL` and `TEXT` on `SQLite`.
//!
//! These run on `SQLite` and on `PostgreSQL`: `create_test_db` opens whichever
//! `DATABASE_URL` names, so the same assertions cover both.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pierre_core::models::User;
use pierre_core::permissions::impersonation::ImpersonationSession;
use pierre_database::database::test_utils::create_test_db;
use pierre_database::RepositoryRegistry;
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
    let operator = fresh_user(&repos).await;
    let target = fresh_user(&repos).await;

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

    let bystander = fresh_user(&repos).await;
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
    let operator = fresh_user(&repos).await;
    let target = fresh_user(&repos).await;

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
    let operator = fresh_user(&repos).await;
    let other_operator = fresh_user(&repos).await;
    let first_target = fresh_user(&repos).await;
    let second_target = fresh_user(&repos).await;

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

#[tokio::test]
async fn list_sessions_honours_every_filter() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let operator = fresh_user(&repos).await;
    let other_operator = fresh_user(&repos).await;
    let target = fresh_user(&repos).await;
    let other_target = fresh_user(&repos).await;

    let ended = ImpersonationSession::new(operator, target, Some("ended".to_owned()));
    repos.impersonation.create_session(&ended).await.unwrap();
    repos.impersonation.end_session(&ended.id).await.unwrap();
    let open = ImpersonationSession::new(operator, other_target, Some("open".to_owned()));
    repos.impersonation.create_session(&open).await.unwrap();
    let foreign = ImpersonationSession::new(other_operator, target, Some("foreign".to_owned()));
    repos.impersonation.create_session(&foreign).await.unwrap();

    let by_operator = repos
        .impersonation
        .list_sessions(Some(operator), None, false, 100)
        .await
        .unwrap();
    let mut ids: Vec<&str> = by_operator.iter().map(|s| s.id.as_str()).collect();
    ids.sort_unstable();
    let mut expected = vec![ended.id.as_str(), open.id.as_str()];
    expected.sort_unstable();
    assert_eq!(ids, expected, "the operator filter keeps both of theirs");

    let by_target = repos
        .impersonation
        .list_sessions(None, Some(target), false, 100)
        .await
        .unwrap();
    let mut ids: Vec<&str> = by_target.iter().map(|s| s.id.as_str()).collect();
    ids.sort_unstable();
    let mut expected = vec![ended.id.as_str(), foreign.id.as_str()];
    expected.sort_unstable();
    assert_eq!(ids, expected, "the target filter keeps both against them");

    let open_only = repos
        .impersonation
        .list_sessions(Some(operator), None, true, 100)
        .await
        .unwrap();
    assert_eq!(open_only.len(), 1);
    assert_eq!(open_only[0].id, open.id, "active_only drops the ended one");

    let both = repos
        .impersonation
        .list_sessions(Some(operator), Some(target), false, 100)
        .await
        .unwrap();
    assert_eq!(both.len(), 1);
    assert_eq!(both[0].id, ended.id, "both filters narrow to the one pair");

    let capped = repos
        .impersonation
        .list_sessions(Some(operator), None, false, 1)
        .await
        .unwrap();
    assert_eq!(capped.len(), 1, "limit caps the page");
    assert_eq!(capped[0].id, open.id, "newest first");
}
