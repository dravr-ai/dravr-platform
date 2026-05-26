// ABOUTME: Seed command dispatcher for pierre-cli
// ABOUTME: Routes `pierre-cli seed <domain>` subcommands to the appropriate seeder module

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use clap::Subcommand;
use pierre_core::errors::AppResult;
use pierre_core::redaction::redact_url;
use pierre_database::backends::factory::Database;
use pierre_seeders::bootstrap::{run as run_bootstrap, SeedArgs as BootstrapArgs};
use pierre_seeders::coaches::{run as run_coaches, SeedArgs as CoachesArgs};
use pierre_seeders::demo_data::{run as run_demo_data, SeedArgs as DemoDataArgs};
use pierre_seeders::insight_samples::{run as run_insight_samples, SeedArgs as InsightSamplesArgs};
use pierre_seeders::llm_usage::{run as run_llm_usage, SeedArgs as LlmUsageArgs};
use pierre_seeders::mobility::{run as run_mobility, SeedArgs as MobilityArgs};
use pierre_seeders::social::{run as run_social, SeedArgs as SocialArgs};
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

    /// Load insight sample definitions from markdown files (no database writes)
    InsightSamples(InsightSamplesArgs),

    /// Populate the `llm_usage` table with realistic call data for analytics dashboards
    LlmUsage(LlmUsageArgs),

    /// Seed stretching exercises, yoga poses, and activity-muscle mappings
    Mobility(MobilityArgs),

    /// Seed friend connections, shared insights, reactions, and adapted insights
    Social(SocialArgs),

    /// Seed diverse synthetic activities for testing without OAuth providers
    SyntheticActivities(SyntheticActivitiesArgs),
}

/// Dispatch a `Seed` subcommand to its seeder module.
///
/// The `InsightSamples` variant is a pure markdown validator and needs no database.
/// Every other variant shares a single lightweight `Database::init_for_seeding`
/// connection (zero encryption key — seeders only touch reference data), avoiding
/// the full `KeyManager` bootstrap that user/token commands require.
pub async fn dispatch(action: SeedCommand, database_url: &str) -> AppResult<()> {
    match action {
        SeedCommand::InsightSamples(args) => run_insight_samples(&args),
        db_action => dispatch_with_database(db_action, database_url).await,
    }
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
        SeedCommand::Social(args) => run_social(args, &repos).await,
        SeedCommand::SyntheticActivities(args) => run_synthetic_activities(args, &repos).await,
        SeedCommand::InsightSamples(_) => {
            unreachable!("InsightSamples is handled by dispatch() before reaching this path")
        }
    }
}
