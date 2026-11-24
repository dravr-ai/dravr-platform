// ABOUTME: Integration tests for per-user pillar context — compose_dossier fact buckets + OKF render
// ABOUTME: Seeds user_facts, composes the Dossier, asserts pillar/north-star/medical grouping + isolation
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_core::transport::TransportPolicy;

use anyhow::Result;
use pierre_contremaitre::messaging_strings::MessagingStringsRegistry;
use pierre_core::models::{Pillar, TenantId};
use pierre_database::backends::factory::{Database, DatabaseBackend};
use pierre_database::database::generate_encryption_key;
use pierre_database::repositories::UpsertUserFactParams;
use pierre_database::RepositoryRegistry;
use pierre_memory::PredicateCode;
use pierre_memory::{FactKind, FactSource, MemoryScope};
use pierre_services::memory_facts::SentenceRenderer;
use pierre_services::okf::render_okf_bundle_default;
use pierre_test_support::db::create_test_db_with_key;
use pierre_test_support::log_capture::{self, CapturedEvent};
use tracing::Level;
use uuid::Uuid;

async fn open_test_db() -> Result<Database> {
    Ok(create_test_db_with_key(generate_encryption_key().to_vec()).await?)
}

async fn seed_fact(
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
            agent_id: None,
            scope: MemoryScope::User,
            kind,
            pillar,
            predicate_code: PredicateCode::Have,
            object,
            confidence: 0.9,
            source: FactSource::Onboarding,
            valid_until: None,
            source_msg_id: None,
            transport_policy: TransportPolicy::AnyTransport,
            ai_consent_sources: &[],
        })
        .await?;
    Ok(())
}

#[tokio::test]
async fn pillar_facts_group_into_dossier_buckets() -> Result<()> {
    let db = open_test_db().await?;
    let repos = db.repositories();
    let tenant = TenantId::from_uuid(Uuid::new_v4());
    let user = Uuid::new_v4();
    let user_s = user.to_string();

    seed_fact(
        repos,
        tenant,
        &user_s,
        FactKind::Goal,
        Some(Pillar::Fuelling),
        "avoid dairy before long runs",
    )
    .await?;
    seed_fact(
        repos,
        tenant,
        &user_s,
        FactKind::NorthStar,
        None,
        "be present and energetic for my kids",
    )
    .await?;
    seed_fact(
        repos,
        tenant,
        &user_s,
        FactKind::Medical,
        None,
        "chest pain during exercise (PAR-Q)",
    )
    .await?;

    let dossier = repos
        .dossier
        .compose_dossier(tenant, user, TransportPolicy::FirstPartyOnly, &|_, _| true)
        .await?;

    let fuelling = dossier
        .pillars
        .get(&Pillar::Fuelling)
        .expect("fuelling pillar bucket present");
    assert_eq!(fuelling.len(), 1);
    assert_eq!(fuelling[0].object, "avoid dairy before long runs");
    assert_eq!(dossier.north_star.len(), 1, "north star fact routed");
    assert_eq!(dossier.medical.len(), 1, "medical fact routed");

    // The OKF bundle renders the pillar section with the fact inside the fence.
    let bundle = render_okf_bundle_default(
        &dossier,
        SentenceRenderer::new(&MessagingStringsRegistry::new(), "en"),
    )
    .expect("non-empty bundle");
    assert!(bundle.contains("pillar: fuelling"));
    assert!(bundle.contains("avoid dairy before long runs"));
    assert!(bundle.contains("be present and energetic for my kids"));
    assert!(bundle.contains("type: medical"));
    // The subsumed flat block must be gone.
    assert!(!bundle.contains("What you remember about this user"));

    Ok(())
}

#[tokio::test]
async fn medical_fact_survives_recency_window_eviction() -> Result<()> {
    // Regression: the OKF "always include Medical/North Star" guarantee must
    // not be defeated by the recency-bounded (LIMIT 40, updated_at DESC) general
    // fact fetch. A PAR-Q flag captured once at onboarding, then buried under
    // more than FACT_BUNDLE_LIMIT newer conversation facts, must still reach the
    // dossier's medical bucket via the by-kind safety fetch.
    let db = open_test_db().await?;
    let repos = db.repositories();
    let tenant = TenantId::from_uuid(Uuid::new_v4());
    let user = Uuid::new_v4();
    let user_s = user.to_string();

    // Oldest fact: the medical flag.
    seed_fact(
        repos,
        tenant,
        &user_s,
        FactKind::Medical,
        None,
        "chest pain during exercise (PAR-Q)",
    )
    .await?;
    // Bury it under 45 newer conversation facts (> the 40-row recency window).
    for i in 0..45 {
        seed_fact(
            repos,
            tenant,
            &user_s,
            FactKind::Goal,
            Some(Pillar::Fuelling),
            &format!("conversation fact number {i}"),
        )
        .await?;
    }

    let dossier = repos
        .dossier
        .compose_dossier(tenant, user, TransportPolicy::FirstPartyOnly, &|_, _| true)
        .await?;
    assert_eq!(
        dossier.medical.len(),
        1,
        "medical flag must survive eviction from the recency window"
    );
    let bundle = render_okf_bundle_default(
        &dossier,
        SentenceRenderer::new(&MessagingStringsRegistry::new(), "en"),
    )
    .expect("non-empty bundle");
    // The flag reaches the agent prompt (PHI redacted); raw text withheld.
    assert!(bundle.contains("a medical/PAR-Q flag is on file"));
    assert!(!bundle.contains("chest pain during exercise (PAR-Q)"));

    Ok(())
}

#[tokio::test]
async fn dossier_facts_are_tenant_scoped() -> Result<()> {
    let db = open_test_db().await?;
    let repos = db.repositories();
    let tenant_a = TenantId::from_uuid(Uuid::new_v4());
    let tenant_b = TenantId::from_uuid(Uuid::new_v4());
    let user = Uuid::new_v4();
    let user_s = user.to_string();

    // Fact lives under tenant B only.
    seed_fact(
        repos,
        tenant_b,
        &user_s,
        FactKind::Goal,
        Some(Pillar::SleepAndRecovery),
        "sleeps 8h on training nights",
    )
    .await?;

    // Composing the same user under tenant A must not see tenant B's fact.
    let dossier_a = repos
        .dossier
        .compose_dossier(tenant_a, user, TransportPolicy::FirstPartyOnly, &|_, _| {
            true
        })
        .await?;
    assert!(dossier_a.pillars.is_empty(), "no cross-tenant facts leak");
    assert!(render_okf_bundle_default(
        &dossier_a,
        SentenceRenderer::new(&MessagingStringsRegistry::new(), "en")
    )
    .is_none());

    // Tenant B sees its own fact.
    let dossier_b = repos
        .dossier
        .compose_dossier(tenant_b, user, TransportPolicy::FirstPartyOnly, &|_, _| {
            true
        })
        .await?;
    assert_eq!(
        dossier_b
            .pillars
            .get(&Pillar::SleepAndRecovery)
            .map(Vec::len),
        Some(1)
    );

    Ok(())
}

#[tokio::test]
async fn failing_fact_reads_warn_with_the_error_and_still_render() -> Result<()> {
    // A fact read that fails at the repository (a bind/decode fault or a
    // missing relation) must not be swallowed: the dossier still renders on
    // the surviving data, but each failed read leaves a WARN naming the error
    // and the user, the same way the neighbouring `get_profile` degrade does.
    // Silently returning an empty vec strips medical flags and every
    // interview answer from the agent's context with no admin signal.
    let db = open_test_db().await?;
    let tenant = TenantId::from_uuid(Uuid::new_v4());
    let user = Uuid::new_v4();
    let user_s = user.to_string();

    // Take the fact table away so every fact read in compose_dossier fails.
    match db.backend() {
        DatabaseBackend::SQLite(sqlite) => {
            sqlx::query("DROP TABLE user_facts")
                .execute(sqlite.pool())
                .await?;
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(pg) => {
            sqlx::query("DROP TABLE user_facts")
                .execute(pg.pool())
                .await?;
        }
    }

    let repos = db.repositories();
    let (events, _guard) = log_capture::capture_events();
    let dossier = repos
        .dossier
        .compose_dossier(tenant, user, TransportPolicy::FirstPartyOnly, &|_, _| true)
        .await?;

    // Degradation is preserved: the dossier still composes, minus the facts.
    assert!(dossier.pillars.is_empty());
    assert!(dossier.medical.is_empty());
    assert!(dossier.north_star.is_empty());
    assert_eq!(dossier.user_id, user);

    let warns: Vec<CapturedEvent> = events
        .events()
        .into_iter()
        .filter(|e| {
            e.level == Level::WARN && e.message().is_some_and(|m| m.contains("fact read failed"))
        })
        .collect();

    // One per read: the recency window, Medical, NorthStar, and by-source.
    assert_eq!(
        warns.len(),
        4,
        "every failed fact read warns, got: {warns:?}"
    );
    for warn in &warns {
        assert_eq!(
            warn.field("user_id"),
            Some(user_s.as_str()),
            "warn names the affected user"
        );
        let error = warn.field("error").unwrap_or_default();
        assert!(
            error.contains("user_facts"),
            "warn carries the repository error, got {error:?}"
        );
    }

    let mut kinds: Vec<&str> = warns.iter().filter_map(|w| w.field("fact_kind")).collect();
    kinds.sort_unstable();
    assert_eq!(
        kinds,
        vec!["medical", "north_star"],
        "the by-kind safety guarantee names which kind was lost"
    );
    assert_eq!(
        warns
            .iter()
            .filter_map(|w| w.field("fact_source"))
            .collect::<Vec<_>>(),
        vec!["onboarding"],
        "the by-source guarantee names the interview source"
    );

    Ok(())
}
