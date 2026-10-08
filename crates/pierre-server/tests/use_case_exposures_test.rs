// ABOUTME: carnet#828 — the per-athlete memory of which use-case starters were shown and tapped
// ABOUTME: A showing counts up, a tap resets the count and stamps the tap, and every read stays in its tenant
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use chrono::{Duration, SubsecRound, Utc};
use pierre_core::models::{TenantId, User, UserStatus};
use pierre_database::repositories::UseCaseExposure;
use pierre_database::RepositoryRegistry;
use pierre_test_support::db::create_test_db;
use uuid::Uuid;

async fn athlete(repos: &RepositoryRegistry) -> Uuid {
    let mut user = User::new(
        format!("athlete-{}@example.com", Uuid::new_v4()),
        "hash".to_owned(),
        Some("Athlete".to_owned()),
    );
    user.user_status = UserStatus::Active;
    let user_id = user.id;
    repos.users.create(&user).await.unwrap();
    user_id
}

fn find<'a>(exposures: &'a [UseCaseExposure], id: &str) -> &'a UseCaseExposure {
    exposures
        .iter()
        .find(|e| e.use_case_id == id)
        .unwrap_or_else(|| panic!("no exposure for {id}"))
}

#[tokio::test]
async fn each_showing_counts_and_a_tap_restarts_the_count() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let user = athlete(repos).await;
    let tenant = TenantId::generate();
    let first = Utc::now().trunc_subsecs(3) - Duration::hours(2);
    let second = first + Duration::hours(1);

    let exposures = &repos.use_case_exposures;
    exposures
        .record_use_cases_shown(tenant, user, &["recovered", "connect"], first)
        .await
        .unwrap();
    exposures
        .record_use_cases_shown(tenant, user, &["recovered"], second)
        .await
        .unwrap();

    let read = exposures.use_case_exposures(tenant, user).await.unwrap();
    assert_eq!(read.len(), 2);
    assert_eq!(find(&read, "recovered").shown_count, 2);
    assert_eq!(find(&read, "recovered").last_shown_at, Some(second));
    assert_eq!(find(&read, "connect").shown_count, 1);
    assert_eq!(find(&read, "connect").tapped_at, None);

    let tap = second + Duration::minutes(5);
    exposures
        .record_use_case_tapped(tenant, user, "recovered", tap)
        .await
        .unwrap();
    let read = exposures.use_case_exposures(tenant, user).await.unwrap();
    let recovered = find(&read, "recovered");
    assert_eq!(
        recovered.shown_count, 0,
        "a tap restarts the untapped count"
    );
    assert_eq!(recovered.tapped_at, Some(tap));
    assert_eq!(recovered.last_shown_at, Some(second));
}

#[tokio::test]
async fn a_tap_on_a_starter_never_counted_shown_is_recorded() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let user = athlete(repos).await;
    let tenant = TenantId::generate();
    let tap = Utc::now().trunc_subsecs(3);

    repos
        .use_case_exposures
        .record_use_case_tapped(tenant, user, "about_me", tap)
        .await
        .unwrap();

    let read = repos
        .use_case_exposures
        .use_case_exposures(tenant, user)
        .await
        .unwrap();
    assert_eq!(
        read,
        [UseCaseExposure {
            use_case_id: "about_me".to_owned(),
            shown_count: 0,
            last_shown_at: None,
            tapped_at: Some(tap),
        }]
    );
}

#[tokio::test]
async fn exposures_stay_in_their_tenant_and_with_their_athlete() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let (user, teammate) = (athlete(repos).await, athlete(repos).await);
    let (tenant, elsewhere) = (TenantId::generate(), TenantId::generate());
    let now = Utc::now();

    repos
        .use_case_exposures
        .record_use_cases_shown(tenant, user, &["load"], now)
        .await
        .unwrap();

    let exposures = &repos.use_case_exposures;
    assert!(exposures
        .use_case_exposures(elsewhere, user)
        .await
        .unwrap()
        .is_empty());
    assert!(exposures
        .use_case_exposures(tenant, teammate)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(
        exposures
            .use_case_exposures(tenant, user)
            .await
            .unwrap()
            .len(),
        1
    );
}
