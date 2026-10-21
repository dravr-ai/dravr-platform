// ABOUTME: Covers the first-run done store and the check-in landing read against whichever backend DATABASE_URL names
// ABOUTME: A day is written once, reads back oldest first, and neither table leaks across tenants or users (carnet#1099)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `first_run_progress` records the days an athlete met a first-run flow's
//! done rule, and `platform_post_claims.posted_at_ms` tells the daily
//! check-in flow when that day's check-in landed. Both statements are written
//! once and emitted for both backends; `create_test_db` opens whichever
//! `DATABASE_URL` names, so the same assertions cover `SQLite` and
//! `PostgreSQL`.

use chrono::{DateTime, Duration, NaiveDate, TimeZone, Utc};
use pierre_core::models::TenantId;
use pierre_database::repositories::FirstRunDay;
use pierre_database::RepositoryRegistry;
use pierre_test_support::db::create_test_db;
use pierre_test_support::server::create_test_user;
use uuid::Uuid;

/// A distinct user per call; the table's foreign key needs the row.
async fn fresh_user(repos: &RepositoryRegistry) -> Uuid {
    let user = create_test_user(
        &format!("first-run-{}@example.com", Uuid::new_v4()),
        Some("First Run Tester".to_owned()),
    );
    repos.users.create(&user).await.unwrap()
}

fn day(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, d).unwrap()
}

fn at(d: u32, hour: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, d, hour, 30, 0).unwrap()
}

#[tokio::test]
async fn an_athlete_with_no_progress_reads_no_days() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let user_id = fresh_user(repos).await;

    let days = repos
        .first_run_progress
        .first_run_days(TenantId::generate(), user_id)
        .await
        .unwrap();
    assert_eq!(days, Vec::<FirstRunDay>::new());
}

#[tokio::test]
async fn a_day_is_written_once_and_reads_back_oldest_first() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let progress = &repos.first_run_progress;
    let user_id = fresh_user(repos).await;
    let tenant = TenantId::generate();

    assert!(
        progress
            .record_first_run_day(tenant, user_id, "check_in", day(13), at(13, 9))
            .await
            .unwrap(),
        "the first row of a day is written"
    );
    assert!(
        !progress
            .record_first_run_day(tenant, user_id, "check_in", day(13), at(13, 18))
            .await
            .unwrap(),
        "a second write of the same flow and day changes nothing"
    );
    assert!(progress
        .record_first_run_day(tenant, user_id, "meet_team", day(12), at(12, 7))
        .await
        .unwrap());
    assert!(progress
        .record_first_run_day(tenant, user_id, "check_in", day(14), at(14, 9))
        .await
        .unwrap());

    let days = progress.first_run_days(tenant, user_id).await.unwrap();
    let read: Vec<(&str, NaiveDate, DateTime<Utc>)> = days
        .iter()
        .map(|d| (d.flow_id.as_str(), d.local_date, d.recorded_at))
        .collect();
    assert_eq!(
        read,
        vec![
            ("meet_team", day(12), at(12, 7)),
            ("check_in", day(13), at(13, 9)),
            ("check_in", day(14), at(14, 9)),
        ],
        "rows read oldest first, and the kept row is the first write's instant"
    );
}

#[tokio::test]
async fn progress_never_crosses_a_tenant_or_a_user() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let progress = &repos.first_run_progress;
    let user_id = fresh_user(repos).await;
    let other_user = fresh_user(repos).await;
    let (tenant, other_tenant) = (TenantId::generate(), TenantId::generate());

    assert!(progress
        .record_first_run_day(tenant, user_id, "last_workout", day(15), at(15, 10))
        .await
        .unwrap());
    // The same flow and day in another tenant is its own row.
    assert!(progress
        .record_first_run_day(other_tenant, user_id, "last_workout", day(15), at(15, 10))
        .await
        .unwrap());

    assert_eq!(
        progress
            .first_run_days(tenant, user_id)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        progress.first_run_days(tenant, other_user).await.unwrap(),
        Vec::<FirstRunDay>::new()
    );
}

#[tokio::test]
async fn the_check_in_landing_reads_back_per_tenant() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let claims = &repos.platform_post_claims;
    let user_id = fresh_user(repos).await;
    let (tenant, other_tenant) = (TenantId::generate(), TenantId::generate());
    let morning = at(16, 8);

    assert_eq!(
        claims
            .platform_post_posted_at(tenant, user_id, "check_in")
            .await
            .unwrap(),
        None,
        "no claim reads as never posted"
    );

    let lease_token = claims
        .claim_platform_post_since(tenant, user_id, "check_in", at(16, 0), morning)
        .await
        .unwrap()
        .expect("the first claim of the day takes the post");
    assert_eq!(
        claims
            .platform_post_posted_at(tenant, user_id, "check_in")
            .await
            .unwrap(),
        None,
        "a claim whose post has not landed reads as not posted"
    );

    let landed = morning + Duration::seconds(3);
    assert!(claims
        .mark_platform_post_posted(tenant, user_id, "check_in", &lease_token, landed)
        .await
        .unwrap());
    assert_eq!(
        claims
            .platform_post_posted_at(tenant, user_id, "check_in")
            .await
            .unwrap(),
        Some(landed)
    );
    assert_eq!(
        claims
            .platform_post_posted_at(other_tenant, user_id, "check_in")
            .await
            .unwrap(),
        None,
        "another tenant never reads this tenant's post"
    );
}
