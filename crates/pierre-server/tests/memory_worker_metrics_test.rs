// ABOUTME: Sprint C6 — integration tests for HarnessMemoryRepository::count_user_facts_metrics
// ABOUTME: Covers empty tenant, single-kind, and mixed-kind aggregation against SQLite in-memory
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use anyhow::Result;
use pierre_core::models::TenantId;
use pierre_database::backends::factory::Database;
use pierre_database::database::generate_encryption_key;
use pierre_database::repositories::UpsertUserFactParams;
#[cfg(feature = "postgresql")]
use pierre_mcp_server::config::environment::PostgresPoolConfig;
use pierre_memory::{FactKind, MemoryScope};
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

fn fact_params<'a>(
    tenant_id: TenantId,
    user_id: &'a str,
    kind: FactKind,
    object: &'a str,
) -> UpsertUserFactParams<'a> {
    UpsertUserFactParams {
        tenant_id,
        user_id,
        coach_id: None,
        scope: MemoryScope::User,
        kind,
        subject: "you",
        predicate: "have",
        object,
        confidence: 0.9,
        source_msg_id: None,
        embedding: None,
    }
}

#[tokio::test]
async fn empty_tenant_returns_zero_metrics() -> Result<()> {
    let db = open_in_memory_db().await?;
    let repos = db.repositories();
    let tenant = TenantId::from(Uuid::new_v4());

    let metrics = repos.memory.count_user_facts_metrics(tenant).await?;

    assert_eq!(metrics.total_facts, 0);
    assert_eq!(metrics.facts_last_24h, 0);
    assert_eq!(metrics.facts_last_7d, 0);
    assert_eq!(metrics.distinct_users, 0);
    assert!(metrics.facts_by_kind.is_empty());
    assert!(metrics.newest_updated_at.is_none());

    Ok(())
}

#[tokio::test]
async fn mixed_kinds_aggregate_correctly() -> Result<()> {
    let db = open_in_memory_db().await?;
    let repos = db.repositories();
    let tenant = TenantId::from(Uuid::new_v4());

    let user_a = Uuid::new_v4().to_string();
    let user_b = Uuid::new_v4().to_string();

    repos
        .memory
        .upsert_user_fact(&fact_params(
            tenant,
            &user_a,
            FactKind::Goal,
            "sub-3 marathon",
        ))
        .await?;
    repos
        .memory
        .upsert_user_fact(&fact_params(
            tenant,
            &user_a,
            FactKind::Injury,
            "left achilles tendinitis",
        ))
        .await?;
    repos
        .memory
        .upsert_user_fact(&fact_params(tenant, &user_b, FactKind::Goal, "body recomp"))
        .await?;
    repos
        .memory
        .upsert_user_fact(&fact_params(
            tenant,
            &user_b,
            FactKind::Preference,
            "prefers outdoor running",
        ))
        .await?;

    let metrics = repos.memory.count_user_facts_metrics(tenant).await?;

    assert_eq!(metrics.total_facts, 4);
    assert_eq!(metrics.distinct_users, 2);
    // All writes just happened so both windows should capture everything.
    assert_eq!(metrics.facts_last_24h, 4);
    assert_eq!(metrics.facts_last_7d, 4);
    assert_eq!(metrics.facts_by_kind.get("goal").copied(), Some(2));
    assert_eq!(metrics.facts_by_kind.get("injury").copied(), Some(1));
    assert_eq!(metrics.facts_by_kind.get("preference").copied(), Some(1));
    assert!(metrics.newest_updated_at.is_some());

    Ok(())
}

#[tokio::test]
async fn metrics_are_tenant_scoped() -> Result<()> {
    let db = open_in_memory_db().await?;
    let repos = db.repositories();
    let tenant_a = TenantId::from(Uuid::new_v4());
    let tenant_b = TenantId::from(Uuid::new_v4());

    let user = Uuid::new_v4().to_string();

    repos
        .memory
        .upsert_user_fact(&fact_params(tenant_a, &user, FactKind::Goal, "alpha goal"))
        .await?;
    repos
        .memory
        .upsert_user_fact(&fact_params(
            tenant_a,
            &user,
            FactKind::Goal,
            "alpha goal 2",
        ))
        .await?;
    repos
        .memory
        .upsert_user_fact(&fact_params(
            tenant_b,
            &user,
            FactKind::Injury,
            "beta injury",
        ))
        .await?;

    let metrics_a = repos.memory.count_user_facts_metrics(tenant_a).await?;
    let metrics_b = repos.memory.count_user_facts_metrics(tenant_b).await?;

    assert_eq!(metrics_a.total_facts, 2);
    assert_eq!(metrics_a.facts_by_kind.get("goal").copied(), Some(2));
    assert!(!metrics_a.facts_by_kind.contains_key("injury"));

    assert_eq!(metrics_b.total_facts, 1);
    assert_eq!(metrics_b.facts_by_kind.get("injury").copied(), Some(1));
    assert!(!metrics_b.facts_by_kind.contains_key("goal"));

    Ok(())
}
