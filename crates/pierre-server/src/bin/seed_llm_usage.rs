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
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use sqlx::{Row, SqlitePool};
use std::env;
use thiserror::Error;
use tracing::info;
use uuid::Uuid;

/// CLI-specific error type for the seed binary
#[derive(Error, Debug)]
enum SeedError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("UUID parse error: {0}")]
    Uuid(#[from] uuid::Error),

    #[error("{0}")]
    Validation(String),
}

type SeedResult<T> = Result<T, SeedError>;

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
async fn main() -> SeedResult<()> {
    let args = SeedArgs::parse();

    let log_level = if args.verbose { "debug" } else { "info" };
    tracing_subscriber::fmt().with_env_filter(log_level).init();

    info!("=== Pierre LLM Usage Data Seeder ===");

    let database_url = args
        .database_url
        .or_else(|| env::var("DATABASE_URL").ok())
        .unwrap_or_else(|| "sqlite:./data/users.db".into());

    info!("Connecting to database: {}", database_url);
    let connection_url = format!("{database_url}?mode=rwc");
    let pool = SqlitePool::connect(&connection_url).await?;

    // Find admin user and their tenant
    let (admin_id, admin_email) = find_admin_user(&pool, args.admin_email.as_deref()).await?;
    let tenant_id = find_tenant_for_user(&pool, &admin_id).await?;
    info!(
        "Target: user {} ({}) → tenant {}",
        admin_email, admin_id, tenant_id
    );

    // Clear existing LLM usage data for idempotent re-runs
    let deleted = sqlx::query("DELETE FROM llm_usage WHERE tenant_id = ?")
        .bind(tenant_id.to_string())
        .execute(&pool)
        .await?;
    if deleted.rows_affected() > 0 {
        info!(
            "Cleared {} existing llm_usage records for tenant",
            deleted.rows_affected()
        );
    }

    // Generate usage data
    info!("Generating {} days of LLM usage data...", args.days);
    let record_count = seed_llm_usage(&pool, &tenant_id, &admin_id, args.days).await?;

    info!("=== Seeding Complete ===");
    info!("LLM Usage Records: {}", record_count);
    print_summary(&pool, &tenant_id).await?;

    Ok(())
}

/// Find admin user by email or get first admin
async fn find_admin_user(pool: &SqlitePool, email: Option<&str>) -> SeedResult<(Uuid, String)> {
    let row = if let Some(email) = email {
        sqlx::query("SELECT id, email FROM users WHERE email = ? AND is_admin = 1")
            .bind(email)
            .fetch_optional(pool)
            .await?
    } else {
        sqlx::query("SELECT id, email FROM users WHERE is_admin = 1 ORDER BY created_at LIMIT 1")
            .fetch_optional(pool)
            .await?
    };

    let Some(row) = row else {
        return Err(SeedError::Validation(
            "No admin user found. Run setup script first.".to_owned(),
        ));
    };

    let id_str: String = row.get("id");
    let email: String = row.get("email");
    let id = Uuid::parse_str(&id_str)?;

    Ok((id, email))
}

/// Look up the `tenant_id` for a given user
async fn find_tenant_for_user(pool: &SqlitePool, user_id: &Uuid) -> SeedResult<Uuid> {
    let row = sqlx::query("SELECT tenant_id FROM users WHERE id = ?")
        .bind(user_id.to_string())
        .fetch_optional(pool)
        .await?;

    let Some(row) = row else {
        return Err(SeedError::Validation(format!(
            "User {user_id} has no tenant_id"
        )));
    };

    let tid_str: String = row.get("tenant_id");
    let tid = Uuid::parse_str(&tid_str)?;
    Ok(tid)
}

/// Generate realistic LLM usage records over the specified number of days
async fn seed_llm_usage(
    pool: &SqlitePool,
    tenant_id: &Uuid,
    user_id: &Uuid,
    days: u32,
) -> SeedResult<u64> {
    let mut rng = StdRng::from_entropy();
    let mut total_records: u64 = 0;
    let tenant_str = tenant_id.to_string();
    let user_str = user_id.to_string();

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
            let base_exec_ms: i64 = match model_config.model {
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
                .unwrap_or(day)
                .to_rfc3339();

            // Some chat calls have a conversation_id
            let conversation_id: Option<String> = if call_type == "chat" && rng.gen_bool(0.7) {
                Some(Uuid::new_v4().to_string())
            } else {
                None
            };

            let id = Uuid::new_v4();
            let result = sqlx::query(
                "INSERT INTO llm_usage (\
                     id, tenant_id, user_id, conversation_id, provider, model, \
                     prompt_tokens, completion_tokens, total_tokens, call_type, \
                     tool_calls_count, execution_time_ms, created_at\
                 ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(id.to_string())
            .bind(&tenant_str)
            .bind(&user_str)
            .bind(&conversation_id)
            .bind(model_config.provider)
            .bind(model_config.model)
            .bind(prompt_tokens)
            .bind(completion_tokens)
            .bind(total_tokens)
            .bind(call_type)
            .bind(tool_calls_count)
            .bind(base_exec_ms)
            .bind(&timestamp)
            .execute(pool)
            .await;

            if result.is_ok() {
                total_records += 1;
            }
        }
    }

    Ok(total_records)
}

/// Print summary of the seeded data
async fn print_summary(pool: &SqlitePool, tenant_id: &Uuid) -> SeedResult<()> {
    let tid = tenant_id.to_string();

    let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM llm_usage WHERE tenant_id = ?")
        .bind(&tid)
        .fetch_one(pool)
        .await?;
    info!("  Total records: {count}");

    print_provider_breakdown(pool, &tid).await?;
    print_call_type_breakdown(pool, &tid).await?;

    Ok(())
}

/// Print per-provider usage breakdown
async fn print_provider_breakdown(pool: &SqlitePool, tenant_id: &str) -> SeedResult<()> {
    let rows: Vec<(String, i64, i64)> = sqlx::query_as(
        "SELECT provider, COUNT(*), SUM(total_tokens) FROM llm_usage WHERE tenant_id = ? GROUP BY provider",
    )
    .bind(tenant_id)
    .fetch_all(pool)
    .await?;
    for (provider, calls, tokens) in &rows {
        info!("  {provider}: {calls} calls, {tokens} tokens");
    }
    Ok(())
}

/// Print per-call-type usage breakdown
async fn print_call_type_breakdown(pool: &SqlitePool, tenant_id: &str) -> SeedResult<()> {
    let rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT call_type, COUNT(*) FROM llm_usage WHERE tenant_id = ? GROUP BY call_type",
    )
    .bind(tenant_id)
    .fetch_all(pool)
    .await?;
    for (call_type, calls) in &rows {
        info!("  {call_type}: {calls} calls");
    }
    Ok(())
}
