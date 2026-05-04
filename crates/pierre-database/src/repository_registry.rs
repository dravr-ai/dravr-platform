// ABOUTME: Trait-object registry that holds Arc<dyn Repository> for every domain
// ABOUTME: Built once at startup from SQLite or PostgreSQL backend, eliminates runtime dispatch
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

#[cfg(feature = "postgresql")]
use crate::backends::postgres::PostgresDatabase;
use crate::database::Database as SqliteDatabase;
use crate::repositories::{
    A2ARepository, AdminRepository, ApiKeyRepository, ChatRepository, ClaimVerdictRepository,
    CoachesRepository, CoachingGroupRepository, DataSourceRepository, DossierRepository,
    FitnessConfigRepository, HarnessMemoryRepository, HealthSnapshotRepository,
    ImpersonationRepository, InsightRepository, LlmCredentialRepository, LlmUsageRepository,
    MessagingRepository, MobilityRepository, NotificationRepository, OAuth2ServerRepository,
    OAuthClientStateRepository, OAuthTokenRepository, PasswordResetRepository,
    PrescribedWorkoutRepository, ProfileRepository, ProviderConnectionRepository, RecipeRepository,
    RecoveryRepository, RouteSummaryRepository, SecurityRepository, SeederRepository,
    SleepRepository, SocialRepository, StoreListingsRepository, SubscriptionsRepository,
    SyncCursorRepository, TenantRepository, TimeSeriesPointRepository, ToolSelectionRepository,
    TrainingHistoryRepository, UsageCounterRepository, UsageRepository, UserMcpTokenRepository,
    UserPhysiologicalProfileRepository, UserRepository, WeatherCacheRepository,
    WorkoutTemplateRepository,
};

/// Holds one `Arc<dyn Repository>` per domain trait.
///
/// Constructed once at startup via [`RepositoryRegistry::from_sqlite`] or
/// [`RepositoryRegistry::from_postgres`]. Consumers access repositories directly
/// without runtime enum dispatch.
pub struct RepositoryRegistry {
    /// User account CRUD and lookup
    pub users: Arc<dyn UserRepository>,
    /// User profile operations
    pub profiles: Arc<dyn ProfileRepository>,
    /// Admin token management
    pub admin: Arc<dyn AdminRepository>,
    /// API key CRUD and usage tracking
    pub api_keys: Arc<dyn ApiKeyRepository>,
    /// Chat conversation and message storage
    pub chat: Arc<dyn ChatRepository>,
    /// Coach persona management
    pub coaches: Arc<dyn CoachesRepository>,
    /// User fitness configuration
    pub fitness_config: Arc<dyn FitnessConfigRepository>,
    /// Admin impersonation sessions
    pub impersonation: Arc<dyn ImpersonationRepository>,
    /// AI-generated insight storage
    pub insights: Arc<dyn InsightRepository>,
    /// LLM provider credential management
    pub llm_credentials: Arc<dyn LlmCredentialRepository>,
    /// LLM usage tracking for cost analysis
    pub llm_usage: Arc<dyn LlmUsageRepository>,
    /// Multi-channel messaging gateway
    pub messaging: Arc<dyn MessagingRepository>,
    /// Stretching exercises and yoga poses
    pub mobility: Arc<dyn MobilityRepository>,
    /// OAuth completion notifications
    pub notifications: Arc<dyn NotificationRepository>,
    /// `OAuth2` authorization server (clients, codes, tokens)
    pub oauth2_server: Arc<dyn OAuth2ServerRepository>,
    /// OAuth client state (CSRF protection)
    pub oauth_client_state: Arc<dyn OAuthClientStateRepository>,
    /// User OAuth provider tokens
    pub oauth_tokens: Arc<dyn OAuthTokenRepository>,
    /// Password reset token management
    pub password_reset: Arc<dyn PasswordResetRepository>,
    /// Provider connection tracking
    pub provider_connections: Arc<dyn ProviderConnectionRepository>,
    /// Recipe CRUD with nutrition
    pub recipes: Arc<dyn RecipeRepository>,
    /// RSA keypairs, key rotation, audit events
    pub security: Arc<dyn SecurityRepository>,
    /// Seed-only database operations
    pub seeder: Arc<dyn SeederRepository>,
    /// Social features (friend connections, shared insights, reactions)
    pub social: Option<Arc<dyn SocialRepository>>,
    /// Store listings for coach marketplace
    pub store_listings: Arc<dyn StoreListingsRepository>,
    /// Tenant CRUD and user-tenant roles
    pub tenants: Arc<dyn TenantRepository>,
    /// Per-tenant MCP tool configuration
    pub tool_selection: Arc<dyn ToolSelectionRepository>,
    /// Usage analytics and statistics
    pub usage: Arc<dyn UsageRepository>,
    /// Rate-limiting usage counters
    pub usage_counters: Arc<dyn UsageCounterRepository>,
    /// User MCP token management
    pub user_mcp_tokens: Arc<dyn UserMcpTokenRepository>,
    /// Agent-to-Agent protocol
    pub a2a: Arc<dyn A2ARepository>,
    /// Coaching group CRUD, membership, and invites
    pub groups: Arc<dyn CoachingGroupRepository>,
    /// Data source (device/provider) tracking
    pub data_sources: Arc<dyn DataSourceRepository>,
    /// Sleep session persistence
    pub sleep: Arc<dyn SleepRepository>,
    /// Recovery metrics persistence
    pub recovery: Arc<dyn RecoveryRepository>,
    /// Health snapshot persistence
    pub health_snapshots: Arc<dyn HealthSnapshotRepository>,
    /// Sync cursor tracking for CDC-based incremental sync
    pub sync_cursors: Arc<dyn SyncCursorRepository>,
    /// Coaching harness memory (compaction, facts, notes, followups, sessions)
    pub memory: Arc<dyn HarnessMemoryRepository>,
    /// Tier 5.5 claim verdicts from the bullshit detector pipeline
    pub claim_verdicts: Arc<dyn ClaimVerdictRepository>,
    /// Stripe-backed subscription rows (one per (tenant, `stripe_subscription`))
    pub subscriptions: Arc<dyn SubscriptionsRepository>,
    /// dravr-meteo persistent weather cache (geographic + hourly buckets)
    pub weather_cache: Arc<dyn WeatherCacheRepository>,
    /// Endurance typed physiological profile (FTP, threshold pace, zones)
    pub user_physiological_profile: Arc<dyn UserPhysiologicalProfileRepository>,
    /// Endurance dossier composer (read-time aggregate from physiology /
    /// goals / zones / nutrition / equipment)
    pub dossier: Arc<dyn DossierRepository>,
    /// Endurance daily training-state rollups (CTL/ATL/TSB/ACWR/monotony/
    /// strain/`ramp_rate`/`daily_load`)
    pub training_history: Arc<dyn TrainingHistoryRepository>,
    /// Endurance cached GPX terrain + climbs per activity
    pub route_summaries: Arc<dyn RouteSummaryRepository>,
    /// Endurance prescribed-workout audit trail (one row per push to a provider calendar)
    pub prescribed_workouts: Arc<dyn PrescribedWorkoutRepository>,
    /// Endurance user-authored workout-template overrides (cornerstones live in TOML)
    pub workout_templates: Arc<dyn WorkoutTemplateRepository>,
    /// Continuous time-series points (`data_point_series` table) backing the
    /// dravr-enforme `TimeSeriesPointStore` adapter.
    pub time_series_points: Arc<dyn TimeSeriesPointRepository>,
}

impl RepositoryRegistry {
    /// Build the registry from a `SQLite` database.
    ///
    /// The `Database` struct implements every repository trait directly,
    /// so a single `Arc<Database>` is cloned into each slot.
    #[must_use]
    pub fn from_sqlite(db: Arc<SqliteDatabase>) -> Self {
        Self {
            users: db.clone(),
            profiles: db.clone(),
            admin: db.clone(),
            api_keys: db.clone(),
            chat: db.clone(),
            coaches: db.clone(),
            fitness_config: db.clone(),
            impersonation: db.clone(),
            insights: db.clone(),
            llm_credentials: db.clone(),
            llm_usage: db.clone(),
            messaging: db.clone(),
            mobility: db.clone(),
            notifications: db.clone(),
            oauth2_server: db.clone(),
            oauth_client_state: db.clone(),
            oauth_tokens: db.clone(),
            password_reset: db.clone(),
            provider_connections: db.clone(),
            recipes: db.clone(),
            security: db.clone(),
            seeder: db.clone(),
            social: Some(db.clone()),
            store_listings: db.clone(),
            tenants: db.clone(),
            tool_selection: db.clone(),
            usage: db.clone(),
            usage_counters: db.clone(),
            user_mcp_tokens: db.clone(),
            a2a: db.clone(),
            groups: db.clone(),
            data_sources: db.clone(),
            sleep: db.clone(),
            recovery: db.clone(),
            health_snapshots: db.clone(),
            sync_cursors: db.clone(),
            memory: db.clone(),
            claim_verdicts: db.clone(),
            subscriptions: db.clone(),
            weather_cache: db.clone(),
            user_physiological_profile: db.clone(),
            dossier: db.clone(),
            training_history: db.clone(),
            route_summaries: db.clone(),
            prescribed_workouts: db.clone(),
            workout_templates: db.clone(),
            time_series_points: db,
        }
    }

    /// Build the registry from a `PostgreSQL` database.
    #[cfg(feature = "postgresql")]
    #[must_use]
    pub fn from_postgres(db: Arc<PostgresDatabase>) -> Self {
        Self {
            users: db.clone(),
            profiles: db.clone(),
            admin: db.clone(),
            api_keys: db.clone(),
            chat: db.clone(),
            coaches: db.clone(),
            fitness_config: db.clone(),
            impersonation: db.clone(),
            insights: db.clone(),
            llm_credentials: db.clone(),
            llm_usage: db.clone(),
            messaging: db.clone(),
            mobility: db.clone(),
            notifications: db.clone(),
            oauth2_server: db.clone(),
            oauth_client_state: db.clone(),
            oauth_tokens: db.clone(),
            password_reset: db.clone(),
            provider_connections: db.clone(),
            recipes: db.clone(),
            security: db.clone(),
            seeder: db.clone(),
            social: Some(db.clone()),
            store_listings: db.clone(),
            tenants: db.clone(),
            tool_selection: db.clone(),
            usage: db.clone(),
            usage_counters: db.clone(),
            user_mcp_tokens: db.clone(),
            a2a: db.clone(),
            groups: db.clone(),
            data_sources: db.clone(),
            sleep: db.clone(),
            recovery: db.clone(),
            health_snapshots: db.clone(),
            sync_cursors: db.clone(),
            memory: db.clone(),
            claim_verdicts: db.clone(),
            subscriptions: db.clone(),
            weather_cache: db.clone(),
            user_physiological_profile: db.clone(),
            dossier: db.clone(),
            training_history: db.clone(),
            route_summaries: db.clone(),
            prescribed_workouts: db.clone(),
            workout_templates: db.clone(),
            time_series_points: db,
        }
    }
}
