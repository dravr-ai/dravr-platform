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
use pierre_seeders::bootstrap::{run as run_bootstrap, SeedArgs as BootstrapArgs};
use pierre_seeders::coaches::{run as run_coaches, SeedArgs as CoachesArgs};
use pierre_seeders::demo_data::{run as run_demo_data, SeedArgs as DemoDataArgs};
use pierre_seeders::llm_usage::{run as run_llm_usage, SeedArgs as LlmUsageArgs};
use pierre_seeders::mobility::{run as run_mobility, SeedArgs as MobilityArgs};
use pierre_seeders::synthetic_activities::{
    run as run_synthetic_activities, SeedArgs as SyntheticActivitiesArgs,
};
use tracing::info;

#[non_exhaustive]
#[derive(Subcommand)]
pub enum SeedCommand {
    /// Create admin and demo users for a fresh deployment (idempotent)
    Bootstrap(BootstrapArgs),

    /// Load coach definitions from markdown files and sync to database
    Coaches(CoachesArgs),

    /// Populate database with realistic demo data for dashboard testing
    DemoData(DemoDataArgs),

    /// Populate the `llm_usage` table with realistic call data for analytics dashboards
    LlmUsage(LlmUsageArgs),

    /// Seed stretching exercises, yoga poses, and activity-muscle mappings
    Mobility(MobilityArgs),

    /// Seed diverse synthetic activities for testing without OAuth providers
    SyntheticActivities(SyntheticActivitiesArgs),
}

/// Dispatch a `Seed` subcommand to its seeder module.
///
/// Every variant but `SyntheticActivities` shares a single lightweight
/// `Database::init_for_seeding` connection (zero encryption key — seeders only
/// touch reference data), avoiding the full `KeyManager` bootstrap that
/// user/token commands require.
pub async fn dispatch(action: SeedCommand, database_url: &str) -> AppResult<()> {
    match action {
        // synthetic-activities seeds an encrypted dev-fixture oauth_token, so it
        // needs the real DEK from KeyManager — the zero seeding key would write
        // a token the server can't decrypt, leaving the user "disconnected".
        SeedCommand::SyntheticActivities(args) => {
            dispatch_synthetic_activities(args, database_url).await
        }
        db_action => dispatch_with_database(db_action, database_url).await,
    }
}

/// Seed synthetic activities + a dev-fixture provider token using the full
/// two-tier key management (real DEK), so the seeded `oauth_token` decrypts in
/// the server exactly like a real provider connection.
async fn dispatch_synthetic_activities(
    args: SyntheticActivitiesArgs,
    database_url: &str,
) -> AppResult<()> {
    info!(
        "Connecting to database for synthetic-activities seeding (full key init): {}",
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
    let repos = database.repositories();
    run_synthetic_activities(args, &repos).await
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
        SeedCommand::Coaches(args) => run_coaches(args, &repos).await,
        SeedCommand::DemoData(args) => run_demo_data(args, &repos).await,
        SeedCommand::LlmUsage(args) => run_llm_usage(args, &repos).await,
        SeedCommand::Mobility(args) => run_mobility(args, &repos).await,
        SeedCommand::SyntheticActivities(_) => {
            unreachable!("SyntheticActivities is handled by dispatch() with full key init")
        }
    }
}
