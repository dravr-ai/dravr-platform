// ABOUTME: Integration test for expire_onboarding_facts — /pillars re-run supersession primitive
// ABOUTME: Verifies onboarding facts are superseded by valid_until (optionally per-pillar), conversation facts untouched
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use anyhow::Result;
use pierre_core::models::{Pillar, TenantId};
use pierre_database::backends::factory::Database;
use pierre_database::database::generate_encryption_key;
use pierre_database::database::test_utils::create_test_db_with_key;
use pierre_database::repositories::UpsertUserFactParams;
use pierre_database::RepositoryRegistry;
use pierre_memory::{FactKind, FactSource, MemoryScope, PredicateCode};
use uuid::Uuid;

async fn open_in_memory_db() -> Result<Database> {
    let encryption_key = generate_encryption_key().to_vec();
    let db = create_test_db_with_key(encryption_key).await?;
    Ok(db)
}

async fn seed(
    repos: &RepositoryRegistry,
    tenant: TenantId,
    user: &str,
    pillar: Option<Pillar>,
    source: FactSource,
    object: &str,
) -> Result<()> {
    repos
        .memory
        .upsert_user_fact(&UpsertUserFactParams {
            tenant_id: tenant,
            user_id: user,
            coach_id: None,
            scope: MemoryScope::User,
            kind: FactKind::Goal,
            pillar,
            predicate_code: PredicateCode::States,
            object,
            confidence: 0.9,
            source,
            valid_until: None,
            source_msg_id: None,
        })
        .await?;
    Ok(())
}

#[tokio::test]
async fn expire_onboarding_facts_supersedes_scoped_then_all() -> Result<()> {
    let db = open_in_memory_db().await?;
    let repos = db.repositories();
    let tenant = TenantId::from_uuid(Uuid::new_v4());
    let user = Uuid::new_v4();
    let user_s = user.to_string();

    seed(
        &repos,
        tenant,
        &user_s,
        Some(Pillar::Fuelling),
        FactSource::Onboarding,
        "avoid dairy",
    )
    .await?;
    seed(
        &repos,
        tenant,
        &user_s,
        Some(Pillar::SleepAndRecovery),
        FactSource::Onboarding,
        "sleeps 7h",
    )
    .await?;
    // A conversation-sourced fact in the same pillar — must NEVER be superseded.
    seed(
        &repos,
        tenant,
        &user_s,
        Some(Pillar::Fuelling),
        FactSource::Conversation,
        "likes coffee",
    )
    .await?;

    // Scoped re-screen: only the Fuelling onboarding fact is superseded.
    let n = repos
        .memory
        .expire_onboarding_facts(tenant, &user_s, Some(Pillar::Fuelling), None, None)
        .await?;
    assert_eq!(n, 1, "only the one Fuelling onboarding fact superseded");

    let dossier = repos.dossier.compose_dossier(tenant, user).await?;
    let fuelling = dossier
        .pillars
        .get(&Pillar::Fuelling)
        .expect("fuelling bucket");
    // One stale (the onboarding fact), one fresh (the conversation fact).
    assert_eq!(fuelling.iter().filter(|f| f.stale).count(), 1);
    assert_eq!(fuelling.iter().filter(|f| !f.stale).count(), 1);

    // Full re-screen: the remaining Sleep onboarding fact is superseded; the
    // already-expired Fuelling one is excluded; the conversation fact untouched.
    let n = repos
        .memory
        .expire_onboarding_facts(tenant, &user_s, None, None, None)
        .await?;
    assert_eq!(
        n, 1,
        "only the still-fresh Sleep onboarding fact superseded"
    );

    let dossier = repos.dossier.compose_dossier(tenant, user).await?;
    assert!(
        dossier.pillars[&Pillar::SleepAndRecovery]
            .iter()
            .all(|f| f.stale),
        "sleep onboarding fact now stale"
    );
    assert!(
        dossier.pillars[&Pillar::Fuelling].iter().any(|f| !f.stale),
        "conversation fact stays fresh"
    );

    Ok(())
}
