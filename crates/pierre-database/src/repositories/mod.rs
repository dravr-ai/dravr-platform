// ABOUTME: Repository module — re-exports the per-domain repository traits and types
// ABOUTME: Per-domain files were split out of the original 3500-line repositories.rs (Finding B)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/// Repository traits for agent-to-agent (`A2A`) protocol persistence.
pub mod a2a;
/// Repository trait for the A2A reaper's bulk fail (one statement, written once).
pub mod a2a_task_reaper;
/// Repository trait for historical activity backfills still owed (resumed by a sweep).
pub mod activity_backfill_jobs;
/// Repository traits for the provider-agnostic activity cache (stale-while-revalidate).
pub mod activity_cache;
/// Repository traits for admin tokens and admin overrides.
pub mod admin;
/// Repository trait for agent package artefacts (flavour, skeleton, workouts beside an agent's prompt).
pub mod agent_artefacts;
/// Repository traits for agents catalogue, coaching groups, store listings.
pub mod agents;
/// Repository trait for API keys.
pub mod api_keys;
/// Repository traits for chat conversation persistence.
pub mod chat;
pub(crate) mod chat_backend;
/// Repository traits for claim verdict (bullshit detector) persistence.
pub mod claim_verdicts;
/// Repository trait for athlete commitments swept against real activity data.
pub mod commitments;
/// Repository traits for data source registration persistence.
pub mod data_source;
/// Repository traits for feature-flag tenant defaults + per-user overrides.
pub mod feature_flags;
/// Repository traits for fitness configuration persistence.
pub mod fitness_config;
/// Repository trait for Guardian pending actions (Confirm human-in-the-loop).
pub mod guardian_actions;
/// Repository traits for coaching harness memory (Tier 0 foundations).
pub mod harness_memory;
/// Repository traits for health, sleep, recovery, time-series persistence and sync cursors.
pub mod health;
/// Repository trait, statements and shared body for super-admin impersonation sessions.
pub mod impersonation;
/// Repository trait for MCP Tasks extension handle persistence.
pub mod mcp_tasks;
/// Shared body of the coaching harness memory repository.
pub mod memory;
/// Repository trait for post-turn memory extractions still owed (resumed by a sweep).
pub mod memory_extraction_jobs;
/// Repository traits for inbound/outbound messaging channel persistence.
pub mod messaging;
/// Shared statements, row decode and body for the link-state lifecycle both backends serve.
pub mod messaging_link_states;
/// Shared SQL and body for the reaction → chat-message lookup both backends serve.
pub mod messaging_reactions;
/// Repository traits for mobility (yoga and stretching) persistence.
pub mod mobility;
/// Repository traits for `OAuth` and system notification persistence.
pub mod notifications;
/// Repository traits for `OAuth` tokens, `OAuth2` server state, `OAuth` client state, provider connections.
pub mod oauth;
/// Shared statements and body for the OAuth client states minted per authorization round trip.
pub mod oauth_client_state;
/// Repository trait for procedural coaching memory (playbooks + pending advice).
pub mod playbooks;
/// Shared statements and body for the pre-approved email allow-list.
pub mod pre_approved_emails;
/// `PrescribedWorkoutRepository`: the ledger of calendar entries Dravr wrote to a provider.
pub mod prescribed_workouts;
/// Repository traits for recipe persistence.
pub mod recipes;
/// Repository trait for messaging turns the shutdown drain handed off to another instance.
pub mod resumable_turns;
/// Repository traits for agent-athlete roster persistence.
pub mod roster;
/// `RouteSummaryRepository`: cached GPX terrain + climbs JSON per activity
pub mod route_summaries;
/// Repository traits for security/audit/key-version persistence.
pub mod security;
/// Repository traits for seed-only repository operations.
pub mod seeder;
/// Shared statements and body for first-party session refresh tokens.
pub mod session_refresh_tokens;
/// Repository trait + helper for the channel-agnostic URL shortener.
pub mod short_links;
/// `SubscriptionsRepository`: provider-agnostic billing subscriptions and webhook event dedupe.
pub mod subscriptions;
/// Repository traits for tenants and subscriptions.
pub mod tenants;
/// Repository traits for tool selection telemetry persistence.
pub mod tool_selection;
/// Shared statements and body for the daily training-state history.
pub mod training_history;
/// Agent-authored training plans (outline + weekly microcycles).
pub mod training_plans;
/// Repository traits for API/`LLM`/usage-counter accounting and `LLM` credentials.
pub mod usage;
/// Repository trait, statements and shared body for user MCP tokens.
pub mod user_mcp_tokens;
/// Repository trait for durable per-user onboarding step completion state.
pub mod user_onboarding;
/// `UserPhysiologicalProfileRepository` + `DossierRepository`: physiology row and the read-time dossier composer
pub mod user_physiological_profiles;
/// Shared statements and bodies for the single-column preference writes on the users row.
pub mod user_preferences;
/// Repository trait, statements and shared body for per-user rate-limit overrides.
pub mod user_rate_limit_overrides;
/// Repository trait, statements and shared body for per-user admin tier overrides.
pub mod user_tier_overrides;
/// Repository trait, statements and shared body for per-user admin tool overrides.
pub mod user_tool_overrides;
/// Repository traits for user accounts, profiles, password resets, physiological profile.
pub mod users;
/// The per-backend uuid column codec shared repository bodies take as an argument.
pub(crate) mod uuid_columns;
/// Repository traits for weather cache persistence.
pub mod weather;
/// Repository trait for the periodic-worker ledger: last tick and current lease per worker.
pub mod worker_runs;
/// `WorkoutTemplateRepository`: user-authored Endurance workout templates.
pub mod workout_templates;
/// Repository traits for prescribed workouts, templates, route summaries, training history.
pub mod workouts;

pub use a2a::*;
pub use a2a_task_reaper::*;
pub use activity_backfill_jobs::*;
pub use activity_cache::*;
pub use admin::*;
pub use agent_artefacts::*;
pub use agents::*;
pub use api_keys::*;
pub use chat::*;
pub use claim_verdicts::*;
pub use commitments::*;
pub use data_source::*;
pub use feature_flags::*;
pub use fitness_config::*;
pub use guardian_actions::*;
pub use harness_memory::*;
pub use health::*;
pub use impersonation::*;
pub use mcp_tasks::*;
pub use memory_extraction_jobs::*;
pub use messaging::*;
pub use mobility::*;
pub use notifications::*;
pub use oauth::*;
pub use playbooks::*;
pub use prescribed_workouts::*;
pub use recipes::*;
pub use resumable_turns::*;
pub use roster::*;
pub use route_summaries::*;
pub use security::*;
pub use seeder::*;
pub use short_links::*;
pub use subscriptions::*;
pub use tenants::*;
pub use tool_selection::*;
pub use training_plans::*;
pub use usage::*;
pub use user_mcp_tokens::*;
pub use user_onboarding::*;
pub use user_physiological_profiles::*;
pub use user_rate_limit_overrides::*;
pub use user_tier_overrides::*;
pub use user_tool_overrides::*;
pub use users::*;
pub use weather::*;
pub use worker_runs::*;
pub use workout_templates::*;
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
