// ABOUTME: Seed command dispatcher for pierre-cli
// ABOUTME: Routes `pierre-cli seed <domain>` subcommands to the appropriate seeder module

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use clap::Subcommand;
use pierre_core::redaction::redact_url;
use pierre_database::plugins::factory::Database;
use pierre_mcp_server::errors::AppResult;
use pierre_mcp_server::seeders;
use tracing::info;

#[non_exhaustive]
#[derive(Subcommand)]
pub enum SeedCommand {
    /// Create admin and demo users for a fresh deployment (idempotent)
    Bootstrap(seeders::bootstrap::SeedArgs),

    /// Load coach definitions from markdown files and sync to database
    Coaches(seeders::coaches::SeedArgs),

    /// Populate database with realistic demo data for dashboard testing
    DemoData(seeders::demo_data::SeedArgs),

    /// Load insight sample definitions from markdown files (no database writes)
    InsightSamples(seeders::insight_samples::SeedArgs),

    /// Populate the `llm_usage` table with realistic call data for analytics dashboards
    LlmUsage(seeders::llm_usage::SeedArgs),

    /// Seed stretching exercises, yoga poses, and activity-muscle mappings
    Mobility(seeders::mobility::SeedArgs),

    /// Seed friend connections, shared insights, reactions, and adapted insights
    Social(seeders::social::SeedArgs),

    /// Seed diverse synthetic activities for testing without OAuth providers
    SyntheticActivities(seeders::synthetic_activities::SeedArgs),
}

/// Dispatch a `Seed` subcommand to its seeder module.
///
/// The `InsightSamples` variant is a pure markdown validator and needs no database.
/// Every other variant shares a single lightweight `Database::init_for_seeding`
/// connection (zero encryption key — seeders only touch reference data), avoiding
/// the full `KeyManager` bootstrap that user/token commands require.
pub async fn dispatch(action: SeedCommand, database_url: &str) -> AppResult<()> {
    match action {
        SeedCommand::InsightSamples(args) => seeders::insight_samples::run(&args),
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
        SeedCommand::Bootstrap(args) => seeders::bootstrap::run(args, &repos).await,
        SeedCommand::Coaches(args) => seeders::coaches::run(args, &repos).await,
        SeedCommand::DemoData(args) => seeders::demo_data::run(args, &repos).await,
        SeedCommand::LlmUsage(args) => seeders::llm_usage::run(args, &repos).await,
        SeedCommand::Mobility(args) => seeders::mobility::run(args, &repos).await,
        SeedCommand::Social(args) => seeders::social::run(args, &repos).await,
        SeedCommand::SyntheticActivities(args) => {
            seeders::synthetic_activities::run(args, &repos).await
        }
        SeedCommand::InsightSamples(_) => {
            unreachable!("InsightSamples is handled by dispatch() before reaching this path")
        }
    }
}
