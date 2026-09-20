// ABOUTME: Covers tenant- and user-scoped fitness configurations against whichever backend DATABASE_URL names
// ABOUTME: Pins the user-then-tenant read fallback, the upsert keeping its id, the listings and delete's true/false contract
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `fitness_configurations` holds a [`FitnessConfig`] per name at tenant
//! scope or for one user. Its statements are written once and emitted for
//! both backends, differing only in how a user id reaches the `user_id`
//! column — native uuid on `PostgreSQL`, `TEXT` on `SQLite`.
//!
//! The repository had no direct test on either backend: the fitness tools
//! call it, but nothing pinned the tenant fallback, the upsert's returned id
//! or `delete_config`'s return value. These run on `SQLite` and on
//! `PostgreSQL`: `create_test_db` opens whichever `DATABASE_URL` names, so
//! the same assertions cover both.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pierre_core::config::FitnessConfig;
use pierre_core::models::{Tenant, TenantId, User};
use pierre_database::database::test_utils::create_test_db;
use pierre_database::RepositoryRegistry;
use uuid::Uuid;

/// A distinct tenant and user per call, so one test's rows cannot satisfy
/// another's assertions; the table's foreign key needs the tenant to exist.
async fn fresh_tenant_and_user(repos: &RepositoryRegistry) -> (TenantId, String) {
    let user = User::new(
        format!("fitness-{}@example.com", Uuid::new_v4()),
        "argon2-hash-placeholder".to_owned(),
        Some("Fitness Config Tester".to_owned()),
    );
    let user_id = repos.users.create(&user).await.unwrap();
    let tenant = Tenant::new(
        "Fitness Tenant".to_owned(),
        format!("fitness-{}", Uuid::new_v4()),
        None,
        "starter".to_owned(),
        user_id,
    );
    repos.tenants.create(&tenant).await.unwrap();
    (tenant.id, user_id.to_string())
}

/// A config that differs from the default in a value the round trip can
/// be checked on: the light-effort ceiling.
fn config_with_light_max(light_max: f32) -> FitnessConfig {
    let mut config = FitnessConfig::default();
    config.intelligence.effort_thresholds.light_max = light_max;
    config
}

/// The light-effort ceiling a stored config came back with.
fn light_max(config: &FitnessConfig) -> f32 {
    config.intelligence.effort_thresholds.light_max
}

#[tokio::test]
async fn a_user_reads_their_own_config_and_falls_back_to_the_tenant_default() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let (tenant_id, user_id) = fresh_tenant_and_user(&repos).await;
    let repo = &repos.fitness_config;

    assert!(
        repo.get_user_config(tenant_id, &user_id, "default")
            .await
            .unwrap()
            .is_none(),
        "with nothing stored at either scope the read is None"
    );

    repo.save_tenant_config(tenant_id, "default", &config_with_light_max(0.210))
        .await
        .unwrap();
    let inherited = repo
        .get_user_config(tenant_id, &user_id, "default")
        .await
        .unwrap()
        .expect("a user with no row of their own reads the tenant default");
    assert!((light_max(&inherited) - 0.210).abs() < f32::EPSILON);

    repo.save_user_config(
        tenant_id,
        &user_id,
        "default",
        &config_with_light_max(0.255),
    )
    .await
    .unwrap();
    let own = repo
        .get_user_config(tenant_id, &user_id, "default")
        .await
        .unwrap()
        .expect("the user's own row wins over the tenant default");
    assert!((light_max(&own) - 0.255).abs() < f32::EPSILON);

    let tenant_level = repo
        .get_tenant_config(tenant_id, "default")
        .await
        .unwrap()
        .expect("the tenant read never sees a user row");
    assert!((light_max(&tenant_level) - 0.210).abs() < f32::EPSILON);
}

#[tokio::test]
async fn saving_the_same_name_again_updates_the_row_and_keeps_its_id() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let (tenant_id, user_id) = fresh_tenant_and_user(&repos).await;
    let repo = &repos.fitness_config;

    let first = repo
        .save_user_config(tenant_id, &user_id, "race", &config_with_light_max(0.240))
        .await
        .unwrap();
    let second = repo
        .save_user_config(tenant_id, &user_id, "race", &config_with_light_max(0.250))
        .await
        .unwrap();
    assert_eq!(
        first, second,
        "the upsert returns the existing row's id, not a fresh one"
    );

    let stored = repo
        .get_user_config(tenant_id, &user_id, "race")
        .await
        .unwrap()
        .unwrap();
    assert!(
        (light_max(&stored) - 0.250).abs() < f32::EPSILON,
        "the second save replaces the config"
    );
    assert_eq!(
        repo.list_user_configurations(tenant_id, &user_id)
            .await
            .unwrap(),
        vec!["race".to_owned()],
        "two saves of one name leave one listing"
    );
}

#[tokio::test]
async fn listings_are_scoped_and_sorted_by_name() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let (tenant_id, user_id) = fresh_tenant_and_user(&repos).await;
    let (other_tenant, _) = fresh_tenant_and_user(&repos).await;
    let repo = &repos.fitness_config;

    repo.save_tenant_config(tenant_id, "winter", &FitnessConfig::default())
        .await
        .unwrap();
    repo.save_tenant_config(tenant_id, "default", &FitnessConfig::default())
        .await
        .unwrap();
    repo.save_user_config(tenant_id, &user_id, "race", &FitnessConfig::default())
        .await
        .unwrap();
    repo.save_tenant_config(other_tenant, "elsewhere", &FitnessConfig::default())
        .await
        .unwrap();

    assert_eq!(
        repo.list_tenant_configurations(tenant_id).await.unwrap(),
        vec!["default".to_owned(), "race".to_owned(), "winter".to_owned()],
        "the tenant listing holds every name stored under the tenant, at either scope, sorted"
    );
    assert_eq!(
        repo.list_user_configurations(tenant_id, &user_id)
            .await
            .unwrap(),
        vec!["race".to_owned()],
        "the user listing holds only that user's rows"
    );
    assert_eq!(
        repo.list_tenant_configurations(other_tenant).await.unwrap(),
        vec!["elsewhere".to_owned()],
        "another tenant's rows never leak into the listing"
    );
}

#[tokio::test]
async fn delete_reports_whether_a_row_went_and_leaves_the_other_scope_alone() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let (tenant_id, user_id) = fresh_tenant_and_user(&repos).await;
    let repo = &repos.fitness_config;

    repo.save_tenant_config(tenant_id, "default", &config_with_light_max(0.200))
        .await
        .unwrap();
    repo.save_user_config(
        tenant_id,
        &user_id,
        "default",
        &config_with_light_max(0.260),
    )
    .await
    .unwrap();

    assert!(
        repo.delete_config(tenant_id, Some(&user_id), "default")
            .await
            .unwrap(),
        "deleting the user's row reports true"
    );
    assert!(
        !repo
            .delete_config(tenant_id, Some(&user_id), "default")
            .await
            .unwrap(),
        "deleting it again reports false"
    );
    let fallback = repo
        .get_user_config(tenant_id, &user_id, "default")
        .await
        .unwrap()
        .expect("the tenant default survives the user-scoped delete");
    assert!((light_max(&fallback) - 0.200).abs() < f32::EPSILON);

    assert!(
        repo.delete_config(tenant_id, None, "default")
            .await
            .unwrap(),
        "deleting the tenant row reports true"
    );
    assert!(
        repo.get_tenant_config(tenant_id, "default")
            .await
            .unwrap()
            .is_none(),
        "nothing is left at tenant scope"
    );
}

#[tokio::test]
async fn a_malformed_user_id_is_refused_on_every_operation() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let (tenant_id, _) = fresh_tenant_and_user(&repos).await;
    let repo = &repos.fitness_config;

    assert!(
        repo.get_user_config(tenant_id, "not-a-uuid", "default")
            .await
            .is_err(),
        "a user id that is not a uuid is refused rather than matching nothing"
    );
    assert!(repo
        .save_user_config(
            tenant_id,
            "not-a-uuid",
            "default",
            &FitnessConfig::default()
        )
        .await
        .is_err());
    assert!(repo
        .delete_config(tenant_id, Some("not-a-uuid"), "default")
        .await
        .is_err());
}
