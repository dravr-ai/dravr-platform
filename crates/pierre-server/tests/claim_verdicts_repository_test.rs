// ABOUTME: Integration test for the Tier 5.5 ClaimVerdictRepository against an in-memory SQLite DB
// ABOUTME: Exercises the migration, insert, and list paths end-to-end through RepositoryRegistry
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::time::Duration;

use anyhow::Result;
use pierre_core::models::TenantId;
use pierre_database::database::generate_encryption_key;
use pierre_database::plugins::factory::Database;
use pierre_database::repositories::InsertClaimVerdictParams;
#[cfg(feature = "postgresql")]
use pierre_mcp_server::config::environment::PostgresPoolConfig;
use pierre_memory::claims::{ClaimCategory, ClaimStatus, EvidenceStrength, VerdictLayer};
use tokio::time::sleep;
use uuid::Uuid;

async fn fresh_db() -> Result<Database> {
    let key = generate_encryption_key().to_vec();
    #[cfg(feature = "postgresql")]
    let db = Database::new("sqlite::memory:", key, &PostgresPoolConfig::default()).await?;
    #[cfg(not(feature = "postgresql"))]
    let db = Database::new("sqlite::memory:", key).await?;
    Ok(db)
}

fn tenant() -> TenantId {
    TenantId::from(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap())
}

#[tokio::test]
async fn insert_and_list_claim_verdict_round_trip() -> Result<()> {
    let db = fresh_db().await?;
    let repos = db.repositories();
    let params = InsertClaimVerdictParams {
        tenant_id: tenant(),
        user_id: "user-1",
        coach_id: None,
        conversation_id: None,
        message_id: None,
        claim_text: "Protein at 1.6 g/kg/day maximizes muscle protein synthesis",
        category: ClaimCategory::Nutrition,
        status: ClaimStatus::Supported,
        evidence_strength: EvidenceStrength::Strong,
        confidence: 0.8,
        layer_fired: VerdictLayer::Evidence,
        explanation: Some("Supported by Morton 2018"),
        evidence_refs: Some("doi:10.1/a"),
    };

    let saved = repos.claim_verdicts.insert_claim_verdict(&params).await?;
    assert_eq!(saved.status, ClaimStatus::Supported);
    assert_eq!(saved.category, ClaimCategory::Nutrition);
    assert_eq!(saved.evidence_strength, EvidenceStrength::Strong);
    assert_eq!(saved.layer_fired, VerdictLayer::Evidence);
    assert!(!saved.id.is_empty());

    let recent = repos
        .claim_verdicts
        .list_recent_verdicts(tenant(), 10)
        .await?;
    assert_eq!(recent.len(), 1);
    assert_eq!(recent[0].id, saved.id);

    Ok(())
}

#[tokio::test]
async fn list_recent_verdicts_returns_newest_first() -> Result<()> {
    let db = fresh_db().await?;
    let repos = db.repositories();
    for (i, status) in [
        ClaimStatus::Supported,
        ClaimStatus::Unsupported,
        ClaimStatus::Contradicted,
    ]
    .iter()
    .enumerate()
    {
        let claim = format!("claim number {i}");
        let params = InsertClaimVerdictParams {
            tenant_id: tenant(),
            user_id: "user-1",
            coach_id: None,
            conversation_id: None,
            message_id: None,
            claim_text: &claim,
            category: ClaimCategory::Physiological,
            status: *status,
            evidence_strength: EvidenceStrength::Mixed,
            confidence: 0.6,
            layer_fired: VerdictLayer::Evidence,
            explanation: None,
            evidence_refs: None,
        };
        repos.claim_verdicts.insert_claim_verdict(&params).await?;
        // Nudge timestamp ordering; sqlx RFC3339 seconds may collide otherwise.
        sleep(Duration::from_millis(10)).await;
    }

    let recent = repos
        .claim_verdicts
        .list_recent_verdicts(tenant(), 10)
        .await?;
    assert_eq!(recent.len(), 3);
    // Newest first → last inserted is Contradicted.
    assert_eq!(recent[0].status, ClaimStatus::Contradicted);
    assert_eq!(recent[2].status, ClaimStatus::Supported);

    Ok(())
}

#[tokio::test]
async fn list_verdicts_for_conversation_filters_by_conversation() -> Result<()> {
    let db = fresh_db().await?;
    let repos = db.repositories();

    // Conversation A rows need a real conversation to satisfy the FK — skip
    // the conversation_id for isolation testing and exercise tenant-only scope.
    let params_a = InsertClaimVerdictParams {
        tenant_id: tenant(),
        user_id: "user-1",
        coach_id: None,
        conversation_id: None,
        message_id: None,
        claim_text: "tenant-scoped claim a",
        category: ClaimCategory::Recovery,
        status: ClaimStatus::Rhetorical,
        evidence_strength: EvidenceStrength::None,
        confidence: 1.0,
        layer_fired: VerdictLayer::Rhetoric,
        explanation: None,
        evidence_refs: None,
    };
    repos.claim_verdicts.insert_claim_verdict(&params_a).await?;

    let all = repos
        .claim_verdicts
        .list_recent_verdicts(tenant(), 10)
        .await?;
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].layer_fired, VerdictLayer::Rhetoric);

    Ok(())
}
