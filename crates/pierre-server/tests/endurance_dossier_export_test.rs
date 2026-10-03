// ABOUTME: Endurance Phase 1 dossier export — multi-tenant isolation + composes with missing slots
// ABOUTME: Validates DossierRepository scoping and the read-time aggregation across physiology / goals / zones / nutrition / equipment
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use chrono::NaiveDate;
use pierre_core::config::profiles::FitnessLevel;
use pierre_core::models::zones::{HrZoneSet, PowerZoneSet};
use pierre_core::models::{
    MeasurementKind, MetricProvenance, ProvenancedValue, SportType, TenantId,
    UserPhysiologicalProfile,
};
use pierre_database::backends::factory::Database;
use pierre_database::DatabaseProvider;
use pierre_test_support::db::create_test_db_with_key;
use uuid::Uuid;

async fn make_test_db() -> Database {
    let encryption_key = b"test_encryption_key_32_bytes_long".to_vec();
    let db = create_test_db_with_key(encryption_key)
        .await
        .expect("create db");
    db.migrate().await.expect("migrate");
    db
}

fn make_profile(user_id: Uuid) -> UserPhysiologicalProfile {
    UserPhysiologicalProfile {
        user_id,
        vo2_max: Some(52.0),
        resting_hr: Some(50),
        max_hr: Some(190),
        lactate_threshold_percentage: Some(0.85),
        threshold_hr: None,
        age: Some(34),
        weight: Some(72.0),
        fitness_level: FitnessLevel::Advanced,
        primary_sport: SportType::Run,
        training_experience_years: Some(10),
        ftp_watts: Some(280),
        threshold_pace_sec_per_km: Some(225.0),
        hr_zones: Some(HrZoneSet::new(120, 140, 160, 180, 200).unwrap()),
        power_zones: Some(PowerZoneSet::new(150, 200, 240, 280, 320).unwrap()),
        critical_power_watts: None,
        w_prime_joules: None,
        critical_speed_mps: None,
        d_prime_meters: None,
    }
}

#[tokio::test]
async fn dossier_with_no_underlying_data_returns_empty_shell() {
    let db = make_test_db().await;
    let tenant_id = TenantId::generate();
    let user_id = Uuid::new_v4();
    let dossier = db
        .repositories()
        .dossier
        .compose_dossier(tenant_id, user_id)
        .await
        .expect("compose");
    assert_eq!(dossier.user_id, user_id);
    assert!(dossier.physiology.is_none());
    assert!(dossier.hr_zones.is_none());
    assert!(dossier.power_zones.is_none());
    assert!(dossier.goals.is_empty());
    assert!(dossier.nutrition.is_none());
    assert!(dossier.equipment.is_none());
}

#[tokio::test]
async fn dossier_pulls_physiology_and_zones_when_present() {
    let db = make_test_db().await;
    let tenant_id = TenantId::generate();
    let user_id = Uuid::new_v4();
    let repos = db.repositories();
    repos
        .user_physiological_profile
        .upsert_user_physiological_profile(tenant_id, user_id, &make_profile(user_id))
        .await
        .expect("upsert physiology");
    let dossier = repos
        .dossier
        .compose_dossier(tenant_id, user_id)
        .await
        .expect("compose");
    assert_eq!(
        dossier.physiology.as_ref().and_then(|p| p.ftp_watts),
        Some(280)
    );
    assert!(dossier.hr_zones.is_some());
    assert!(dossier.power_zones.is_some());
}

#[tokio::test]
async fn dossier_is_tenant_scoped() {
    let db = make_test_db().await;
    let tenant_a = TenantId::generate();
    let tenant_b = TenantId::generate();
    let user_id = Uuid::new_v4();

    let mut profile_a = make_profile(user_id);
    profile_a.ftp_watts = Some(250);
    let mut profile_b = make_profile(user_id);
    profile_b.ftp_watts = Some(310);

    let repos = db.repositories();
    repos
        .user_physiological_profile
        .upsert_user_physiological_profile(tenant_a, user_id, &profile_a)
        .await
        .expect("upsert A");
    repos
        .user_physiological_profile
        .upsert_user_physiological_profile(tenant_b, user_id, &profile_b)
        .await
        .expect("upsert B");

    let dossier_a = repos
        .dossier
        .compose_dossier(tenant_a, user_id)
        .await
        .expect("compose A");
    let dossier_b = repos
        .dossier
        .compose_dossier(tenant_b, user_id)
        .await
        .expect("compose B");
    assert_eq!(dossier_a.physiology.unwrap().ftp_watts, Some(250));
    assert_eq!(dossier_b.physiology.unwrap().ftp_watts, Some(310));
    assert_eq!(dossier_a.tenant_id, tenant_a.as_uuid());
    assert_eq!(dossier_b.tenant_id, tenant_b.as_uuid());
}

#[tokio::test]
async fn dossier_compose_tolerates_missing_goals_and_nutrition() {
    // The Endurance dossier slots for goals / nutrition / equipment are
    // optional. When the user has no rows in `user_profiles` and no goals
    // recorded, the composer must still return a 200 with empty / None
    // slots rather than failing — this guards endpoint UX for fresh accounts.
    let db = make_test_db().await;
    let tenant_id = TenantId::generate();
    let user_id = Uuid::new_v4();
    db.repositories()
        .user_physiological_profile
        .upsert_user_physiological_profile(tenant_id, user_id, &make_profile(user_id))
        .await
        .expect("upsert physiology");
    let dossier = db
        .repositories()
        .dossier
        .compose_dossier(tenant_id, user_id)
        .await
        .expect("compose");
    assert!(dossier.physiology.is_some());
    assert!(
        dossier.goals.is_empty(),
        "no goals seeded → dossier.goals must be empty"
    );
    assert!(
        dossier.nutrition.is_none(),
        "no profile json → nutrition slot must be None"
    );
    assert!(
        dossier.equipment.is_none(),
        "no profile json → equipment slot must be None"
    );
}

#[tokio::test]
async fn dossier_serialises_to_endurance_conformant_json() {
    let db = make_test_db().await;
    let tenant_id = TenantId::generate();
    let user_id = Uuid::new_v4();
    db.repositories()
        .user_physiological_profile
        .upsert_user_physiological_profile(tenant_id, user_id, &make_profile(user_id))
        .await
        .expect("upsert");

    let dossier = db
        .repositories()
        .dossier
        .compose_dossier(tenant_id, user_id)
        .await
        .expect("compose");
    let json = serde_json::to_value(&dossier).expect("serialize");
    let obj = json.as_object().expect("object root");
    for key in [
        "user_id",
        "tenant_id",
        "physiology",
        "hr_zones",
        "power_zones",
    ] {
        assert!(obj.contains_key(key), "missing key: {key}");
    }
}

/// carnet#714: the dossier is how the agent reads physiology, so a critical
/// power reaches it with its kind and origin in the same object — the agent
/// cannot read the number without reading that it is an estimate.
#[tokio::test]
async fn dossier_carries_critical_power_with_its_provenance() {
    let db = make_test_db().await;
    let tenant_id = TenantId::generate();
    let user_id = Uuid::new_v4();
    let mut profile = make_profile(user_id);
    profile.critical_power_watts = Some(ProvenancedValue::new(
        312,
        MetricProvenance {
            kind: MeasurementKind::Estimated,
            origin: Some("vekta".to_owned()),
            as_of: NaiveDate::from_ymd_opt(2026, 10, 1),
        },
    ));
    let repos = db.repositories();
    repos
        .user_physiological_profile
        .upsert_user_physiological_profile(tenant_id, user_id, &profile)
        .await
        .expect("upsert");

    let dossier = repos
        .dossier
        .compose_dossier(tenant_id, user_id)
        .await
        .expect("compose");
    let json = serde_json::to_value(&dossier).expect("serialize");
    assert_eq!(
        json["physiology"]["critical_power_watts"],
        serde_json::json!({"value": 312, "kind": "estimated", "origin": "vekta", "as_of": "2026-10-01"})
    );
    assert!(
        json["physiology"].get("w_prime_joules").is_none(),
        "an absent value is absent, not a zero"
    );
}
