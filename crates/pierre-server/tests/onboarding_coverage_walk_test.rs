// ABOUTME: Integration test for the full onboarding coverage walk across all 7 topics
// ABOUTME: Drives North Star + 6 pillars through the real Dossier composition + CoverageMap to completion
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use anyhow::Result;
#[cfg(feature = "postgresql")]
use pierre_config::environment::PostgresPoolConfig;
use pierre_core::models::{
    CoverageMap, CoverageTarget, GuidedFlow, OnboardingState, Pillar, TenantId, MAX_PROBE_ATTEMPTS,
};
use pierre_database::backends::factory::Database;
use pierre_database::database::generate_encryption_key;
use pierre_database::repositories::UpsertUserFactParams;
use pierre_database::RepositoryRegistry;
use pierre_memory::{FactKind, FactSource, MemoryScope};
use uuid::Uuid;

async fn open_in_memory_db() -> Result<Database> {
    let encryption_key = generate_encryption_key().to_vec();
    #[cfg(feature = "postgresql")]
    let db = Database::new(
        "sqlite::memory:",
        encryption_key,
        &PostgresPoolConfig::default(),
    )
    .await?;
    #[cfg(not(feature = "postgresql"))]
    let db = Database::new("sqlite::memory:", encryption_key).await?;
    Ok(db)
}

/// Persist one onboarding fact for a topic. North Star answers are stored as
/// `FactKind::NorthStar` (no pillar); pillar answers carry the probed pillar —
/// exactly how `stages::onboarding::extraction_params` stamps them.
async fn capture(
    repos: &RepositoryRegistry,
    tenant: TenantId,
    user: &str,
    kind: FactKind,
    pillar: Option<Pillar>,
    object: &str,
) -> Result<()> {
    repos
        .memory
        .upsert_user_fact(&UpsertUserFactParams {
            tenant_id: tenant,
            user_id: user,
            coach_id: None,
            scope: MemoryScope::User,
            kind,
            pillar,
            subject: "you",
            predicate: "have",
            object,
            confidence: 0.9,
            source: FactSource::Onboarding,
            valid_until: None,
            source_msg_id: None,
            embedding: None,
        })
        .await?;
    Ok(())
}

/// Re-derive the coverage map from the freshly composed dossier — the same
/// read-time derivation the pipeline and the onboarding-status endpoint use.
async fn coverage(repos: &RepositoryRegistry, tenant: TenantId, user: Uuid) -> Result<CoverageMap> {
    let dossier = repos.dossier.compose_dossier(tenant, user).await?;
    Ok(CoverageMap::from_dossier(&dossier))
}

/// Walk the guided onboarding through all seven topics (North Star + the six
/// pillars) against the real repository + Dossier composition, asserting the
/// coverage map advances one topic at a time, in canonical order, and reaches
/// completion. This is the deterministic stand-in for the LLM-driven
/// conversational walk: each "answer" is a persisted onboarding fact.
#[tokio::test]
async fn onboarding_walk_covers_all_seven_topics_in_order() -> Result<()> {
    let db = open_in_memory_db().await?;
    let repos = db.repositories();
    let tenant = TenantId::from_uuid(Uuid::new_v4());
    let user = Uuid::new_v4();
    let user_s = user.to_string();

    // Topic 0: nothing covered yet → probe the North Star first.
    let cov = coverage(&repos, tenant, user).await?;
    assert_eq!(cov.covered_count(), 0);
    assert!(!cov.is_complete());
    assert_eq!(cov.next_target(&[]), Some(CoverageTarget::NorthStar));

    // Answer the North Star → next probe is the first pillar in canonical order.
    capture(
        &repos,
        tenant,
        &user_s,
        FactKind::NorthStar,
        None,
        "be present and energetic for my kids",
    )
    .await?;
    let cov = coverage(&repos, tenant, user).await?;
    assert_eq!(cov.covered_count(), 1);
    assert_eq!(
        cov.next_target(&[]),
        Some(CoverageTarget::Pillar(Pillar::TrainingAndMovement))
    );

    // Walk the six pillars in canonical order. After each is answered, the next
    // target must be the next still-uncovered pillar — proving the self-healing
    // walk advances exactly one topic per turn and never repeats a covered one.
    let expected_next = [
        Some(CoverageTarget::Pillar(Pillar::Fuelling)),
        Some(CoverageTarget::Pillar(Pillar::SleepAndRecovery)),
        Some(CoverageTarget::Pillar(Pillar::MentalResilience)),
        Some(CoverageTarget::Pillar(Pillar::CommunityAndConnection)),
        Some(CoverageTarget::Pillar(Pillar::RecoveryOptimisation)),
        None, // all seven covered
    ];
    for (i, pillar) in Pillar::ALL.into_iter().enumerate() {
        capture(
            &repos,
            tenant,
            &user_s,
            FactKind::Goal,
            Some(pillar),
            &format!("a durable {} fact", pillar.as_str()),
        )
        .await?;
        let cov = coverage(&repos, tenant, user).await?;
        // North Star (1) + the pillars answered so far (i + 1).
        assert_eq!(
            cov.covered_count(),
            i + 2,
            "after answering {}, {} topics should be covered",
            pillar.as_str(),
            i + 2
        );
        assert_eq!(
            cov.next_target(&[]),
            expected_next[i],
            "wrong next probe after covering {}",
            pillar.as_str()
        );
    }

    // Terminal state: North Star + all six pillars covered.
    let cov = coverage(&repos, tenant, user).await?;
    assert!(cov.is_complete(), "onboarding should be complete");
    assert_eq!(cov.covered_count(), 7);
    assert_eq!(cov.next_target(&[]), None);

    Ok(())
}

/// A pillar covered only by a stale fact does not count — onboarding re-opens
/// that topic. Mirrors the 12-month PAR-Q / `/pillars full` re-screen behavior.
#[tokio::test]
async fn stale_pillar_fact_reopens_that_topic() -> Result<()> {
    let db = open_in_memory_db().await?;
    let repos = db.repositories();
    let tenant = TenantId::from_uuid(Uuid::new_v4());
    let user = Uuid::new_v4();
    let user_s = user.to_string();

    // Cover North Star + every pillar, then expire the Fuelling onboarding fact.
    capture(
        &repos,
        tenant,
        &user_s,
        FactKind::NorthStar,
        None,
        "my north star",
    )
    .await?;
    for pillar in Pillar::ALL {
        capture(
            &repos,
            tenant,
            &user_s,
            FactKind::Goal,
            Some(pillar),
            &format!("a {} fact", pillar.as_str()),
        )
        .await?;
    }
    assert!(coverage(&repos, tenant, user).await?.is_complete());

    // Supersede the Fuelling onboarding fact (sets valid_until in the past).
    let superseded = repos
        .memory
        .expire_onboarding_facts(tenant, &user_s, Some(Pillar::Fuelling), None)
        .await?;
    assert_eq!(superseded, 1);

    // Coverage drops by one and the walk re-targets the now-uncovered Fuelling.
    let cov = coverage(&repos, tenant, user).await?;
    assert!(
        !cov.is_complete(),
        "stale Fuelling fact must re-open onboarding"
    );
    assert_eq!(cov.covered_count(), 6);
    assert_eq!(
        cov.next_target(&[]),
        Some(CoverageTarget::Pillar(Pillar::Fuelling))
    );

    Ok(())
}

/// The delivered-probe history survives the `onboarding_state` column format and
/// drives the advance against a real Dossier-derived coverage map.
///
/// Coverage alone cannot express "already asked": fact extraction is
/// fire-and-forget, so on the turn right after an answer the Dossier still reads
/// uncovered. Without a persisted ask history the walk re-asks the question the
/// athlete just answered — the 2026-07-24 shape. With it, the walk moves on, and
/// a topic that never yields a fact still terminates after
/// `MAX_PROBE_ATTEMPTS`.
#[tokio::test]
async fn delivered_probe_history_round_trips_and_drives_the_advance() -> Result<()> {
    let db = open_in_memory_db().await?;
    let repos = db.repositories();
    let tenant = TenantId::from_uuid(Uuid::new_v4());
    let user = Uuid::new_v4();

    // Turn 1 delivered the North Star probe; nothing extracted yet.
    let state = OnboardingState::start(chrono::Utc::now().to_rfc3339(), GuidedFlow::Pillars)
        .with_delivered_probe(CoverageTarget::NorthStar.slug());
    let reloaded =
        OnboardingState::from_column(Some(&state.to_column()?)).expect("walk still active");
    assert_eq!(reloaded.probed.len(), 1);
    assert_eq!(reloaded.probed[0].as_str(), "north_star");

    // Coverage is still 0/7 — the answer's fact has not landed — yet the walk
    // advances to the first pillar rather than re-asking the North Star.
    let cov = coverage(&repos, tenant, user).await?;
    assert_eq!(cov.covered_count(), 0);
    assert_eq!(
        cov.next_target(&reloaded.probed),
        Some(CoverageTarget::Pillar(Pillar::TrainingAndMovement))
    );

    // Now burn every topic's attempt budget without a single fact landing: the
    // walk must terminate instead of looping forever on an unextractable answer.
    let mut exhausted = reloaded;
    for _ in 0..MAX_PROBE_ATTEMPTS {
        exhausted = exhausted.with_delivered_probe(CoverageTarget::NorthStar.slug());
        for pillar in Pillar::ALL {
            exhausted = exhausted.with_delivered_probe(CoverageTarget::Pillar(pillar).slug());
        }
    }
    let exhausted =
        OnboardingState::from_column(Some(&exhausted.to_column()?)).expect("walk still active");
    assert_eq!(
        cov.next_target(&exhausted.probed),
        None,
        "every topic out of attempts must end the walk"
    );

    // And the Dossier still tells the truth: nothing was actually captured, so
    // `/pillars <pillar>` can re-screen any of it later.
    assert!(!cov.is_complete());
    assert_eq!(cov.covered_count(), 0);
    Ok(())
}
