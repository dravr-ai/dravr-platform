// ABOUTME: View-structs over RepositoryRegistry that narrow consumer dependencies to a cohesive subset
// ABOUTME: Each view bundles repositories used together; consumers depend on the narrowest view they need
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Repository view-structs
//!
//! `RepositoryRegistry` is the master assembly — one struct with one
//! `Arc<dyn …>` per domain trait. Most consumers need only a handful of those
//! traits, and taking the full registry as a parameter advertises a much wider
//! dependency surface than the call site actually uses.
//!
//! The views in this module bundle repositories that consumers touch
//! *together*. A handler or service that needs only authentication state takes
//! [`AuthRepos`]; one that needs coach / chat / memory plumbing takes
//! [`CoachRepos`]; etc. Each view is built once from the registry via
//! `RepositoryRegistry::<view>_repos()` (cheap — every field is a clone of an
//! `Arc<dyn Trait>`).
//!
//! ## Single source of truth
//!
//! Every registry field belongs to exactly one canonical view (see the
//! per-view documentation below). When two consumers want overlapping
//! repositories the answer is to take the canonical view; when a consumer
//! truly spans multiple views, it takes both view-structs (or, for the
//! composition root, the full registry).

use std::sync::Arc;

use crate::repositories::{
    A2ARepository, AdminRepository, ApiKeyRepository, ChatRepository, ClaimVerdictRepository,
    CoachesRepository, CoachingGroupRepository, DataSourceRepository, DossierRepository,
    FeatureFlagsRepository, FitnessConfigRepository, HarnessMemoryRepository,
    HealthSnapshotRepository, ImpersonationRepository, LlmCredentialRepository, LlmUsageRepository,
    MessagingRepository, MobilityRepository, NotificationRepository, OAuth2ServerRepository,
    OAuthClientStateRepository, OAuthTokenRepository, PasswordResetRepository,
    PrescribedWorkoutRepository, ProfileRepository, ProviderConnectionRepository, RecipeRepository,
    RecoveryRepository, RosterRepository, RouteSummaryRepository, SecurityRepository,
    SeederRepository, SleepRepository, SocialRepository, StoreListingsRepository,
    SubscriptionsRepository, SyncCursorRepository, TenantRepository, ToolSelectionRepository,
    TrainingHistoryRepository, UsageCounterRepository, UsageRepository, UserMcpTokenRepository,
    UserPhysiologicalProfileRepository, UserRateLimitOverrideRepository, UserRepository,
    WeatherCacheRepository, WorkoutTemplateRepository,
};
use crate::RepositoryRegistry;
use dravr_riviere::TimeSeriesStore;

/// Repositories backing authentication, identity, and protocol-auth surfaces.
///
/// Consumers: middleware (`tenant`, `auth`), admin-route context, identity
/// route layer, OAuth backend resolver, provider-refresh service, the A2A
/// client (which authenticates via `api_keys` + `a2a` together).
///
/// Every field listed here has its canonical home in this view; other views
/// must not duplicate them.
#[derive(Clone)]
pub struct AuthRepos {
    /// User account CRUD and lookup
    pub users: Arc<dyn UserRepository>,
    /// User profile operations
    pub profiles: Arc<dyn ProfileRepository>,
    /// Tenant CRUD and user-tenant roles
    pub tenants: Arc<dyn TenantRepository>,
    /// API key CRUD and usage tracking
    pub api_keys: Arc<dyn ApiKeyRepository>,
    /// Admin token management
    pub admin: Arc<dyn AdminRepository>,
    /// Password reset token management
    pub password_reset: Arc<dyn PasswordResetRepository>,
    /// RSA keypairs, key rotation, audit events
    pub security: Arc<dyn SecurityRepository>,
    /// User OAuth provider tokens
    pub oauth_tokens: Arc<dyn OAuthTokenRepository>,
    /// OAuth client state (CSRF protection)
    pub oauth_client_state: Arc<dyn OAuthClientStateRepository>,
    /// `OAuth2` authorization server (clients, codes, tokens)
    pub oauth2_server: Arc<dyn OAuth2ServerRepository>,
    /// Admin impersonation sessions
    pub impersonation: Arc<dyn ImpersonationRepository>,
    /// User MCP token management
    pub user_mcp_tokens: Arc<dyn UserMcpTokenRepository>,
    /// Agent-to-Agent protocol clients, sessions, tasks
    pub a2a: Arc<dyn A2ARepository>,
}

impl AuthRepos {
    /// Build an `AuthRepos` view from the master registry.
    ///
    /// All fields are `Arc::clone` of the registry's `Arc<dyn Trait>` slots
    /// — cheap reference-count bumps, no data copy.
    #[must_use]
    pub fn from_registry(registry: &RepositoryRegistry) -> Self {
        Self {
            users: Arc::clone(&registry.users),
            profiles: Arc::clone(&registry.profiles),
            tenants: Arc::clone(&registry.tenants),
            api_keys: Arc::clone(&registry.api_keys),
            admin: Arc::clone(&registry.admin),
            password_reset: Arc::clone(&registry.password_reset),
            security: Arc::clone(&registry.security),
            oauth_tokens: Arc::clone(&registry.oauth_tokens),
            oauth_client_state: Arc::clone(&registry.oauth_client_state),
            oauth2_server: Arc::clone(&registry.oauth2_server),
            impersonation: Arc::clone(&registry.impersonation),
            user_mcp_tokens: Arc::clone(&registry.user_mcp_tokens),
            a2a: Arc::clone(&registry.a2a),
        }
    }
}

/// Repositories backing coach personas, conversations, harness memory, and
/// claim-verdict bookkeeping.
///
/// Consumers: chat pipeline (`chat`, `memory`, `users` — the last via
/// `AuthRepos`), claim-verdict / coach-grading / myth-busting services
/// (`claim_verdicts`), memory-facts service (`memory`), MCP resources layer
/// (`coaches`, `groups`, `store_listings`), messaging group-bind service
/// (`groups`).
#[derive(Clone)]
pub struct CoachRepos {
    /// Coach persona management
    pub coaches: Arc<dyn CoachesRepository>,
    /// Store listings for coach marketplace
    pub store_listings: Arc<dyn StoreListingsRepository>,
    /// Chat conversation and message storage
    pub chat: Arc<dyn ChatRepository>,
    /// Coaching group CRUD, membership, and invites
    pub groups: Arc<dyn CoachingGroupRepository>,
    /// Coach-athlete roster assignments
    pub roster: Arc<dyn RosterRepository>,
    /// Endurance dossier composer (read-time aggregate from physiology /
    /// goals / zones / nutrition / equipment)
    pub dossier: Arc<dyn DossierRepository>,
    /// Endurance prescribed-workout audit trail
    pub prescribed_workouts: Arc<dyn PrescribedWorkoutRepository>,
    /// Endurance user-authored workout-template overrides
    pub workout_templates: Arc<dyn WorkoutTemplateRepository>,
    /// Coaching harness memory (compaction, facts, notes, followups, sessions)
    pub memory: Arc<dyn HarnessMemoryRepository>,
    /// Tier 5.5 claim verdicts from the bullshit detector pipeline
    pub claim_verdicts: Arc<dyn ClaimVerdictRepository>,
}

impl CoachRepos {
    /// Build a `CoachRepos` view from the master registry.
    #[must_use]
    pub fn from_registry(registry: &RepositoryRegistry) -> Self {
        Self {
            coaches: Arc::clone(&registry.coaches),
            store_listings: Arc::clone(&registry.store_listings),
            chat: Arc::clone(&registry.chat),
            groups: Arc::clone(&registry.groups),
            roster: Arc::clone(&registry.roster),
            dossier: Arc::clone(&registry.dossier),
            prescribed_workouts: Arc::clone(&registry.prescribed_workouts),
            workout_templates: Arc::clone(&registry.workout_templates),
            memory: Arc::clone(&registry.memory),
            claim_verdicts: Arc::clone(&registry.claim_verdicts),
        }
    }
}

/// Repositories backing fitness state.
///
/// Covers sync cursors, provider connections, device sources, sleep /
/// recovery / health snapshots, training history, physiological profiles,
/// route summaries, and the persistent weather cache.
///
/// Consumers: the `dravr-enforme` sync adapter (`PierreSyncStorage` —
/// implements 8 enforme store traits over `sleep`, `recovery`,
/// `health_snapshots`, `data_sources`, `sync_cursors`, `time_series_points`,
/// plus `oauth_tokens` from `AuthRepos`), prefetch / training services,
/// endurance dossier composition.
#[derive(Clone)]
pub struct FitnessRepos {
    /// User fitness configuration
    pub fitness_config: Arc<dyn FitnessConfigRepository>,
    /// Provider connection tracking
    pub provider_connections: Arc<dyn ProviderConnectionRepository>,
    /// Sync cursor tracking for CDC-based incremental sync
    pub sync_cursors: Arc<dyn SyncCursorRepository>,
    /// Continuous time-series points (`data_point_series` table), via riviere's
    /// `TimeSeriesStore`
    pub time_series_points: Arc<dyn TimeSeriesStore>,
    /// Endurance cached GPX terrain + climbs per activity
    pub route_summaries: Arc<dyn RouteSummaryRepository>,
    /// Recovery metrics persistence
    pub recovery: Arc<dyn RecoveryRepository>,
    /// Sleep session persistence
    pub sleep: Arc<dyn SleepRepository>,
    /// Health snapshot persistence
    pub health_snapshots: Arc<dyn HealthSnapshotRepository>,
    /// Data source (device/provider) tracking
    pub data_sources: Arc<dyn DataSourceRepository>,
    /// Endurance daily training-state rollups (CTL/ATL/TSB/ACWR/monotony/
    /// strain/`ramp_rate`/`daily_load`)
    pub training_history: Arc<dyn TrainingHistoryRepository>,
    /// Endurance typed physiological profile (FTP, threshold pace, zones)
    pub user_physiological_profile: Arc<dyn UserPhysiologicalProfileRepository>,
    /// dravr-meteo persistent weather cache (geographic + hourly buckets)
    pub weather_cache: Arc<dyn WeatherCacheRepository>,
}

impl FitnessRepos {
    /// Build a `FitnessRepos` view from the master registry.
    #[must_use]
    pub fn from_registry(registry: &RepositoryRegistry) -> Self {
        Self {
            fitness_config: Arc::clone(&registry.fitness_config),
            provider_connections: Arc::clone(&registry.provider_connections),
            sync_cursors: Arc::clone(&registry.sync_cursors),
            time_series_points: Arc::clone(&registry.time_series_points),
            route_summaries: Arc::clone(&registry.route_summaries),
            recovery: Arc::clone(&registry.recovery),
            sleep: Arc::clone(&registry.sleep),
            health_snapshots: Arc::clone(&registry.health_snapshots),
            data_sources: Arc::clone(&registry.data_sources),
            training_history: Arc::clone(&registry.training_history),
            user_physiological_profile: Arc::clone(&registry.user_physiological_profile),
            weather_cache: Arc::clone(&registry.weather_cache),
        }
    }
}

/// Repositories backing social graph, multi-channel messaging, and OAuth
/// completion notifications.
///
/// Consumers: messaging ingress / linking / OTP / dispatch / session
/// services, social feature endpoints, notification dispatch.
#[derive(Clone)]
pub struct SocialRepos {
    /// Social features (friend connections, shared insights, reactions).
    /// `None` when social features are not enabled at startup.
    pub social: Option<Arc<dyn SocialRepository>>,
    /// Multi-channel messaging gateway
    pub messaging: Arc<dyn MessagingRepository>,
    /// OAuth completion notifications
    pub notifications: Arc<dyn NotificationRepository>,
}

impl SocialRepos {
    /// Build a `SocialRepos` view from the master registry.
    #[must_use]
    pub fn from_registry(registry: &RepositoryRegistry) -> Self {
        Self {
            social: registry.social.as_ref().map(Arc::clone),
            messaging: Arc::clone(&registry.messaging),
            notifications: Arc::clone(&registry.notifications),
        }
    }
}

/// Repositories backing usage accounting, rate-limiting overrides, LLM cost
/// tracking, feature flags, tool selection, and Stripe subscriptions.
///
/// Consumers: websocket usage panel, admin rate-limit override handlers,
/// admin feature-flag handlers, tool-runtime `ToolSelectionService`,
/// billing routes (subscriptions + invoices), LLM accounting service.
#[derive(Clone)]
pub struct UsageRepos {
    /// Usage analytics and statistics
    pub usage: Arc<dyn UsageRepository>,
    /// Rate-limiting usage counters
    pub usage_counters: Arc<dyn UsageCounterRepository>,
    /// Runtime feature-flag storage
    pub feature_flags: Arc<dyn FeatureFlagsRepository>,
    /// Per-tenant MCP tool configuration
    pub tool_selection: Arc<dyn ToolSelectionRepository>,
    /// Per-user rate-limit overrides
    pub user_rate_limit_overrides: Arc<dyn UserRateLimitOverrideRepository>,
    /// LLM provider credential management
    pub llm_credentials: Arc<dyn LlmCredentialRepository>,
    /// LLM usage tracking for cost analysis
    pub llm_usage: Arc<dyn LlmUsageRepository>,
    /// Stripe-backed subscription rows
    pub subscriptions: Arc<dyn SubscriptionsRepository>,
}

impl UsageRepos {
    /// Build a `UsageRepos` view from the master registry.
    #[must_use]
    pub fn from_registry(registry: &RepositoryRegistry) -> Self {
        Self {
            usage: Arc::clone(&registry.usage),
            usage_counters: Arc::clone(&registry.usage_counters),
            feature_flags: Arc::clone(&registry.feature_flags),
            tool_selection: Arc::clone(&registry.tool_selection),
            user_rate_limit_overrides: Arc::clone(&registry.user_rate_limit_overrides),
            llm_credentials: Arc::clone(&registry.llm_credentials),
            llm_usage: Arc::clone(&registry.llm_usage),
            subscriptions: Arc::clone(&registry.subscriptions),
        }
    }
}

/// Repositories backing content libraries — recipes, mobility (stretching
/// + yoga), and the seed-only bootstrap surface.
///
/// Consumers: MCP resources layer (recipe + mobility listings), seeder
/// binaries.
#[derive(Clone)]
pub struct ContentRepos {
    /// Recipe CRUD with nutrition
    pub recipes: Arc<dyn RecipeRepository>,
    /// Stretching exercises and yoga poses
    pub mobility: Arc<dyn MobilityRepository>,
    /// Seed-only database operations
    pub seeder: Arc<dyn SeederRepository>,
}

impl ContentRepos {
    /// Build a `ContentRepos` view from the master registry.
    #[must_use]
    pub fn from_registry(registry: &RepositoryRegistry) -> Self {
        Self {
            recipes: Arc::clone(&registry.recipes),
            mobility: Arc::clone(&registry.mobility),
            seeder: Arc::clone(&registry.seeder),
        }
    }
}

impl RepositoryRegistry {
    /// Build an [`AuthRepos`] view from this registry. Cheap — clones
    /// `Arc<dyn Trait>` for each contained slot.
    #[must_use]
    pub fn auth_repos(&self) -> AuthRepos {
        AuthRepos::from_registry(self)
    }

    /// Build a [`CoachRepos`] view from this registry.
    #[must_use]
    pub fn coach_repos(&self) -> CoachRepos {
        CoachRepos::from_registry(self)
    }

    /// Build a [`FitnessRepos`] view from this registry.
    #[must_use]
    pub fn fitness_repos(&self) -> FitnessRepos {
        FitnessRepos::from_registry(self)
    }

    /// Build a [`SocialRepos`] view from this registry.
    #[must_use]
    pub fn social_repos(&self) -> SocialRepos {
        SocialRepos::from_registry(self)
    }

    /// Build a [`UsageRepos`] view from this registry.
    #[must_use]
    pub fn usage_repos(&self) -> UsageRepos {
        UsageRepos::from_registry(self)
    }

    /// Build a [`ContentRepos`] view from this registry.
    #[must_use]
    pub fn content_repos(&self) -> ContentRepos {
        ContentRepos::from_registry(self)
    }
}
