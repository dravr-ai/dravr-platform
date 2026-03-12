// ABOUTME: LLM usage data seeder for testing consumption analytics dashboards
// ABOUTME: Generates 30 days of realistic LLM call data across multiple providers and models
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! LLM usage seeder for Pierre MCP Server.
//!
//! Populates the `llm_usage` table with realistic data across multiple
//! providers, models, and call types so the admin consumption analytics
//! dashboard has meaningful charts and summary cards.
//!
//! Usage:
//! ```bash
//! # Seed with default settings (30 days, assigns to first admin's tenant)
//! cargo run --bin seed-llm-usage
//!
//! # Specify admin email and day count
//! cargo run --bin seed-llm-usage -- --admin-email admin@example.com --days 60
//! ```

use chrono::{Datelike, Duration, Timelike, Utc};
use clap::Parser;
use pierre_core::errors::{AppError, AppResult};
use pierre_database::plugins::factory::Database;
use pierre_database::repositories::SeederRepository;
use pierre_database::seed_models::SeedLlmUsageRecord;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use std::env;
use tracing::info;
use uuid::Uuid;

#[derive(Parser)]
#[command(
    name = "seed-llm-usage",
    about = "Pierre LLM Usage Data Seeder",
    long_about = "Populate the llm_usage table with realistic LLM call data for analytics dashboards"
)]
struct SeedArgs {
    /// Admin email to look up the target tenant (uses first admin if not specified)
    #[arg(long)]
    admin_email: Option<String>,

    /// Database URL override
    #[arg(long)]
    database_url: Option<String>,

    /// Number of days of historical data to generate
    #[arg(long, default_value = "30")]
    days: u32,

    /// Enable verbose logging
    #[arg(long, short = 'v')]
    verbose: bool,
}

/// Provider/model configuration for data generation
struct ModelConfig {
    provider: &'static str,
    model: &'static str,
    /// Relative weight for how often this model appears (higher = more calls)
    weight: u32,
    /// Typical prompt token range
    prompt_tokens_range: (i64, i64),
    /// Typical completion token range
    completion_tokens_range: (i64, i64),
}

/// Call types with relative weights
struct CallTypeConfig {
    call_type: &'static str,
    weight: u32,
}

/// Model configurations matching the pricing table in src/llm/pricing.rs
const MODEL_CONFIGS: &[ModelConfig] = &[
    ModelConfig {
        provider: "gemini",
        model: "gemini-2.5-pro-preview",
        weight: 15,
        prompt_tokens_range: (1000, 5000),
        completion_tokens_range: (500, 2000),
    },
    ModelConfig {
        provider: "gemini",
        model: "gemini-2.5-flash-preview",
        weight: 35,
        prompt_tokens_range: (500, 3000),
        completion_tokens_range: (200, 1500),
    },
    ModelConfig {
        provider: "gemini",
        model: "gemini-2.0-flash-exp",
        weight: 25,
        prompt_tokens_range: (500, 2500),
        completion_tokens_range: (200, 1200),
    },
    ModelConfig {
        provider: "groq",
        model: "llama-3.3-70b-versatile",
        weight: 15,
        prompt_tokens_range: (800, 4000),
        completion_tokens_range: (300, 1800),
    },
    ModelConfig {
        provider: "groq",
        model: "llama-3.1-8b-instant",
        weight: 10,
        prompt_tokens_range: (300, 2000),
        completion_tokens_range: (100, 800),
    },
];

/// Call type distribution
const CALL_TYPES: &[CallTypeConfig] = &[
    CallTypeConfig {
        call_type: "chat",
        weight: 50,
    },
    CallTypeConfig {
        call_type: "insight",
        weight: 30,
    },
    CallTypeConfig {
        call_type: "mcp_tool",
        weight: 20,
    },
];

/// Total weight across all models (for weighted random selection)
fn total_model_weight() -> u32 {
    MODEL_CONFIGS.iter().map(|m| m.weight).sum()
}

/// Total weight across all call types
fn total_call_type_weight() -> u32 {
    CALL_TYPES.iter().map(|c| c.weight).sum()
}

/// Select a model config using weighted random selection
fn pick_model(rng: &mut impl Rng) -> &'static ModelConfig {
    let mut roll = rng.gen_range(0..total_model_weight());
    for config in MODEL_CONFIGS {
        if roll < config.weight {
            return config;
        }
        roll -= config.weight;
    }
    // Fallback (should not reach here given correct weights)
    &MODEL_CONFIGS[0]
}

/// Select a call type using weighted random selection
fn pick_call_type(rng: &mut impl Rng) -> &'static str {
    let mut roll = rng.gen_range(0..total_call_type_weight());
    for config in CALL_TYPES {
        if roll < config.weight {
            return config.call_type;
        }
        roll -= config.weight;
    }
    "chat"
}

#[tokio::main]
async fn main() -> AppResult<()> {
    let args = SeedArgs::parse();

    let log_level = if args.verbose { "debug" } else { "info" };
    tracing_subscriber::fmt().with_env_filter(log_level).init();

    info!("=== Pierre LLM Usage Data Seeder ===");

    let database_url = args
        .database_url
        .or_else(|| env::var("DATABASE_URL").ok())
        .unwrap_or_else(|| "sqlite:./data/users.db".into());

    info!("Connecting to database: {}", database_url);
    let db = Database::init_for_seeding(&database_url).await?;

    // Find admin user and their tenant
    let admin = if let Some(ref email) = args.admin_email {
        db.seed_find_user_by_email(email).await?
    } else {
        db.seed_get_admin_user().await?
    };

    let Some(admin) = admin else {
        return Err(AppError::config(
            "No admin user found. Run setup script first.",
        ));
    };

    let tenant_id_str = db.seed_get_user_tenant(admin.id).await?;
    let Some(tenant_id_str) = tenant_id_str else {
        return Err(AppError::config(format!(
            "User {} has no tenant_id",
            admin.email
        )));
    };
    let tenant_id = Uuid::parse_str(&tenant_id_str)
        .map_err(|e| AppError::config(format!("Invalid tenant_id UUID: {e}")))?;

    info!(
        "Target: user {} ({}) -> tenant {}",
        admin.email, admin.id, tenant_id
    );

    // Clear existing LLM usage data for idempotent re-runs
    let deleted = db.seed_delete_llm_usage_by_tenant(tenant_id).await?;
    if deleted > 0 {
        info!("Cleared {} existing llm_usage records for tenant", deleted);
    }

    // Generate usage data
    info!("Generating {} days of LLM usage data...", args.days);
    let record_count = seed_llm_usage(&db, tenant_id, admin.id, args.days).await?;

    info!("=== Seeding Complete ===");
    info!("LLM Usage Records: {}", record_count);

    Ok(())
}

/// Generate realistic LLM usage records over the specified number of days
async fn seed_llm_usage(
    db: &Database,
    tenant_id: Uuid,
    user_id: Uuid,
    days: u32,
) -> AppResult<u64> {
    let mut rng = StdRng::from_entropy();
    let mut total_records: u64 = 0;

    for day_offset in 0..days {
        let day = Utc::now() - Duration::days(i64::from(day_offset));
        let weekday = day.weekday().num_days_from_monday();

        // Weekends have fewer calls (40-60% of weekday volume)
        let is_weekend = weekday >= 5;
        let base_calls: u32 = if is_weekend {
            rng.gen_range(4..8)
        } else {
            rng.gen_range(8..16)
        };

        for _ in 0..base_calls {
            let model_config = pick_model(&mut rng);
            let call_type = pick_call_type(&mut rng);

            let prompt_tokens = rng
                .gen_range(model_config.prompt_tokens_range.0..=model_config.prompt_tokens_range.1);
            let completion_tokens = rng.gen_range(
                model_config.completion_tokens_range.0..=model_config.completion_tokens_range.1,
            );
            let total_tokens = prompt_tokens + completion_tokens;

            // Tool calls: only for mcp_tool call type
            let tool_calls_count: i64 = if call_type == "mcp_tool" {
                rng.gen_range(1..=5)
            } else {
                0
            };

            // Execution time: larger models take longer
            let execution_time_ms: i64 = match model_config.model {
                m if m.contains("pro") => rng.gen_range(800..3000),
                m if m.contains("70b") => rng.gen_range(600..2500),
                _ => rng.gen_range(200..1200),
            };

            // Generate a business-hours-biased timestamp
            let hour: u32 = if rng.gen_bool(0.75) {
                rng.gen_range(8..20)
            } else {
                rng.gen_range(0..24)
            };
            let minute: u32 = rng.gen_range(0..60);
            let second: u32 = rng.gen_range(0..60);

            let timestamp = day
                .with_hour(hour)
                .unwrap_or(day)
                .with_minute(minute)
                .unwrap_or(day)
                .with_second(second)
                .unwrap_or(day);

            // Some chat calls have a conversation_id
            let conversation_id: Option<String> = if call_type == "chat" && rng.gen_bool(0.7) {
                Some(Uuid::new_v4().to_string())
            } else {
                None
            };

            let record = SeedLlmUsageRecord {
                id: Uuid::new_v4(),
                tenant_id,
                user_id,
                conversation_id,
                provider: model_config.provider.to_owned(),
                model: model_config.model.to_owned(),
                prompt_tokens,
                completion_tokens,
                total_tokens,
                call_type: call_type.to_owned(),
                tool_calls_count,
                execution_time_ms,
                created_at: timestamp,
            };

            if db.seed_insert_llm_usage(&record).await.is_ok() {
                total_records += 1;
            }
        }
    }

    Ok(total_records)
}
