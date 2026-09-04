// ABOUTME: Endurance Phase 5 — WorkoutTemplateRepository round trip + multi-tenant isolation + slug lookup
// ABOUTME: Validates upsert / list_user / get_user_by_slug semantics on the SQLite tier; PG mirrors via the same trait
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::collections::HashMap;
use std::time::Duration;

use chrono::Utc;
use pierre_core::models::periodization::{
    Contraindication, EvidenceTier, ParamRange, PhaseFit, PhaseKind, Progression, ProgressionLever,
    ReadinessLevel, RpeRange, WorkoutParams, WorkoutPurpose,
};
use pierre_core::models::{
    IntensityDistribution, SportType, TenantId, WorkoutStep, WorkoutTargetZones, WorkoutTemplate,
};
use pierre_database::backends::factory::Database;
use pierre_database::database::test_utils::create_test_db_with_key;
use pierre_database::DatabaseProvider;
use tokio::time::sleep;
use uuid::Uuid;

async fn make_test_db() -> Database {
    let encryption_key = b"test_encryption_key_32_bytes_long".to_vec();
    let db = create_test_db_with_key(encryption_key)
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
        purpose: WorkoutPurpose::Endurance,
        sport_variants: Vec::new(),
        evidence_tier: EvidenceTier::CoachJudgement,
        caveat: Some("a user-authored steady run; no trial behind it".to_owned()),
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
        params: WorkoutParams::default(),
        progression: Progression::default(),
        fit: PhaseFit {
            phases: vec![PhaseKind::Base],
            readiness_min: ReadinessLevel::P1,
            max_per_week: 3,
            min_spacing_hours: 0,
            contraindications: Vec::new(),
        },
        evidence_refs: Vec::new(),
        is_compiled_in: false,
        updated_at: Utc::now(),
    }
}

#[tokio::test]
async fn upsert_then_list_round_trips() {
    let db = make_test_db().await;
    let tenant_id = TenantId::generate();
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
    let tenant_id = TenantId::generate();
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
    let tenant_id = TenantId::generate();
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
    let tenant_a = TenantId::generate();
    let tenant_b = TenantId::generate();
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
    let tenant_id = TenantId::generate();
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
    let tenant_id = TenantId::generate();
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

/// The catalogue layer of a template with nothing left at its default: a
/// purpose that does not follow from the intensity distribution, two sport
/// variants, a grey tier with its caveat, filled parameter ranges with an
/// intensity anchor per sport, a progression order, a phase fit and two
/// evidence references.
fn catalogue_template(tenant_id: TenantId, user_id: Uuid) -> WorkoutTemplate {
    let mut template = sample_template(tenant_id, user_id, "vo2_4x8_custom");
    template.intensity_distribution = IntensityDistribution::Threshold;
    template.purpose = WorkoutPurpose::Vo2maxLong;
    template.sport_variants = vec![SportType::Run, SportType::Ride];
    template.evidence_tier = EvidenceTier::Grey;
    template.caveat = Some("adapted from a club session; no trial of its own".to_owned());
    template.params = WorkoutParams {
        sets: None,
        reps: Some(ParamRange {
            min: 4,
            max: 5,
            default: 4,
        }),
        contacts: None,
        work_seconds: Some(ParamRange {
            min: 420,
            max: 600,
            default: 480,
        }),
        rest_seconds: Some(ParamRange {
            min: 90,
            max: 180,
            default: 120,
        }),
        duration_minutes: Some(ParamRange {
            min: 60,
            max: 75,
            default: 65,
        }),
        rpe: Some(RpeRange { min: 7, max: 8 }),
        load: None,
        intensity: HashMap::from([
            (SportType::Run, "Z5".to_owned()),
            (SportType::Ride, "100-105%".to_owned()),
        ]),
        intensity_label: Some("~90% HRmax".to_owned()),
    };
    template.progression = Progression {
        order: vec![
            ProgressionLever::AddRep,
            ProgressionLever::LengthenRep,
            ProgressionLever::ShortenRest,
        ],
        max_weekly_step: 2,
    };
    template.fit = PhaseFit {
        phases: vec![PhaseKind::Build, PhaseKind::Peak],
        readiness_min: ReadinessLevel::P1,
        max_per_week: 2,
        min_spacing_hours: 48,
        contraindications: vec![
            Contraindication::NoviceFirstSeason,
            Contraindication::AcuteInjury,
        ],
    };
    template.evidence_refs = vec![
        "evidence/sports_science/training_prescription/seiler-2013-interval-duration-4x8.md"
            .to_owned(),
        "evidence/sports_science/training_prescription/seiler-2010-best-practice.md".to_owned(),
    ];
    template
}

#[tokio::test]
async fn catalogue_fields_round_trip_through_their_own_columns() {
    let db = make_test_db().await;
    let tenant_id = TenantId::generate();
    let user_id = Uuid::new_v4();
    let template = catalogue_template(tenant_id, user_id);

    let repos = db.repositories();
    repos
        .workout_templates
        .upsert_workout_template(&template)
        .await
        .expect("upsert");

    let row = repos
        .workout_templates
        .get_user_workout_template(tenant_id, user_id, "vo2_4x8_custom")
        .await
        .expect("get")
        .expect("the row exists");

    assert_eq!(
        row.purpose,
        WorkoutPurpose::Vo2maxLong,
        "purpose is stored, not derived from the threshold distribution"
    );
    assert_eq!(row.sport_variants, vec![SportType::Run, SportType::Ride]);
    assert_eq!(row.evidence_tier, EvidenceTier::Grey);
    assert_eq!(
        row.caveat.as_deref(),
        Some("adapted from a club session; no trial of its own")
    );
    assert_eq!(row.params, template.params);
    assert_eq!(
        row.params.work_seconds.as_ref().map(|r| r.default),
        Some(480)
    );
    assert_eq!(row.params.intensity.len(), 2);
    assert_eq!(
        row.params
            .intensity
            .get(&SportType::Ride)
            .map(String::as_str),
        Some("100-105%")
    );
    assert_eq!(row.progression, template.progression);
    assert_eq!(row.progression.order.len(), 3);
    assert_eq!(row.progression.max_weekly_step, 2);
    assert_eq!(row.fit, template.fit);
    assert_eq!(row.fit.phases, vec![PhaseKind::Build, PhaseKind::Peak]);
    assert_eq!(row.fit.readiness_min, ReadinessLevel::P1);
    assert_eq!(row.fit.min_spacing_hours, 48);
    assert_eq!(row.evidence_refs, template.evidence_refs);
    assert_eq!(row.evidence_refs.len(), 2);

    let listed = repos
        .workout_templates
        .list_user_workout_templates(tenant_id, user_id)
        .await
        .expect("list");
    assert_eq!(listed.len(), 1);
    assert_eq!(
        listed[0].params, template.params,
        "list reads the same columns"
    );
    assert_eq!(listed[0].fit, template.fit);
}

#[tokio::test]
async fn catalogue_fields_written_at_default_read_back_as_default() {
    let db = make_test_db().await;
    let tenant_id = TenantId::generate();
    let user_id = Uuid::new_v4();
    let mut template = sample_template(tenant_id, user_id, "plain_steady");
    template.purpose = WorkoutPurpose::Endurance;
    template.sport_variants = Vec::new();
    template.evidence_tier = EvidenceTier::CoachJudgement;
    template.caveat = None;
    template.params = WorkoutParams::default();
    template.progression = Progression::default();
    template.fit = PhaseFit::default();
    template.evidence_refs = Vec::new();

    let repos = db.repositories();
    repos
        .workout_templates
        .upsert_workout_template(&template)
        .await
        .expect("upsert");

    let row = repos
        .workout_templates
        .get_user_workout_template(tenant_id, user_id, "plain_steady")
        .await
        .expect("get")
        .expect("the row exists");

    assert_eq!(row.purpose, WorkoutPurpose::Endurance);
    assert!(row.sport_variants.is_empty());
    assert_eq!(row.evidence_tier, EvidenceTier::CoachJudgement);
    assert_eq!(row.caveat, None);
    assert_eq!(row.params, WorkoutParams::default());
    assert_eq!(row.progression, Progression::default());
    assert_eq!(row.fit, PhaseFit::default());
    assert_eq!(row.fit.readiness_min, ReadinessLevel::P2);
    assert_eq!(row.fit.max_per_week, 7);
    assert!(row.evidence_refs.is_empty());
}

#[tokio::test]
async fn upsert_replaces_the_catalogue_fields_in_place() {
    let db = make_test_db().await;
    let tenant_id = TenantId::generate();
    let user_id = Uuid::new_v4();
    let mut template = catalogue_template(tenant_id, user_id);

    let repos = db.repositories();
    repos
        .workout_templates
        .upsert_workout_template(&template)
        .await
        .expect("upsert v1");

    template.purpose = WorkoutPurpose::Threshold;
    template.evidence_tier = EvidenceTier::Rct;
    template.caveat = None;
    template.fit.readiness_min = ReadinessLevel::P3;
    template
        .progression
        .order
        .push(ProgressionLever::RaiseIntensity);
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
    let row = &listed[0];
    assert_eq!(row.purpose, WorkoutPurpose::Threshold);
    assert_eq!(row.evidence_tier, EvidenceTier::Rct);
    assert_eq!(
        row.caveat, None,
        "a cleared caveat is cleared in the row too"
    );
    assert_eq!(row.fit.readiness_min, ReadinessLevel::P3);
    assert_eq!(row.progression.order.len(), 4);
    assert_eq!(row.progression.order[3], ProgressionLever::RaiseIntensity);
}
