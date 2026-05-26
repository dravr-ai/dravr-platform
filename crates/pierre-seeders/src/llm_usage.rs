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
//! pierre-cli seed llm-usage
//!
//! # Specify admin email and day count
//! pierre-cli seed llm-usage --admin-email admin@example.com --days 60
//! ```

use chrono::{Datelike, Duration, Timelike, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_database::seed_models::SeedLlmUsageRecord;
use pierre_database::RepositoryRegistry;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use tracing::info;
use uuid::Uuid;

/// CLI arguments for the LLM usage seeder.
#[derive(clap::Args)]
pub struct SeedArgs {
    /// Admin email to look up the target tenant (uses first admin if not specified)
    #[arg(long)]
    pub admin_email: Option<String>,

    /// Number of days of historical data to generate
    #[arg(long, default_value = "30")]
    pub days: u32,
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
    let mut roll = rng.random_range(0..total_model_weight());
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
    let mut roll = rng.random_range(0..total_call_type_weight());
    for config in CALL_TYPES {
        if roll < config.weight {
            return config.call_type;
        }
        roll -= config.weight;
    }
    "chat"
}

/// Generate synthetic LLM call records across multiple providers for analytics dashboards.
///
/// # Errors
///
/// Returns an error if no admin user is found, if the user has no tenant, or if any
/// repository operation fails while inserting usage records.
pub async fn run(args: SeedArgs, repos: &RepositoryRegistry) -> AppResult<()> {
    info!("=== Pierre LLM Usage Data Seeder ===");
    let (user_id, tenant_id) = resolve_admin_tenant(repos, args.admin_email.as_deref()).await?;
    clear_existing_usage(repos, tenant_id).await?;
    let record_count = generate_usage_data(repos, tenant_id, user_id, args.days).await?;
    info!("=== Seeding Complete: {record_count} LLM usage records ===");
    Ok(())
}

async fn generate_usage_data(
    repos: &RepositoryRegistry,
    tenant_id: Uuid,
    user_id: Uuid,
    days: u32,
) -> AppResult<u64> {
    info!("Generating {days} days of LLM usage data...");
    seed_llm_usage(repos, tenant_id, user_id, days).await
}

/// Look up the admin user (by email or fall back to first) and resolve their tenant UUID.
async fn resolve_admin_tenant(
    repos: &RepositoryRegistry,
    admin_email: Option<&str>,
) -> AppResult<(Uuid, Uuid)> {
    let admin = if let Some(email) = admin_email {
        repos.seeder.seed_find_user_by_email(email).await?
    } else {
        repos.seeder.seed_get_admin_user().await?
    };

    let Some(admin) = admin else {
        return Err(AppError::config(
            "No admin user found. Run setup script first.",
        ));
    };

    let tenant_id_str = repos
        .seeder
        .seed_get_user_tenant(admin.id)
        .await?
        .ok_or_else(|| AppError::config(format!("User {} has no tenant_id", admin.email)))?;
    let tenant_id = Uuid::parse_str(&tenant_id_str)
        .map_err(|e| AppError::config(format!("Invalid tenant_id UUID: {e}")))?;

    info!(
        "Target: user {} ({}) -> tenant {}",
        admin.email, admin.id, tenant_id
    );

    Ok((admin.id, tenant_id))
}

/// Delete existing LLM usage records for a tenant so re-runs are idempotent.
async fn clear_existing_usage(repos: &RepositoryRegistry, tenant_id: Uuid) -> AppResult<()> {
    let deleted = repos
        .seeder
        .seed_delete_llm_usage_by_tenant(tenant_id)
        .await?;
    if deleted > 0 {
        info!("Cleared {deleted} existing llm_usage records for tenant");
    }
    Ok(())
}

/// Generate realistic LLM usage records over the specified number of days
async fn seed_llm_usage(
    repos: &RepositoryRegistry,
    tenant_id: Uuid,
    user_id: Uuid,
    days: u32,
) -> AppResult<u64> {
    let mut rng = StdRng::from_os_rng();
    let mut total_records: u64 = 0;

    for day_offset in 0..days {
        let day = Utc::now() - Duration::days(i64::from(day_offset));
        let weekday = day.weekday().num_days_from_monday();

        // Weekends have fewer calls (40-60% of weekday volume)
        let is_weekend = weekday >= 5;
        let base_calls: u32 = if is_weekend {
            rng.random_range(4..8)
        } else {
            rng.random_range(8..16)
        };

        for _ in 0..base_calls {
            let model_config = pick_model(&mut rng);
            let call_type = pick_call_type(&mut rng);

            let prompt_tokens = rng.random_range(
                model_config.prompt_tokens_range.0..=model_config.prompt_tokens_range.1,
            );
            let completion_tokens = rng.random_range(
                model_config.completion_tokens_range.0..=model_config.completion_tokens_range.1,
            );
            let total_tokens = prompt_tokens + completion_tokens;

            // Tool calls: only for mcp_tool call type
            let tool_calls_count: i64 = if call_type == "mcp_tool" {
                rng.random_range(1..=5)
            } else {
                0
            };

            // Execution time: larger models take longer
            let execution_time_ms: i64 = match model_config.model {
                m if m.contains("pro") => rng.random_range(800..3000),
                m if m.contains("70b") => rng.random_range(600..2500),
                _ => rng.random_range(200..1200),
            };

            // Generate a business-hours-biased timestamp
            let hour: u32 = if rng.random_bool(0.75) {
                rng.random_range(8..20)
            } else {
                rng.random_range(0..24)
            };
            let minute: u32 = rng.random_range(0..60);
            let second: u32 = rng.random_range(0..60);

            let timestamp = day
                .with_hour(hour)
                .unwrap_or(day)
                .with_minute(minute)
                .unwrap_or(day)
                .with_second(second)
                .unwrap_or(day);

            // Some chat calls have a conversation_id
            let conversation_id: Option<String> = if call_type == "chat" && rng.random_bool(0.7) {
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

            if repos.seeder.seed_insert_llm_usage(&record).await.is_ok() {
                total_records += 1;
            }
        }
    }

    Ok(total_records)
}
