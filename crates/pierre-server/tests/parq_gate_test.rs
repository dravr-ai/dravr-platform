// ABOUTME: Integration test for the PAR-Q+ gate — Yes answers persist coach-visible medical flags
// ABOUTME: Verifies flags land in the Dossier medical bucket with a 12-month freshness horizon
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use anyhow::Result;
use chrono::Utc;
#[cfg(feature = "postgresql")]
use pierre_config::environment::PostgresPoolConfig;
use pierre_core::models::TenantId;
use pierre_database::backends::factory::Database;
use pierre_database::database::generate_encryption_key;
use pierre_services::parq;
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

#[tokio::test]
async fn parq_yes_answers_raise_medical_flags() -> Result<()> {
    let db = open_in_memory_db().await?;
    let repos = db.repositories();
    let tenant = TenantId::from(Uuid::new_v4());
    let user = Uuid::new_v4();
    let user_s = user.to_string();

    // Two "yes" answers + one unknown id (ignored).
    let yes = vec![
        "heart_condition".to_owned(),
        "chest_pain".to_owned(),
        "not_a_question".to_owned(),
    ];
    let raised = parq::persist_parq_flags(repos.memory.as_ref(), tenant, &user_s, &yes).await?;
    assert_eq!(raised, 2, "two known yes answers raise two flags");

    let dossier = repos.dossier.compose_dossier(tenant, user).await?;
    assert_eq!(dossier.medical.len(), 2, "flags land in the medical bucket");

    // 12-month freshness: valid_until is set and in the future (not stale).
    let now = Utc::now();
    for fact in &dossier.medical {
        let vu = fact.valid_until.expect("medical flag carries valid_until");
        assert!(vu > now, "flag is fresh");
        assert!(!fact.stale);
        assert_eq!(fact.kind, "medical");
        assert_eq!(fact.source, "onboarding");
    }

    Ok(())
}

#[tokio::test]
async fn parq_all_no_raises_nothing() -> Result<()> {
    let db = open_in_memory_db().await?;
    let repos = db.repositories();
    let tenant = TenantId::from(Uuid::new_v4());
    let user = Uuid::new_v4();

    let raised =
        parq::persist_parq_flags(repos.memory.as_ref(), tenant, &user.to_string(), &[]).await?;
    assert_eq!(raised, 0);

    let dossier = repos.dossier.compose_dossier(tenant, user).await?;
    assert!(dossier.medical.is_empty());
    Ok(())
}
