// ABOUTME: End-to-end test for the personalized-physiology layer — DB physiology + activities → snapshot → personalized verdict → persist
// ABOUTME: Exercises build_athlete_metrics over real repos, then the verdict pipeline, then the claim_verdicts round-trip
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use anyhow::Result;
use chrono::{Duration, Utc};
use pierre_core::config::profiles::FitnessLevel;
use pierre_core::models::activity::ActivityBuilder;
use pierre_core::models::{Activity, SportType, TenantId, UserPhysiologicalProfile};
use pierre_database::repositories::InsertClaimVerdictParams;
use pierre_evals::{
    check_claim, claim_extractor::ExtractedClaim, evidence_retriever::EvidenceCorpus,
    ConservativeStrategy, PersonalizedContext,
};
use pierre_intelligence::AlgorithmConfig;
use pierre_memory::claims::{ClaimCategory, ClaimStatus, EvidenceStrength, VerdictLayer};
use pierre_services::athlete_snapshot::build_athlete_metrics;
use uuid::Uuid;

fn tenant() -> TenantId {
    TenantId::from(Uuid::parse_str("00000000-0000-0000-0000-000000000042").unwrap())
}

fn synthetic_run(id: &str, days_ago: i64) -> Activity {
    ActivityBuilder::new(
        id.to_owned(),
        format!("synthetic {id}"),
        SportType::Run,
        Utc::now() - Duration::days(days_ago),
        3600,
        "synthetic".to_owned(),
    )
    .distance_meters(10_000.0)
    .average_heart_rate(150)
    .average_power(200)
    .normalized_power(205)
    .build()
}

fn profile(user_id: Uuid) -> UserPhysiologicalProfile {
    UserPhysiologicalProfile {
        user_id,
        vo2_max: Some(50.0),
        resting_hr: Some(48),
        max_hr: Some(192),
        lactate_threshold_percentage: Some(0.85),
        age: Some(35),
        weight: Some(70.0),
        fitness_level: FitnessLevel::Intermediate,
        primary_sport: SportType::Run,
        training_experience_years: Some(8),
        ftp_watts: Some(250),
        threshold_pace_sec_per_km: Some(240.0),
        hr_zones: None,
        power_zones: None,
    }
}

fn corpus() -> EvidenceCorpus {
    EvidenceCorpus::from_jsonl(
        r#"{"id":"doi:10.1/a","category":"nutrition","proposition":"Protein 1.6 g/kg/day","strength":"strong","citation":"Morton 2018"}"#,
    )
    .expect("parse corpus")
}

#[tokio::test]
async fn physiology_and_activities_drive_a_personalized_contradiction() -> Result<()> {
    common::init_server_config();
    let database = common::create_test_database().await?;
    let repos = database.repositories();
    let tenant_id = tenant();
    // A real user row — activity_cache.user_id has a FK to users(id).
    let (user_id, _user) = common::create_test_user(&database).await?;

    // Seed physiology + 16 distinct training days so the snapshot clears the
    // 14-day stale-data gate.
    repos
        .user_physiological_profile
        .upsert_user_physiological_profile(tenant_id, user_id, &profile(user_id))
        .await?;
    let activities: Vec<Activity> = (1..=16)
        .map(|d| synthetic_run(&format!("run-{d}"), d))
        .collect();
    repos
        .activity_cache
        .upsert_activities(user_id, &tenant_id, "strava", &activities)
        .await?;

    // Build the snapshot through the real service path.
    let metrics =
        build_athlete_metrics(&repos, &AlgorithmConfig::default(), tenant_id, user_id).await;

    assert!(
        metrics.is_usable(),
        "snapshot should clear the data gate, got data_days={}",
        metrics.data_days
    );
    assert_eq!(metrics.vdot, Some(50.0));
    assert_eq!(metrics.vo2max, Some(50.0));
    assert_eq!(metrics.max_hr, Some(192.0));
    assert_eq!(metrics.ftp_watts, Some(250.0));
    assert!(metrics.threshold_pace_range.is_some());
    assert!(metrics.data_days >= 14, "data_days={}", metrics.data_days);

    // Run the verdict pipeline: a 2:30/km threshold prescription is absurdly
    // fast for VO2max 50 → the personalized layer fires Contradicted before evidence.
    let strategy = ConservativeStrategy::default();
    let ctx = PersonalizedContext {
        metrics: &metrics,
        tolerance: &strategy,
    };
    let claim = ExtractedClaim {
        text: "Run your threshold pace at 2:30/km.".to_owned(),
        category: ClaimCategory::TrainingPrescription,
    };
    let outcome = check_claim(&claim, &[], &corpus(), EvidenceStrength::Mixed, Some(&ctx));
    assert_eq!(outcome.layer_fired, VerdictLayer::Personalized);
    assert_eq!(outcome.status, ClaimStatus::Contradicted);

    // Persist the verdict (closes the loop and re-validates the migration CHECK).
    let params = InsertClaimVerdictParams {
        tenant_id,
        user_id: &user_id.to_string(),
        coach_id: None,
        conversation_id: None,
        message_id: None,
        claim_text: &claim.text,
        category: claim.category,
        status: outcome.status,
        evidence_strength: outcome.evidence_strength,
        confidence: outcome.confidence,
        layer_fired: outcome.layer_fired,
        explanation: Some(&outcome.explanation),
        evidence_refs: outcome.evidence_refs.as_deref(),
    };
    let saved = repos.claim_verdicts.insert_claim_verdict(&params).await?;
    assert_eq!(saved.layer_fired, VerdictLayer::Personalized);
    assert_eq!(saved.status, ClaimStatus::Contradicted);

    Ok(())
}

#[tokio::test]
async fn missing_physiology_yields_unusable_snapshot() -> Result<()> {
    // No profile, no activities → snapshot is not usable → the personalized layer stays silent.
    common::init_server_config();
    let database = common::create_test_database().await?;
    let repos = database.repositories();
    let user_id = Uuid::parse_str("00000000-0000-0000-0000-0000000000bb").unwrap();

    let metrics =
        build_athlete_metrics(&repos, &AlgorithmConfig::default(), tenant(), user_id).await;
    assert!(!metrics.is_usable());
    assert_eq!(metrics.data_days, 0);
    assert_eq!(metrics.vdot, None);

    Ok(())
}
