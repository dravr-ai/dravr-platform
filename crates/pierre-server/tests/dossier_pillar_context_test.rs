// ABOUTME: Integration tests for per-user pillar context — compose_dossier fact buckets + OKF render
// ABOUTME: Seeds user_facts, composes the Dossier, asserts pillar/north-star/medical grouping + isolation
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use anyhow::Result;
#[cfg(feature = "postgresql")]
use pierre_config::environment::PostgresPoolConfig;
use pierre_core::models::{Pillar, TenantId};
use pierre_database::backends::factory::Database;
use pierre_database::database::generate_encryption_key;
use pierre_database::repositories::UpsertUserFactParams;
use pierre_database::RepositoryRegistry;
use pierre_memory::{FactKind, FactSource, MemoryScope};
use pierre_services::okf::render_okf_bundle_default;
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

#[tokio::test]
async fn pillar_facts_group_into_dossier_buckets() -> Result<()> {
    let db = open_in_memory_db().await?;
    let repos = db.repositories();
    let tenant = TenantId::from(Uuid::new_v4());
    let user = Uuid::new_v4();
    let user_s = user.to_string();

    seed_fact(
        &repos,
        tenant,
        &user_s,
        FactKind::Goal,
        Some(Pillar::Fuelling),
        "avoid dairy before long runs",
    )
    .await?;
    seed_fact(
        &repos,
        tenant,
        &user_s,
        FactKind::NorthStar,
        None,
        "be present and energetic for my kids",
    )
    .await?;
    seed_fact(
        &repos,
        tenant,
        &user_s,
        FactKind::Medical,
        None,
        "chest pain during exercise (PAR-Q)",
    )
    .await?;

    let dossier = repos.dossier.compose_dossier(tenant, user).await?;

    let fuelling = dossier
        .pillars
        .get(&Pillar::Fuelling)
        .expect("fuelling pillar bucket present");
    assert_eq!(fuelling.len(), 1);
    assert_eq!(fuelling[0].object, "avoid dairy before long runs");
    assert_eq!(dossier.north_star.len(), 1, "north star fact routed");
    assert_eq!(dossier.medical.len(), 1, "medical fact routed");

    // The OKF bundle renders the pillar section with the fact inside the fence.
    let bundle = render_okf_bundle_default(&dossier).expect("non-empty bundle");
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
    let db = open_in_memory_db().await?;
    let repos = db.repositories();
    let tenant = TenantId::from(Uuid::new_v4());
    let user = Uuid::new_v4();
    let user_s = user.to_string();

    // Oldest fact: the medical flag.
    seed_fact(
        &repos,
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
            &repos,
            tenant,
            &user_s,
            FactKind::Goal,
            Some(Pillar::Fuelling),
            &format!("conversation fact number {i}"),
        )
        .await?;
    }

    let dossier = repos.dossier.compose_dossier(tenant, user).await?;
    assert_eq!(
        dossier.medical.len(),
        1,
        "medical flag must survive eviction from the recency window"
    );
    let bundle = render_okf_bundle_default(&dossier).expect("non-empty bundle");
    // The flag reaches the coach prompt (PHI redacted); raw text withheld.
    assert!(bundle.contains("a medical/PAR-Q flag is on file"));
    assert!(!bundle.contains("chest pain during exercise (PAR-Q)"));

    Ok(())
}

#[tokio::test]
async fn dossier_facts_are_tenant_scoped() -> Result<()> {
    let db = open_in_memory_db().await?;
    let repos = db.repositories();
    let tenant_a = TenantId::from(Uuid::new_v4());
    let tenant_b = TenantId::from(Uuid::new_v4());
    let user = Uuid::new_v4();
    let user_s = user.to_string();

    // Fact lives under tenant B only.
    seed_fact(
        &repos,
        tenant_b,
        &user_s,
        FactKind::Goal,
        Some(Pillar::SleepAndRecovery),
        "sleeps 8h on training nights",
    )
    .await?;

    // Composing the same user under tenant A must not see tenant B's fact.
    let dossier_a = repos.dossier.compose_dossier(tenant_a, user).await?;
    assert!(dossier_a.pillars.is_empty(), "no cross-tenant facts leak");
    assert!(render_okf_bundle_default(&dossier_a).is_none());

    // Tenant B sees its own fact.
    let dossier_b = repos.dossier.compose_dossier(tenant_b, user).await?;
    assert_eq!(
        dossier_b
            .pillars
            .get(&Pillar::SleepAndRecovery)
            .map(Vec::len),
        Some(1)
    );

    Ok(())
}
