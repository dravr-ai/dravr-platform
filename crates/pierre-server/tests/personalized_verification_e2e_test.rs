// ABOUTME: End-to-end test for the personalized-physiology layer — DB physiology + activities → snapshot → personalized verdict → persist
// ABOUTME: Exercises build_athlete_metrics over real repos (physiology, activities, synced nights), the verdicts, the round-trip
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use std::collections::BTreeSet;

use anyhow::Result;
use chrono::{Duration, NaiveDate, Utc};
use dravr_cageux::config::intelligence::AlgorithmConfig;
use dravr_cageux::config::intelligence::TrainingZonesConfig;
use pierre_core::ai_policy::Withheld;
use pierre_core::config::profiles::FitnessLevel;
use pierre_core::models::activity::ActivityBuilder;
use pierre_core::models::{
    Activity, DataSource, DeviceType, SportType, StoredRecoveryMetrics, StoredSleepSession,
    TenantId, ThresholdProvenance, UserPhysiologicalProfile,
};
use pierre_core::transport::{Transport, TransportPolicy};
use pierre_database::repositories::InsertClaimVerdictParams;
use pierre_database::RepositoryRegistry;
use pierre_evals::ClaimSource;
use pierre_evals::{
    check_claim, claim_extractor::ExtractedClaim, evidence_retriever::EvidenceCorpus,
    extract_heuristic, AthleteMetrics, ConservativeStrategy, PersonalizedContext, RecoveryReadings,
};
use pierre_memory::claims::{ClaimCategory, ClaimStatus, EvidenceStrength, VerdictLayer};
use pierre_providers::ai_scope::{ai_read, serve_over, with_ai_consent};
use pierre_providers::registry::global_registry;
use pierre_services::athlete_snapshot::build_athlete_metrics;
use uuid::Uuid;

fn tenant() -> TenantId {
    TenantId::from_uuid(Uuid::parse_str("00000000-0000-0000-0000-000000000042").unwrap())
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
        threshold_hr: None,
        age: Some(35),
        weight: Some(70.0),
        fitness_level: FitnessLevel::Intermediate,
        primary_sport: SportType::Run,
        training_experience_years: Some(8),
        ftp_watts: Some(250),
        threshold_pace_sec_per_km: Some(240.0),
        hr_zones: None,
        power_zones: None,
        critical_power_watts: None,
        w_prime_joules: None,
        critical_speed_mps: None,
        d_prime_meters: None,
        threshold_provenance: ThresholdProvenance::default(),
        transport_policy: TransportPolicy::AnyTransport,
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
    let metrics = build_athlete_metrics(
        repos,
        global_registry().as_ref(),
        &AlgorithmConfig::default(),
        &TrainingZonesConfig::default(),
        tenant_id,
        user_id,
    )
    .await;

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
        source: ClaimSource::Reply,
    };
    let outcome = check_claim(
        &claim,
        &[],
        &corpus(),
        EvidenceStrength::Mixed,
        Some(&ctx),
        None,
    );
    assert_eq!(outcome.layer_fired, VerdictLayer::Personalized);
    assert_eq!(outcome.status, ClaimStatus::Contradicted);

    // Persist the verdict (closes the loop and re-validates the migration CHECK).
    let params = InsertClaimVerdictParams {
        tenant_id,
        user_id: &user_id.to_string(),
        agent_id: None,
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
        transport_policy: TransportPolicy::AnyTransport,
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

    let metrics = build_athlete_metrics(
        repos,
        global_registry().as_ref(),
        &AlgorithmConfig::default(),
        &TrainingZonesConfig::default(),
        tenant(),
        user_id,
    )
    .await;
    assert!(!metrics.is_usable());
    assert_eq!(metrics.data_days, 0);
    assert_eq!(metrics.vdot, None);

    Ok(())
}

/// A WHOOP strap's night, 23:00 the evening before `day`.
fn whoop_night(
    user_id: Uuid,
    data_source_id: &str,
    day: NaiveDate,
    hours: u32,
) -> StoredSleepSession {
    let start = (day - Duration::days(1))
        .and_hms_opt(23, 0, 0)
        .unwrap()
        .and_utc();
    StoredSleepSession {
        id: Uuid::new_v4().to_string(),
        user_id: user_id.to_string(),
        data_source_id: data_source_id.to_owned(),
        is_nap: false,
        start_datetime: start,
        end_datetime: start + Duration::hours(i64::from(hours)),
        total_sleep_seconds: Some(hours * 3600),
        deep_sleep_seconds: None,
        light_sleep_seconds: None,
        rem_sleep_seconds: None,
        awake_seconds: None,
        sleep_efficiency: None,
        avg_heart_rate: None,
        min_heart_rate: None,
        avg_hrv: None,
        sleep_score: Some(40),
        stages: Vec::new(),
        source_name: "whoop".to_owned(),
    }
}

/// The WHOOP morning reading for `day`, with WHOOP's own recovery score on it.
fn whoop_morning(
    user_id: Uuid,
    data_source_id: &str,
    day: NaiveDate,
    hrv: f64,
) -> StoredRecoveryMetrics {
    StoredRecoveryMetrics {
        id: Uuid::new_v4().to_string(),
        user_id: user_id.to_string(),
        data_source_id: data_source_id.to_owned(),
        date: day,
        recovery_score: Some(30),
        readiness_score: None,
        hrv_ms: None,
        hrv_rmssd: Some(hrv),
        resting_heart_rate: Some(48),
        stress_score: None,
        body_battery: None,
        spo2: None,
        respiratory_rate: None,
        skin_temp_deviation: None,
        daily_strain: None,
        athlete_note: None,
        source_name: "whoop".to_owned(),
        recorded_at: day.and_hms_opt(8, 0, 0).unwrap().and_utc(),
    }
}

/// A week of WHOOP nights and mornings for `user_id`: 8 h a night and HRV
/// 60 ms, then 5 h and 51 ms last night.
async fn seed_whoop_week(
    repos: &RepositoryRegistry,
    tenant_id: TenantId,
    user_id: Uuid,
) -> Result<()> {
    let data_source_id = repos
        .data_sources
        .upsert_data_source(
            &tenant_id,
            &DataSource {
                id: String::new(),
                user_id: user_id.to_string(),
                provider: "whoop".to_owned(),
                device_model: None,
                software_version: None,
                source: None,
                device_type: DeviceType::Band,
                original_source_name: None,
            },
        )
        .await?;
    let today = Utc::now().date_naive();
    for back in 0..7 {
        let day = today - Duration::days(back);
        let (hours, hrv) = if back == 0 { (5, 51.0) } else { (8, 60.0) };
        repos
            .sleep
            .upsert_sleep_session(
                &tenant_id,
                &whoop_night(user_id, &data_source_id, day, hours),
            )
            .await?;
        repos
            .recovery
            .upsert_recovery_metrics(
                &tenant_id,
                &whoop_morning(user_id, &data_source_id, day, hrv),
            )
            .await?;
    }
    Ok(())
}

/// The snapshot a chat turn builds on Dravr's own surface: a read for a model,
/// under the athlete's AI consents, with `consent_withheld` the providers they
/// have not consented to hand to AI.
async fn turn_snapshot(
    repos: &RepositoryRegistry,
    tenant_id: TenantId,
    user_id: Uuid,
    consent_withheld: BTreeSet<String>,
) -> (AthleteMetrics, Withheld) {
    serve_over(
        Transport::WebApp,
        with_ai_consent(
            consent_withheld,
            ai_read(Box::pin(build_athlete_metrics(
                repos,
                global_registry().as_ref(),
                &AlgorithmConfig::default(),
                &TrainingZonesConfig::default(),
                tenant_id,
                user_id,
            ))),
        ),
    )
    .await
}

/// carnet#853: an athlete with no profile and no activities but a week of
/// WHOOP nights, who consented to WHOOP's AI use. Under the shipped provider
/// terms WHOOP's measurements reach the verifier on Dravr's own surface (its
/// scores do not), and a reply claiming a long night the strap did not record
/// is contradicted.
#[tokio::test]
async fn synced_nights_drive_a_personalized_sleep_contradiction() -> Result<()> {
    common::init_server_config();
    let database = common::create_test_database().await?;
    let repos = database.repositories();
    let tenant_id = tenant();
    let (user_id, _user) = common::create_test_user(&database).await?;
    seed_whoop_week(repos, tenant_id, user_id).await?;

    let (metrics, withheld) = turn_snapshot(repos, tenant_id, user_id, BTreeSet::new()).await;
    assert_eq!(
        withheld.dropped, 0,
        "WHOOP's measurements are allowed: {withheld:?}"
    );
    assert!(
        metrics.is_usable(),
        "synced nights alone make the layer usable"
    );
    assert_eq!(metrics.data_days, 0);
    let sleep = metrics.recovery.sleep_hours.expect("a week of nights");
    assert_eq!(sleep.last, Some(5.0));
    assert_eq!(sleep.prior_mean, Some(8.0));
    assert_eq!(metrics.recovery.hrv_ms.and_then(|h| h.last), Some(51.0));

    let strategy = ConservativeStrategy::default();
    let ctx = PersonalizedContext {
        metrics: &metrics,
        tolerance: &strategy,
    };
    let claims = extract_heuristic("You slept 8 hours last night. Your HRV dropped 15% this week.");
    assert_eq!(claims.len(), 2, "both sentences are extracted: {claims:?}");
    let verdicts: Vec<_> = claims
        .iter()
        .map(|claim| {
            check_claim(
                claim,
                &claims,
                &corpus(),
                EvidenceStrength::Mixed,
                Some(&ctx),
                None,
            )
        })
        .collect();
    assert_eq!(verdicts[0].status, ClaimStatus::Contradicted);
    assert_eq!(verdicts[0].layer_fired, VerdictLayer::Personalized);
    assert!(
        verdicts[0]
            .explanation
            .contains("sleep 8 h vs your 5 h last night"),
        "{}",
        verdicts[0].explanation
    );
    assert_eq!(verdicts[1].status, ClaimStatus::Supported);
    assert_eq!(verdicts[1].layer_fired, VerdictLayer::Personalized);

    let saved = repos
        .claim_verdicts
        .insert_claim_verdict(&InsertClaimVerdictParams {
            tenant_id,
            user_id: &user_id.to_string(),
            agent_id: None,
            conversation_id: None,
            message_id: None,
            claim_text: &claims[0].text,
            category: claims[0].category,
            status: verdicts[0].status,
            evidence_strength: verdicts[0].evidence_strength,
            confidence: verdicts[0].confidence,
            layer_fired: verdicts[0].layer_fired,
            explanation: Some(&verdicts[0].explanation),
            evidence_refs: verdicts[0].evidence_refs.as_deref(),
            transport_policy: TransportPolicy::FirstPartyOnly,
        })
        .await?;
    assert_eq!(saved.status, ClaimStatus::Contradicted);
    assert_eq!(saved.layer_fired, VerdictLayer::Personalized);

    Ok(())
}

/// carnet#853: the same week of WHOOP nights, but the athlete has withdrawn
/// their consent to WHOOP's AI use (carnet#726). The verifier reads what a
/// model may see, so none of WHOOP's nights reach it and a sleep figure is
/// not settled against them.
#[tokio::test]
async fn withdrawn_ai_consent_keeps_the_nights_from_the_verifier() -> Result<()> {
    common::init_server_config();
    let database = common::create_test_database().await?;
    let repos = database.repositories();
    let tenant_id = tenant();
    let (user_id, _user) = common::create_test_user(&database).await?;
    seed_whoop_week(repos, tenant_id, user_id).await?;

    let (metrics, withheld) = turn_snapshot(
        repos,
        tenant_id,
        user_id,
        BTreeSet::from(["whoop".to_owned()]),
    )
    .await;
    assert_eq!(
        withheld.dropped, 14,
        "WHOOP's seven nights and seven mornings: {withheld:?}"
    );
    assert_eq!(metrics.recovery, RecoveryReadings::default());
    assert!(
        !metrics.is_usable(),
        "nothing left to check a claim against"
    );

    let strategy = ConservativeStrategy::default();
    let ctx = PersonalizedContext {
        metrics: &metrics,
        tolerance: &strategy,
    };
    let claims = extract_heuristic("You slept 8 hours last night.");
    assert_eq!(claims.len(), 1, "{claims:?}");
    let verdict = check_claim(
        &claims[0],
        &claims,
        &corpus(),
        EvidenceStrength::Mixed,
        Some(&ctx),
        None,
    );
    assert_ne!(
        verdict.layer_fired,
        VerdictLayer::Personalized,
        "{verdict:?}"
    );
    assert_ne!(verdict.status, ClaimStatus::Contradicted, "{verdict:?}");

    Ok(())
}
