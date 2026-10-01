// ABOUTME: Every LLM provider TenantLlmManager accepts can be stored in user_llm_credentials and read back
// ABOUTME: Pins the provider CHECK to the LlmProvider enum on either backend; re-running the shipped migration keeps every row

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The `user_llm_credentials.provider` CHECK listed gemini, groq, openai,
//! anthropic and local, while the LLM settings routes and
//! [`TenantLlmManager`] also accept cohere: saving Cohere credentials failed
//! the constraint on both backends. Each test stores credentials through
//! the manager and reads the decrypted key back from the database, so an
//! environment-variable fallback cannot stand in for the stored row.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use chrono::Utc;
use pierre_auth::tenant::llm_manager::{
    CredentialSource, LlmProvider, StoreLlmCredentialsRequest, TenantLlmManager,
};
use pierre_core::models::{Tenant, TenantId};
use pierre_database::backends::factory::{Database, DatabaseBackend};
use pierre_test_support::db::create_test_db;
use pierre_test_support::server::create_test_user;
use uuid::Uuid;

/// The shipped migration, per backend, so the test runs the exact statements
/// a deploy runs rather than a paraphrase of them.
const SQLITE_MIGRATION_SQL: &str =
    include_str!("../../../migrations/20261001180000_user_llm_credentials_cohere_provider.sql");
#[cfg(feature = "postgresql")]
const POSTGRES_MIGRATION_SQL: &str =
    include_str!("../../../migrations_pg/20261001180000_user_llm_credentials_cohere_provider.sql");

async fn run_migration(database: &Database) {
    match database.backend() {
        DatabaseBackend::SQLite(sqlite) => {
            sqlx::raw_sql(SQLITE_MIGRATION_SQL)
                .execute(sqlite.pool())
                .await
                .unwrap();
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(postgres) => {
            sqlx::raw_sql(POSTGRES_MIGRATION_SQL)
                .execute(postgres.pool())
                .await
                .unwrap();
        }
    }
}

/// The provider after `provider` in declaration order, `None` after the
/// last. The match is exhaustive, so a new [`LlmProvider`] variant does not
/// compile until it joins the walk the round-trip test stores.
const fn next_provider(provider: LlmProvider) -> Option<LlmProvider> {
    match provider {
        LlmProvider::Gemini => Some(LlmProvider::Groq),
        LlmProvider::Groq => Some(LlmProvider::OpenAi),
        LlmProvider::OpenAi => Some(LlmProvider::Anthropic),
        LlmProvider::Anthropic => Some(LlmProvider::Cohere),
        LlmProvider::Cohere => Some(LlmProvider::Local),
        LlmProvider::Local => None,
    }
}

/// Every provider, in declaration order.
fn every_provider() -> Vec<LlmProvider> {
    let mut providers = vec![LlmProvider::Gemini];
    while let Some(next) = next_provider(*providers.last().unwrap()) {
        providers.push(next);
    }
    providers
}

/// A user who owns a tenant, both stored.
async fn user_with_tenant(database: &Database) -> (Uuid, TenantId) {
    let user = create_test_user(&format!("llm-{}@example.com", Uuid::new_v4()), None);
    let user_id = database.repositories().users.create(&user).await.unwrap();
    let tenant_id = TenantId::generate();
    let now = Utc::now();
    database
        .repositories()
        .tenants
        .create(&Tenant {
            id: tenant_id,
            name: "LLM credentials".to_owned(),
            slug: tenant_id.to_string(),
            domain: None,
            plan: "professional".to_owned(),
            owner_user_id: user_id,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
    database
        .repositories()
        .users
        .update_tenant_id(user_id, tenant_id)
        .await
        .unwrap();
    (user_id, tenant_id)
}

/// Store `api_key` for `provider` under `user_id` (`None` = tenant default).
async fn store(
    database: &Database,
    user_id: Option<Uuid>,
    tenant_id: TenantId,
    created_by: Uuid,
    provider: LlmProvider,
    api_key: &str,
) {
    TenantLlmManager::store_credentials(
        user_id,
        tenant_id,
        StoreLlmCredentialsRequest {
            provider,
            api_key: api_key.to_owned(),
            base_url: None,
            default_model: Some(format!("{provider}-model")),
        },
        created_by,
        &*database.repositories().llm_credentials,
        &*database.repositories().security,
    )
    .await
    .unwrap_or_else(|e| panic!("storing {provider} credentials failed: {e}"));
}

#[tokio::test]
async fn cohere_credentials_store_and_read_back_for_a_user_and_a_tenant() {
    let database = create_test_db().await.unwrap();
    let (user_id, tenant_id) = user_with_tenant(&database).await;

    store(
        &database,
        Some(user_id),
        tenant_id,
        user_id,
        LlmProvider::Cohere,
        "cohere-user-key",
    )
    .await;
    store(
        &database,
        None,
        tenant_id,
        user_id,
        LlmProvider::Cohere,
        "cohere-tenant-key",
    )
    .await;

    let user_creds = TenantLlmManager::get_credentials(
        Some(user_id),
        tenant_id,
        LlmProvider::Cohere,
        &*database.repositories().llm_credentials,
        &*database.repositories().security,
    )
    .await
    .unwrap();
    assert_eq!(user_creds.source, CredentialSource::UserSpecific);
    assert_eq!(user_creds.provider, LlmProvider::Cohere);
    assert_eq!(user_creds.api_key, "cohere-user-key");
    assert_eq!(user_creds.default_model.as_deref(), Some("cohere-model"));

    let tenant_creds = TenantLlmManager::get_credentials(
        None,
        tenant_id,
        LlmProvider::Cohere,
        &*database.repositories().llm_credentials,
        &*database.repositories().security,
    )
    .await
    .unwrap();
    assert_eq!(tenant_creds.source, CredentialSource::TenantDefault);
    assert_eq!(tenant_creds.api_key, "cohere-tenant-key");
}

#[tokio::test]
async fn every_llm_provider_stores_and_reads_back() {
    let database = create_test_db().await.unwrap();
    let (user_id, tenant_id) = user_with_tenant(&database).await;
    let providers = every_provider();

    for provider in &providers {
        store(
            &database,
            Some(user_id),
            tenant_id,
            user_id,
            *provider,
            &format!("{provider}-key"),
        )
        .await;
    }

    for provider in providers {
        let creds = TenantLlmManager::get_credentials(
            Some(user_id),
            tenant_id,
            provider,
            &*database.repositories().llm_credentials,
            &*database.repositories().security,
        )
        .await
        .unwrap();
        assert_eq!(creds.source, CredentialSource::UserSpecific, "{provider}");
        assert_eq!(creds.api_key, format!("{provider}-key"));
    }
}

#[tokio::test]
async fn the_migration_keeps_every_stored_credential() {
    let database = create_test_db().await.unwrap();
    let (user_id, tenant_id) = user_with_tenant(&database).await;
    store(
        &database,
        Some(user_id),
        tenant_id,
        user_id,
        LlmProvider::Groq,
        "groq-key",
    )
    .await;
    store(
        &database,
        None,
        tenant_id,
        user_id,
        LlmProvider::Cohere,
        "cohere-tenant-key",
    )
    .await;

    // On SQLite the table is rebuilt: copied, dropped and renamed.
    run_migration(&database).await;

    let groq = TenantLlmManager::get_credentials(
        Some(user_id),
        tenant_id,
        LlmProvider::Groq,
        &*database.repositories().llm_credentials,
        &*database.repositories().security,
    )
    .await
    .unwrap();
    assert_eq!(groq.source, CredentialSource::UserSpecific);
    assert_eq!(groq.api_key, "groq-key");
    assert_eq!(groq.default_model.as_deref(), Some("groq-model"));

    let cohere = TenantLlmManager::get_credentials(
        None,
        tenant_id,
        LlmProvider::Cohere,
        &*database.repositories().llm_credentials,
        &*database.repositories().security,
    )
    .await
    .unwrap();
    assert_eq!(cohere.source, CredentialSource::TenantDefault);
    assert_eq!(cohere.api_key, "cohere-tenant-key");
}
