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
    A2ARepository, ActivityCacheRepository, AdminRepository, ApiKeyRepository, ChatRepository,
    ClaimVerdictRepository, CoachesRepository, CoachingGroupRepository, CommitmentRepository,
    DataSourceRepository, DossierRepository, EmailVerificationRepository, FeatureFlagsRepository,
    FitnessConfigRepository, GuardianPendingActionsRepository, HarnessMemoryRepository,
    HealthSnapshotRepository, ImpersonationRepository, LlmCredentialRepository, LlmUsageRepository,
    McpTaskRepository, MessagingRepository, MobilityRepository, NotificationRepository,
    OAuth2ServerRepository, OAuthClientStateRepository, OAuthTokenRepository,
    PasswordResetRepository, PlaybookRepository, PreApprovedEmailRepository,
    PrescribedWorkoutRepository, ProfileRepository, ProviderConnectionRepository, RecipeRepository,
    RecoveryRepository, RosterRepository, RouteSummaryRepository, SecurityRepository,
    SeederRepository, ShortLinkRepository, SleepRepository, StoreListingsRepository,
    SubscriptionsRepository, SyncCursorRepository, TenantRepository, ToolSelectionRepository,
    TrainingHistoryRepository, TrainingPlanRepository, UsageCounterRepository, UsageRepository,
    UserMcpTokenRepository, UserOnboardingRepository, UserPhysiologicalProfileRepository,
    UserRateLimitOverrideRepository, UserRepository, UserTierOverrideRepository,
    UserToolOverrideRepository, WeatherCacheRepository, WorkoutTemplateRepository,
};
use dravr_riviere::TimeSeriesStore;

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
    /// Athlete commitments swept against real activity data
    pub commitments: Arc<dyn CommitmentRepository>,
    /// User fitness configuration
    pub fitness_config: Arc<dyn FitnessConfigRepository>,
    /// Admin impersonation sessions
    pub impersonation: Arc<dyn ImpersonationRepository>,
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
    /// Email-verification token management
    pub email_verification: Arc<dyn EmailVerificationRepository>,
    /// Password reset token management
    pub password_reset: Arc<dyn PasswordResetRepository>,
    /// Standing per-email pre-approvals consulted by the registration approval
    /// decision; managed by `pierre-cli user allow / disallow / list-allowed`
    pub pre_approved_emails: Arc<dyn PreApprovedEmailRepository>,
    /// Provider connection tracking
    pub provider_connections: Arc<dyn ProviderConnectionRepository>,
    /// Recipe CRUD with nutrition
    pub recipes: Arc<dyn RecipeRepository>,
    /// RSA keypairs, key rotation, audit events
    pub security: Arc<dyn SecurityRepository>,
    /// Seed-only database operations
    pub seeder: Arc<dyn SeederRepository>,
    /// Procedural coaching memory: learned `trigger -> intervention` playbooks + pending advice
    pub playbooks: Arc<dyn PlaybookRepository>,
    /// Coach-authored training plans: outline (macrocycle) + weekly microcycles
    pub training_plans: Arc<dyn TrainingPlanRepository>,
    /// URL shortener: `code` → `target_url` for `WhatsApp`-clickable chat links
    pub short_links: Arc<dyn ShortLinkRepository>,
    /// Durable per-user onboarding step completion state (server-driven onboarding flow)
    pub user_onboarding: Arc<dyn UserOnboardingRepository>,
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
    /// Claim verdicts from the bullshit detector pipeline
    pub claim_verdicts: Arc<dyn ClaimVerdictRepository>,
    /// MCP Tasks extension handles (io.modelcontextprotocol/tasks)
    pub mcp_tasks: Arc<dyn McpTaskRepository>,
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
    /// Endurance user-authored workout templates (the catalogue bank lives in TOML)
    pub workout_templates: Arc<dyn WorkoutTemplateRepository>,
    /// Continuous time-series points (`data_point_series` table). Implements
    /// riviere's `TimeSeriesStore`; backs the dravr-enforme write adapter.
    pub time_series_points: Arc<dyn TimeSeriesStore>,
    /// Coach-athlete roster assignments (1:N junction). Gates routes that
    /// require `manages_roster=true` and surfaces who coaches whom.
    pub roster: Arc<dyn RosterRepository>,
    /// Per-user rate-limit overrides (industry-standard exemption pattern).
    /// Row presence wins over `UserTier::monthly_limit()` in admin views and
    /// the rate-limit middleware.
    pub user_rate_limit_overrides: Arc<dyn UserRateLimitOverrideRepository>,
    /// Per-user admin tier override marker. Row presence makes the billing
    /// webhook skip `set_tier`/`set_plan` so a Stripe event cannot clobber a
    /// manual operator override.
    pub user_tier_overrides: Arc<dyn UserTierOverrideRepository>,
    /// Per-user admin tool override. Row presence force-enables/disables a
    /// single MCP tool for one user, overlaid above the tenant tool-selection
    /// computation (below `PIERRE_DISABLED_TOOLS`, above plan + tenant override).
    pub user_tool_overrides: Arc<dyn UserToolOverrideRepository>,
    /// Runtime feature-flag storage. Backs `/api/me/features` and the admin
    /// per-tenant/per-user toggle endpoints.
    pub feature_flags: Arc<dyn FeatureFlagsRepository>,
    /// Guardian pending actions parked by `TaintedDestructive::Confirm`,
    /// claimed single-use by the `/confirm` and `/deny` slash commands.
    pub guardian_actions: Arc<dyn GuardianPendingActionsRepository>,
    /// Provider-agnostic activity cache. Backs stale-while-revalidate reads on
    /// the chat path so a slow scrape (Garmin/sciotte) or redundant API call
    /// never blocks a turn.
    pub activity_cache: Arc<dyn ActivityCacheRepository>,
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
            commitments: db.clone(),
            fitness_config: db.clone(),
            impersonation: db.clone(),
            llm_credentials: db.clone(),
            llm_usage: db.clone(),
            messaging: db.clone(),
            mobility: db.clone(),
            notifications: db.clone(),
            oauth2_server: db.clone(),
            oauth_client_state: db.clone(),
            oauth_tokens: db.clone(),
            email_verification: db.clone(),
            password_reset: db.clone(),
            pre_approved_emails: db.clone(),
            provider_connections: db.clone(),
            recipes: db.clone(),
            security: db.clone(),
            seeder: db.clone(),
            playbooks: db.clone(),
            training_plans: db.clone(),
            short_links: db.clone(),
            user_onboarding: db.clone(),
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
            mcp_tasks: db.clone(),
            subscriptions: db.clone(),
            weather_cache: db.clone(),
            user_physiological_profile: db.clone(),
            dossier: db.clone(),
            training_history: db.clone(),
            route_summaries: db.clone(),
            prescribed_workouts: db.clone(),
            workout_templates: db.clone(),
            time_series_points: db.clone(),
            roster: db.clone(),
            user_rate_limit_overrides: db.clone(),
            user_tier_overrides: db.clone(),
            user_tool_overrides: db.clone(),
            activity_cache: db.clone(),
            feature_flags: db.clone(),
            guardian_actions: db,
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
            commitments: db.clone(),
            fitness_config: db.clone(),
            impersonation: db.clone(),
            llm_credentials: db.clone(),
            llm_usage: db.clone(),
            messaging: db.clone(),
            mobility: db.clone(),
            notifications: db.clone(),
            oauth2_server: db.clone(),
            oauth_client_state: db.clone(),
            oauth_tokens: db.clone(),
            email_verification: db.clone(),
            password_reset: db.clone(),
            pre_approved_emails: db.clone(),
            provider_connections: db.clone(),
            recipes: db.clone(),
            security: db.clone(),
            seeder: db.clone(),
            playbooks: db.clone(),
            training_plans: db.clone(),
            short_links: db.clone(),
            user_onboarding: db.clone(),
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
            mcp_tasks: db.clone(),
            subscriptions: db.clone(),
            weather_cache: db.clone(),
            user_physiological_profile: db.clone(),
            dossier: db.clone(),
            training_history: db.clone(),
            route_summaries: db.clone(),
            prescribed_workouts: db.clone(),
            workout_templates: db.clone(),
            time_series_points: db.clone(),
            roster: db.clone(),
            user_rate_limit_overrides: db.clone(),
            user_tier_overrides: db.clone(),
            user_tool_overrides: db.clone(),
            activity_cache: db.clone(),
            feature_flags: db.clone(),
            guardian_actions: db,
        }
    }
}
