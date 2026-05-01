// ABOUTME: Endurance Phase 5 — WorkoutTemplateRepository round trip + multi-tenant isolation + slug lookup
// ABOUTME: Validates upsert / list_user / get_user_by_slug semantics on the SQLite tier; PG mirrors via the same trait
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::time::Duration;

use chrono::Utc;
#[cfg(feature = "postgresql")]
use pierre_core::config::database::PostgresPoolConfig;
use pierre_core::models::{
    IntensityDistribution, SportType, TenantId, WorkoutStep, WorkoutTargetZones, WorkoutTemplate,
};
use pierre_database::backends::factory::Database;
use pierre_database::DatabaseProvider;
use tokio::time::sleep;
use uuid::Uuid;

async fn make_test_db() -> Database {
    let encryption_key = b"test_encryption_key_32_bytes_long".to_vec();
    #[cfg(feature = "postgresql")]
    let db = Database::new(
        "sqlite::memory:",
        encryption_key,
        &PostgresPoolConfig::default(),
    )
    .await
    .expect("create db");
    #[cfg(not(feature = "postgresql"))]
    let db = Database::new("sqlite::memory:", encryption_key)
        .await
        .expect("create db");
    db.migrate().await.expect("migrate");
    db
}

fn sample_template(tenant_id: TenantId, user_id: Uuid, slug: &str) -> WorkoutTemplate {
    WorkoutTemplate {
        id: Uuid::new_v4(),
        tenant_id: Some(tenant_id.as_uuid()),
        user_id: Some(user_id),
        slug: slug.to_owned(),
        name: format!("Custom — {slug}"),
        sport: SportType::Run,
        duration_minutes: 60,
        intensity_distribution: IntensityDistribution::Polarized,
        structure: vec![
            WorkoutStep {
                label: "Warm-up".to_owned(),
                duration_seconds: 600,
                distance_meters: None,
                target_zone: "Z1".to_owned(),
                repeat: 1,
                note: None,
            },
            WorkoutStep {
                label: "Steady".to_owned(),
                duration_seconds: 2400,
                distance_meters: None,
                target_zone: "Z2".to_owned(),
                repeat: 1,
                note: None,
            },
            WorkoutStep {
                label: "Cool-down".to_owned(),
                duration_seconds: 600,
                distance_meters: None,
                target_zone: "Z1".to_owned(),
                repeat: 1,
                note: None,
            },
        ],
        target_zones: WorkoutTargetZones {
            hr_pct_of_lt2: Some([0.65, 0.78, 0.85, 0.95, 1.05]),
            power_pct_of_ftp: None,
        },
        is_compiled_in: false,
        updated_at: Utc::now(),
    }
}

#[tokio::test]
async fn upsert_then_list_round_trips() {
    let db = make_test_db().await;
    let tenant_id = TenantId::new();
    let user_id = Uuid::new_v4();
    let template = sample_template(tenant_id, user_id, "custom_long_run");

    let repos = db.repositories();
    repos
        .workout_templates
        .upsert_workout_template(&template)
        .await
        .expect("upsert");

    let listed = repos
        .workout_templates
        .list_user_workout_templates(tenant_id, user_id)
        .await
        .expect("list");
    assert_eq!(listed.len(), 1, "exactly one user-authored row expected");
    let row = &listed[0];
    assert_eq!(row.slug, "custom_long_run");
    assert_eq!(row.name, "Custom — custom_long_run");
    assert_eq!(row.sport, SportType::Run);
    assert_eq!(row.duration_minutes, 60);
    assert!(matches!(
        row.intensity_distribution,
        IntensityDistribution::Polarized
    ));
    assert_eq!(row.structure.len(), 3);
    assert_eq!(row.structure[0].label, "Warm-up");
    assert!(row.target_zones.hr_pct_of_lt2.is_some());
    assert!(
        !row.is_compiled_in,
        "user-authored row must be flagged false"
    );
}

#[tokio::test]
async fn get_user_workout_template_returns_match_or_none() {
    let db = make_test_db().await;
    let tenant_id = TenantId::new();
    let user_id = Uuid::new_v4();
    let template = sample_template(tenant_id, user_id, "tempo_pyramid");

    let repos = db.repositories();
    repos
        .workout_templates
        .upsert_workout_template(&template)
        .await
        .expect("upsert");

    let hit = repos
        .workout_templates
        .get_user_workout_template(tenant_id, user_id, "tempo_pyramid")
        .await
        .expect("get hit");
    assert!(hit.is_some(), "lookup by slug must return the row");
    assert_eq!(hit.unwrap().slug, "tempo_pyramid");

    let miss = repos
        .workout_templates
        .get_user_workout_template(tenant_id, user_id, "not_a_real_slug")
        .await
        .expect("get miss");
    assert!(miss.is_none(), "unknown slug must return None");
}

#[tokio::test]
async fn upsert_with_same_id_updates_in_place() {
    let db = make_test_db().await;
    let tenant_id = TenantId::new();
    let user_id = Uuid::new_v4();
    let mut template = sample_template(tenant_id, user_id, "ramped_intervals");

    let repos = db.repositories();
    repos
        .workout_templates
        .upsert_workout_template(&template)
        .await
        .expect("upsert v1");

    template.duration_minutes = 90;
    template.name = "Ramped intervals — v2".to_owned();
    template.updated_at = Utc::now();
    repos
        .workout_templates
        .upsert_workout_template(&template)
        .await
        .expect("upsert v2");

    let listed = repos
        .workout_templates
        .list_user_workout_templates(tenant_id, user_id)
        .await
        .expect("list");
    assert_eq!(listed.len(), 1, "same-id upsert must not duplicate rows");
    assert_eq!(listed[0].duration_minutes, 90);
    assert_eq!(listed[0].name, "Ramped intervals — v2");
}

#[tokio::test]
async fn templates_are_tenant_and_user_scoped() {
    let db = make_test_db().await;
    let tenant_a = TenantId::new();
    let tenant_b = TenantId::new();
    let user_alpha = Uuid::new_v4();
    let user_beta = Uuid::new_v4();

    let repos = db.repositories();
    repos
        .workout_templates
        .upsert_workout_template(&sample_template(tenant_a, user_alpha, "scoped_a"))
        .await
        .expect("upsert a");
    repos
        .workout_templates
        .upsert_workout_template(&sample_template(tenant_b, user_alpha, "scoped_b"))
        .await
        .expect("upsert b");
    repos
        .workout_templates
        .upsert_workout_template(&sample_template(tenant_a, user_beta, "scoped_c"))
        .await
        .expect("upsert c");

    {
        let rows = repos
            .workout_templates
            .list_user_workout_templates(tenant_a, user_alpha)
            .await
            .expect("list tenant_a / user_alpha");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].slug, "scoped_a");
    }
    {
        let rows = repos
            .workout_templates
            .list_user_workout_templates(tenant_b, user_alpha)
            .await
            .expect("list tenant_b / user_alpha");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].slug, "scoped_b");
    }
    {
        let rows = repos
            .workout_templates
            .list_user_workout_templates(tenant_a, user_beta)
            .await
            .expect("list tenant_a / user_beta");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].slug, "scoped_c");
    }

    let other_user = Uuid::new_v4();
    let empty = repos
        .workout_templates
        .list_user_workout_templates(tenant_a, other_user)
        .await
        .expect("list tenant_a / unknown user");
    assert!(empty.is_empty(), "unknown user must return empty list");
}

#[tokio::test]
async fn list_orders_newest_first() {
    let db = make_test_db().await;
    let tenant_id = TenantId::new();
    let user_id = Uuid::new_v4();

    let repos = db.repositories();
    for i in 0..3 {
        let mut template = sample_template(tenant_id, user_id, &format!("ordered_{i}"));
        template.updated_at = Utc::now();
        repos
            .workout_templates
            .upsert_workout_template(&template)
            .await
            .expect("upsert ordered");
        // Keep updated_at strictly distinct across rows.
        sleep(Duration::from_millis(10)).await;
    }

    let listed = repos
        .workout_templates
        .list_user_workout_templates(tenant_id, user_id)
        .await
        .expect("list ordered");
    assert_eq!(listed.len(), 3);
    assert!(
        listed
            .windows(2)
            .all(|w| w[0].updated_at >= w[1].updated_at),
        "rows must be sorted by updated_at descending"
    );
    assert_eq!(listed[0].slug, "ordered_2");
    assert_eq!(listed[2].slug, "ordered_0");
}

#[tokio::test]
async fn upsert_rejects_missing_tenant_or_user() {
    let db = make_test_db().await;
    let tenant_id = TenantId::new();
    let user_id = Uuid::new_v4();

    let mut without_tenant = sample_template(tenant_id, user_id, "missing_tenant");
    without_tenant.tenant_id = None;
    let result = db
        .repositories()
        .workout_templates
        .upsert_workout_template(&without_tenant)
        .await;
    assert!(
        result.is_err(),
        "upsert must reject templates without a tenant_id (cornerstones belong in TOML)"
    );

    let mut without_user = sample_template(tenant_id, user_id, "missing_user");
    without_user.user_id = None;
    let result = db
        .repositories()
        .workout_templates
        .upsert_workout_template(&without_user)
        .await;
    assert!(
        result.is_err(),
        "upsert must reject templates without a user_id (cornerstones belong in TOML)"
    );
}
