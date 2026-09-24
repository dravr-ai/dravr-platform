// ABOUTME: Seed command dispatcher for pierre-cli
// ABOUTME: Routes `pierre-cli seed <domain>` subcommands to the appropriate seeder module

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use clap::Subcommand;
use pierre_auth::key_management::KeyManager;
#[cfg(feature = "postgresql")]
use pierre_core::config::database::PostgresPoolConfig;
use pierre_core::errors::AppResult;
use pierre_core::redaction::redact_url;
use pierre_database::backends::factory::Database;
use pierre_database::RepositoryRegistry;
use pierre_llm::config::LlmProviderType;
use pierre_seeders::agents::{run as run_agents, SeedArgs as AgentsArgs};
use pierre_seeders::bootstrap::{run as run_bootstrap, SeedArgs as BootstrapArgs};
use pierre_seeders::demo_data::{run as run_demo_data, SeedArgs as DemoDataArgs};
use pierre_seeders::llm_usage::{run as run_llm_usage, SeedArgs as LlmUsageArgs};
use pierre_seeders::mobility::{run as run_mobility, SeedArgs as MobilityArgs};
use pierre_seeders::synthetic_activities::{
    run as run_synthetic_activities, SeedArgs as SyntheticActivitiesArgs,
};
use pierre_seeders::trainingpeaks_delegation::{
    run as run_trainingpeaks_delegation, SeedArgs as TrainingPeaksDelegationArgs,
};
use tracing::info;

#[non_exhaustive]
#[derive(Subcommand)]
pub enum SeedCommand {
    /// Create admin and demo users for a fresh deployment (idempotent)
    Bootstrap(BootstrapArgs),

    /// Load agent definitions from markdown files, sync them to the database, and delete catalogue agents whose file is gone
    Agents(AgentsArgs),

    /// Populate database with realistic demo data for dashboard testing
    DemoData(DemoDataArgs),

    /// Populate the `llm_usage` table with realistic call data for analytics dashboards
    LlmUsage(LlmUsageArgs),

    /// Seed stretching exercises, yoga poses, and activity-muscle mappings
    Mobility(MobilityArgs),

    /// Seed diverse synthetic activities for testing without OAuth providers
    SyntheticActivities(SyntheticActivitiesArgs),

    /// Seed a group whose coach proposed a TrainingPeaks link to a member, for the member-confirm flows
    TrainingpeaksDelegation(TrainingPeaksDelegationArgs),
}

/// Dispatch a `Seed` subcommand to its seeder module.
///
/// Every variant but the two that write an encrypted token
/// (`SyntheticActivities`, `TrainingpeaksDelegation`) shares a single
/// lightweight `Database::init_for_seeding` connection (zero encryption key —
/// those seeders only touch reference data), avoiding the full `KeyManager`
/// bootstrap that user/token commands require.
pub async fn dispatch(action: SeedCommand, database_url: &str) -> AppResult<()> {
    match action {
        // synthetic-activities seeds an encrypted dev-fixture oauth_token, so it
        // needs the real DEK from KeyManager — the zero seeding key would write
        // a token the server can't decrypt, leaving the user "disconnected".
        SeedCommand::SyntheticActivities(args) => {
            let repos = keyed_repositories(database_url, "synthetic-activities").await?;
            run_synthetic_activities(args, &repos).await
        }
        // The coach's stand-in TrainingPeaks session is an encrypted token too.
        // The group threads get the model every new conversation gets from
        // the configured LLM provider, unless one is named.
        SeedCommand::TrainingpeaksDelegation(mut args) => {
            args.model = args.model.or_else(LlmProviderType::model_from_env);
            let repos = keyed_repositories(database_url, "trainingpeaks-delegation").await?;
            run_trainingpeaks_delegation(args, &repos).await
        }
        db_action => dispatch_with_database(db_action, database_url).await,
    }
}

/// Repositories over the full two-tier key management (the real DEK), for a
/// seeder that writes an encrypted `oauth_token`: the token then decrypts in
/// the server exactly like a real provider connection.
async fn keyed_repositories(database_url: &str, seeder: &str) -> AppResult<RepositoryRegistry> {
    info!(
        "Connecting to database for {seeder} seeding (full key init): {}",
        redact_url(database_url)
    );
    let (mut key_manager, database_encryption_key) = KeyManager::bootstrap()?;
    let mut database = Database::new(
        database_url,
        database_encryption_key.to_vec(),
        #[cfg(feature = "postgresql")]
        &PostgresPoolConfig::default(),
    )
    .await?;
    key_manager.complete_initialization(&mut database).await?;
    Ok(database.repositories())
}

async fn dispatch_with_database(action: SeedCommand, database_url: &str) -> AppResult<()> {
    info!(
        "Connecting to database for seeding: {}",
        redact_url(database_url)
    );
    let db = Database::init_for_seeding(database_url).await?;
    let repos = db.repositories();

    match action {
        SeedCommand::Bootstrap(args) => run_bootstrap(args, &repos).await,
        SeedCommand::Agents(args) => run_agents(args, &repos).await,
        SeedCommand::DemoData(args) => run_demo_data(args, &repos).await,
        SeedCommand::LlmUsage(args) => run_llm_usage(args, &repos).await,
        SeedCommand::Mobility(args) => run_mobility(args, &repos).await,
        SeedCommand::SyntheticActivities(_) | SeedCommand::TrainingpeaksDelegation(_) => {
            unreachable!("token-writing seeders are handled by dispatch() with full key init")
        }
    }
}
