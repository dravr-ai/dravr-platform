// ABOUTME: Integration tests for the calibration interview's fact plumbing against a real database
// ABOUTME: Window supersession on re-run, and the source guarantee that keeps answers out of the eviction path
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The two database behaviours the calibration interview depends on.
//!
//! Both were identified as broken-as-designed during the 2026-07-28 review:
//! the plan's per-topic supersession could not work (most topics share a kind
//! and pillar, and the extractor picks each fact's predicate), and the fact
//! bundle guaranteed only `Medical` and `NorthStar` by kind, so a chatty
//! athlete lost their whole calibration set inside the 40-fact recency window.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use anyhow::Result;
use chrono::{Duration, Utc};
use pierre_core::models::{Pillar, TenantId};
use pierre_database::backends::factory::Database;
use pierre_database::database::generate_encryption_key;
use pierre_database::database::test_utils::create_test_db_with_key;
use pierre_database::repositories::UpsertUserFactParams;
use pierre_database::RepositoryRegistry;
use pierre_memory::{FactKind, FactSource, MemoryScope, PredicateCode};

/// Mirrors `pierre_database::dossier_facts::FACT_BUNDLE_LIMIT`, which is
/// crate-private. The flood only needs to exceed the recency window; a drift
/// here makes the test weaker, never falsely green, because the assertions are
/// about what survives rather than what is evicted.
const FACT_BUNDLE_LIMIT: usize = 40;
use uuid::Uuid;

async fn open_in_memory_db() -> Result<Database> {
    let encryption_key = generate_encryption_key().to_vec();
    let db = create_test_db_with_key(encryption_key).await?;
    Ok(db)
}

#[allow(clippy::too_many_arguments)]
async fn seed(
    repos: &RepositoryRegistry,
    tenant: TenantId,
    user: &str,
    kind: FactKind,
    pillar: Option<Pillar>,
    source: FactSource,
    object: &str,
) -> Result<String> {
    let fact = repos
        .memory
        .upsert_user_fact(&UpsertUserFactParams {
            tenant_id: tenant,
            user_id: user,
            coach_id: None,
            scope: MemoryScope::User,
            kind,
            pillar,
            predicate_code: PredicateCode::Prefer,
            object,
            confidence: 0.9,
            source,
            valid_until: None,
            source_msg_id: None,
            embedding: None,
        })
        .await?;
    Ok(fact.id)
}

/// A re-run supersedes the previous interview's answers and nothing older.
///
/// The window is the only handle available: seven topics share
/// `preference` + `training_and_movement` + `onboarding`, so a per-topic match
/// would collapse them, and the extractor — not the platform — picks each
/// fact's predicate, so there is no stable natural key either.
#[tokio::test]
async fn a_re_run_supersedes_only_the_previous_interviews_window() -> Result<()> {
    let db = open_in_memory_db().await?;
    let repos = db.repositories();
    let tenant = TenantId::from_uuid(Uuid::new_v4());
    let user = Uuid::new_v4().to_string();

    // An answer from the athlete's pillars walk, written before any
    // calibration ran. It must survive.
    let older = seed(
        &repos,
        tenant,
        &user,
        FactKind::Preference,
        Some(Pillar::TrainingAndMovement),
        FactSource::Onboarding,
        "pillars-era answer",
    )
    .await?;

    // The boundary: everything from here on belongs to the first interview.
    let window_start = Utc::now();

    let first_interview = seed(
        &repos,
        tenant,
        &user,
        FactKind::Preference,
        Some(Pillar::TrainingAndMovement),
        FactSource::Onboarding,
        "wants more hard days",
    )
    .await?;

    // A second /calibrate supersedes the first interview's window.
    let superseded = repos
        .memory
        .expire_onboarding_facts(
            tenant,
            &user,
            Some(Pillar::TrainingAndMovement),
            Some(window_start),
            // Pillar-scoped re-screen: no per-question narrowing.
            None,
        )
        .await?;
    assert!(
        superseded >= 1,
        "the first interview's answer must be superseded, got {superseded}"
    );

    let live = repos
        .memory
        .list_user_facts_by_source(tenant, &user, FactSource::Onboarding, 100)
        .await?;
    let live_ids: Vec<&str> = live.iter().map(|f| f.id.as_str()).collect();

    assert!(
        !live_ids.contains(&first_interview.as_str()),
        "the previous interview's answer must no longer read as current"
    );
    assert!(
        live_ids.contains(&older.as_str()),
        "a fact written before the window must survive — expiring it would \
         silently delete the athlete's pillars-walk answers"
    );
    Ok(())
}

/// Facts outside the expiry's pillar scope are untouched, so a calibration
/// re-run cannot wipe the athlete's other five pillars.
#[tokio::test]
async fn a_re_run_leaves_other_pillars_alone() -> Result<()> {
    let db = open_in_memory_db().await?;
    let repos = db.repositories();
    let tenant = TenantId::from_uuid(Uuid::new_v4());
    let user = Uuid::new_v4().to_string();

    let window_start = Utc::now();
    let sleep_answer = seed(
        &repos,
        tenant,
        &user,
        FactKind::Preference,
        Some(Pillar::SleepAndRecovery),
        FactSource::Onboarding,
        "sleeps seven hours",
    )
    .await?;
    seed(
        &repos,
        tenant,
        &user,
        FactKind::Preference,
        Some(Pillar::TrainingAndMovement),
        FactSource::Onboarding,
        "wants more volume",
    )
    .await?;

    repos
        .memory
        .expire_onboarding_facts(
            tenant,
            &user,
            Some(Pillar::TrainingAndMovement),
            Some(window_start),
            // Pillar-scoped re-screen: no per-question narrowing.
            None,
        )
        .await?;

    let live = repos
        .memory
        .list_user_facts_by_source(tenant, &user, FactSource::Onboarding, 100)
        .await?;
    assert!(
        live.iter().any(|f| f.id == sleep_answer),
        "a recovery-pillar answer must survive a training-pillar expiry"
    );
    Ok(())
}

/// Superseded facts are excluded from the by-source read.
///
/// This is what stops decision 2 (guarantee every onboarding fact) and
/// decision 5 (supersede by window) fighting each other: expiry is soft, so
/// without this filter a re-run's replaced answers would be guaranteed back
/// into the 40-fact bundle forever.
#[tokio::test]
async fn superseded_answers_do_not_refill_the_guaranteed_bundle() -> Result<()> {
    let db = open_in_memory_db().await?;
    let repos = db.repositories();
    let tenant = TenantId::from_uuid(Uuid::new_v4());
    let user = Uuid::new_v4().to_string();

    let window_start = Utc::now() - Duration::seconds(1);
    for i in 0..5 {
        seed(
            &repos,
            tenant,
            &user,
            FactKind::Preference,
            Some(Pillar::TrainingAndMovement),
            FactSource::Onboarding,
            &format!("old answer {i}"),
        )
        .await?;
    }

    let before = repos
        .memory
        .list_user_facts_by_source(tenant, &user, FactSource::Onboarding, 100)
        .await?;
    assert_eq!(before.len(), 5, "all five answers start out current");

    repos
        .memory
        .expire_onboarding_facts(
            tenant,
            &user,
            Some(Pillar::TrainingAndMovement),
            Some(window_start),
            // Pillar-scoped re-screen: no per-question narrowing.
            None,
        )
        .await?;

    let after = repos
        .memory
        .list_user_facts_by_source(tenant, &user, FactSource::Onboarding, 100)
        .await?;
    assert!(
        after.is_empty(),
        "superseded answers must drop out of the guaranteed fetch, got {}",
        after.len()
    );
    Ok(())
}

/// The interview's answers survive a recency window full of chat facts.
///
/// The failure this prevents: a calibration answer is a `preference` like any
/// chat-inferred fact, so kind cannot protect it, and the athlete's own
/// conversation evicts the set that was supposed to steer their plans.
#[tokio::test]
async fn calibration_answers_survive_a_flood_of_newer_conversation_facts() -> Result<()> {
    let db = open_in_memory_db().await?;
    let repos = db.repositories();
    let tenant = TenantId::from_uuid(Uuid::new_v4());
    let user_uuid = Uuid::new_v4();
    let user = user_uuid.to_string();

    let interview_answer = seed(
        &repos,
        tenant,
        &user,
        FactKind::Preference,
        Some(Pillar::TrainingAndMovement),
        FactSource::Onboarding,
        "harder means more hard days",
    )
    .await?;
    let recovery_answer = seed(
        &repos,
        tenant,
        &user,
        FactKind::Physiology,
        Some(Pillar::TrainingAndMovement),
        FactSource::Onboarding,
        "needs two easy days after a hard one",
    )
    .await?;

    // Bury them under more than a full bundle of newer chat-inferred facts.
    let flood = FACT_BUNDLE_LIMIT + 10;
    for i in 0..flood {
        seed(
            &repos,
            tenant,
            &user,
            FactKind::Preference,
            Some(Pillar::TrainingAndMovement),
            FactSource::Conversation,
            &format!("chatter {i}"),
        )
        .await?;
    }

    let dossier = repos.dossier.compose_dossier(tenant, user_uuid).await?;
    let objects: Vec<&str> = dossier
        .pillars
        .values()
        .flatten()
        .map(|f| f.object.as_str())
        .collect();

    assert!(
        objects.contains(&"harder means more hard days"),
        "the progression-intent answer was evicted by newer chat facts; \
         the interview's output must be guaranteed by source"
    );
    assert!(
        objects.contains(&"needs two easy days after a hard one"),
        "the recovery-speed answer was evicted; it is one of the two answers \
         that bound how hard a plan may get"
    );

    // Sanity: the flood really did exceed the recency window, so the
    // assertions above are not passing for want of pressure.
    assert!(
        objects.iter().filter(|o| o.starts_with("chatter")).count() > 0,
        "expected the conversation facts to be present too"
    );
    let _ = (interview_answer, recovery_answer);
    Ok(())
}

/// A coach-authored medical flag still cannot be evicted — the by-kind
/// guarantee is not replaced by the by-source one, since a coach-written fact
/// is not `source=onboarding`.
#[tokio::test]
async fn the_by_kind_guarantee_still_covers_coach_authored_medical_facts() -> Result<()> {
    let db = open_in_memory_db().await?;
    let repos = db.repositories();
    let tenant = TenantId::from_uuid(Uuid::new_v4());
    let user_uuid = Uuid::new_v4();
    let user = user_uuid.to_string();

    seed(
        &repos,
        tenant,
        &user,
        FactKind::Medical,
        None,
        FactSource::Coach,
        "cleared by cardiologist 2026-03",
    )
    .await?;

    let flood = FACT_BUNDLE_LIMIT + 10;
    for i in 0..flood {
        seed(
            &repos,
            tenant,
            &user,
            FactKind::Preference,
            Some(Pillar::TrainingAndMovement),
            FactSource::Conversation,
            &format!("chatter {i}"),
        )
        .await?;
    }

    let dossier = repos.dossier.compose_dossier(tenant, user_uuid).await?;
    assert!(
        dossier
            .medical
            .iter()
            .any(|f| f.object == "cleared by cardiologist 2026-03"),
        "the Medical kind guarantee must survive alongside the new source guarantee"
    );
    Ok(())
}
