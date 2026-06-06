// ABOUTME: Repository module — re-exports the per-domain repository traits and types
// ABOUTME: Per-domain files were split out of the original 3500-line repositories.rs (Finding B)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/// Repository traits for agent-to-agent (`A2A`) protocol persistence.
pub mod a2a;
/// Repository traits for the provider-agnostic activity cache (stale-while-revalidate).
pub mod activity_cache;
/// Repository traits for admin tokens and admin overrides.
pub mod admin;
/// Repository traits for API keys and user-scoped `MCP` tokens.
pub mod api_keys;
/// Repository traits for chat conversation persistence.
pub mod chat;
/// Repository traits for claim verdict (Tier 5.5 bullshit detector) persistence.
pub mod claim_verdicts;
/// Repository traits for coaches catalogue, coaching groups, store listings.
pub mod coaches;
/// Repository traits for data source registration persistence.
pub mod data_source;
/// Repository traits for feature-flag tenant defaults + per-user overrides.
pub mod feature_flags;
/// Repository traits for fitness configuration persistence.
pub mod fitness_config;
/// Repository traits for coaching harness memory (Tier 0 foundations).
pub mod harness_memory;
/// Repository traits for health, sleep, recovery, time-series persistence and sync cursors.
pub mod health;
/// Repository traits for inbound/outbound messaging channel persistence.
pub mod messaging;
/// Repository traits for mobility (yoga and stretching) persistence.
pub mod mobility;
/// Repository traits for `OAuth` and system notification persistence.
pub mod notifications;
/// Repository traits for `OAuth` tokens, `OAuth2` server state, `OAuth` client state, provider connections.
pub mod oauth;
/// Repository traits for recipe persistence.
pub mod recipes;
/// Repository traits for coach-athlete roster persistence.
pub mod roster;
/// Repository traits for security/audit/key-version persistence.
pub mod security;
/// Repository traits for seed-only repository operations.
pub mod seeder;
/// Repository traits for social graph persistence.
pub mod social;
/// Repository traits for tenants and subscriptions.
pub mod tenants;
/// Repository traits for tool selection telemetry persistence.
pub mod tool_selection;
/// Repository traits for API/`LLM`/usage-counter accounting and `LLM` credentials.
pub mod usage;
/// Repository traits for user accounts, profiles, password resets, impersonation, physiological profile.
pub mod users;
/// Repository traits for weather cache persistence.
pub mod weather;
/// Repository traits for prescribed workouts, templates, route summaries, training history.
pub mod workouts;

pub use a2a::*;
pub use activity_cache::*;
pub use admin::*;
pub use api_keys::*;
pub use chat::*;
pub use claim_verdicts::*;
pub use coaches::*;
pub use data_source::*;
pub use feature_flags::*;
pub use fitness_config::*;
pub use harness_memory::*;
pub use health::*;
pub use messaging::*;
pub use mobility::*;
pub use notifications::*;
pub use oauth::*;
pub use recipes::*;
pub use roster::*;
pub use security::*;
pub use seeder::*;
pub use social::*;
pub use tenants::*;
pub use tool_selection::*;
pub use usage::*;
pub use users::*;
pub use weather::*;
pub use workouts::*;

use async_trait::async_trait;
use pierre_core::errors::AppResult;

/// Database lifecycle trait for connection creation and schema migration.
///
/// Individual data access is provided by the focused repository traits above.
/// Backends implement both `DatabaseProvider` (for lifecycle) and whichever
/// repository traits they support (for data access).
#[async_trait]
pub trait DatabaseProvider: Send + Sync {
    /// Create a new database connection with encryption key
    async fn new(database_url: &str, encryption_key: Vec<u8>) -> AppResult<Self>
    where
        Self: Sized;

    /// Run database migrations to set up schema
    async fn migrate(&self) -> AppResult<()>;
}
