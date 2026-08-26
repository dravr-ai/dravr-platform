// ABOUTME: Core data models and types for the Pierre fitness API
// ABOUTME: Re-exports Activity, User, SportType and other fundamental data structures
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Data Models
//!
// NOTE: All `.clone()` calls in this module are Safe - they are necessary for:
// - HashMap key ownership for statistics aggregation (stage_type.clone())
// - Data structure ownership transfers across model boundaries
//!
//! This module contains the core data structures used throughout the Pierre MCP Server.
//! These models provide a unified representation of fitness data from various providers
//! like Strava and Fitbit.
//!
//! ## Design Principles
//!
//! - **Provider Agnostic**: Models abstract away provider-specific differences
//! - **Extensible**: Optional fields accommodate different provider capabilities
//! - **Serializable**: All models support JSON serialization for MCP protocol
//! - **Type Safe**: Strong typing prevents common data handling errors
//!
//! ## Core Models
//!
//! - `Activity`: Represents a single fitness activity (run, ride, etc.)
//! - `Athlete`: User profile information
//! - `Stats`: Aggregated fitness statistics
//! - `PersonalRecord`: Individual performance records
//! - `SportType`: Enumeration of supported activity types

// Domain modules
pub mod a2a;
mod athlete;
/// The difficulty-calibration interview's topic table and next-topic policy.
pub mod calibration;
/// Endurance athlete dossier composed at read time from physiology, goals,
/// zones, nutrition, and equipment slots.
pub mod dossier;
mod health;
mod nutrition;
mod oauth;
/// Guided-flow state + the two next-topic policies (coverage, calibration list).
pub mod onboarding;
/// The canonical six fitness-adapted health pillars (single source of truth
/// for per-user context, pillar-tagged facts, and the OKF bundle).
pub mod pillar;
/// Coach-athlete roster assignment shape backing `coach_athlete_assignments`.
pub mod roster;
mod sleep;
mod tenant;
mod tool_selection;
/// Endurance daily training-state rollup (`DailyTrainingState`) backing
/// the `training_history` table and `GET /api/v1/endurance/history`.
pub mod training_history;
mod user;
/// Endurance Phase 5 workout-template + prescription audit shapes
/// backing the `workout_templates` + `prescribed_workouts` tables.
pub mod workout_template;
/// Endurance per-user training-zone boundaries (HR + power) and the
/// zone-distribution aggregate computed across one or more activities.
pub mod zones;

/// Data source and device tracking (from dravr-equilibre)
pub mod data_source;

/// Provider data freshness and refresh configuration
pub mod refresh;

// Activity and sport types come from dravr-cageux (canonical source)
pub use dravr_cageux::models::activity;
pub use dravr_cageux::models::activity::{
    Activity, ActivityBuilder, HeartRateZone, Lap, PowerZone, SegmentEffort, Split, TimeSeriesData,
};
pub use dravr_cageux::models::sport;
pub use dravr_cageux::models::sport::SportType;
// Form banding lives in the sports-science engine (dravr-cageux) so every
// surface reads the same edges; see `FormBand` for why raw TSB is never banded.
pub use dravr_cageux::training_load::FormBand;

mod sport_type_alias;
pub use sport_type_alias::resolve_sport_type;

mod sport_profile;
pub use sport_profile::SportProfile;

// Sleep domain
pub use sleep::{SleepSession, SleepStage, SleepStageType};

// Health domain
pub use health::{HealthMetrics, RecoveryMetrics};

// Data source domain (from dravr-equilibre)
pub use data_source::{DataSource, DevicePriority, DeviceType, ProviderPriority};

// Provider data refresh domain
pub use refresh::{
    DataFreshness, ProviderFreshness, RefreshConfig, RefreshStatus, ScheduledRefreshConfig,
    SmartScheduleWeights,
};

// Stored health models for persistence (from dravr-equilibre)
pub use dravr_equilibre::{
    EventCategory, EventRecord, StoredHealthMetrics, StoredRecoveryMetrics, StoredSleepSession,
    SyncResult, SyncStatus, WorkoutDetails as StoredWorkoutDetails,
};

// Nutrition domain
pub use nutrition::{FoodItem, MealEntry, MealType, NutritionLog};

// Athlete domain
pub use athlete::{Athlete, PeriodTotals, PersonalRecord, PrMetric, Stats};
pub use roster::CoachAthleteAssignment;

// User domain
pub use user::{
    default_locale, CoachingPersona, ColorScheme, PreApprovedEmail, User, UserId,
    UserPhysiologicalProfile, UserStatus, UserTier,
};

// Endurance zones + dossier + training-history + workout-template domain
pub use calibration::{CalibrationConditions, CalibrationTopic};
pub use dossier::{Dossier, DossierFact};
pub use onboarding::{
    CoverageMap, CoverageTarget, GuidedFlow, LoadSnapshot, OnboardingState, TopicSlug,
    COMPLETION_RELEASE_WINDOW_MINUTES, MAX_PROBE_ATTEMPTS,
};
pub use pillar::Pillar;
pub use training_history::{DailyTrainingKey, DailyTrainingState};
pub use workout_template::{
    IntensityDistribution, PrescribedWorkout, WorkoutStep, WorkoutTargetZones, WorkoutTemplate,
};
pub use zones::{HrZoneSet, PowerZoneSet, ZoneDistribution};

// OAuth domain
pub use oauth::{
    connection_needs_reauth, AuthRequest, AuthResponse, ConnectionStatus, ConnectionType,
    DecryptedToken, EncryptedToken, OAuthAppCredentials, OAuthNotification, ProviderConnection,
    StravaPoolApp, UserOAuthApp, UserOAuthToken, UserSession,
};

// OAuth client state for provider authorization flows
mod oauth_client;
pub use oauth_client::OAuthClientState;

// Tenant domain
pub use tenant::{
    AuthorizationCode, LlmCredentialRecord, LlmCredentialSummary, OAuthApp, OAuthAppParams,
    OAuthClientGrant, Tenant, TenantId, TenantOAuthCredentials,
};

// Tool selection domain
pub use tool_selection::{
    CategorySummary, EffectiveTool, SetToolOverrideRequest, TenantPlan, TenantToolOverride,
    ToolAvailabilitySummary, ToolCatalogEntry, ToolCategory, ToolEnablementSource,
};

// OAuth 2.0 server persistence models
mod oauth2_server;
pub use oauth2_server::{
    DeviceAuthorization, OAuth2AuthCode, OAuth2Client, OAuth2RefreshToken, OAuth2State,
};

// User MCP token types for AI client authentication
mod user_mcp_token;
pub use user_mcp_token::{
    CreateUserMcpTokenRequest, UserMcpToken, UserMcpTokenCreated, UserMcpTokenInfo,
};

// Chat conversation and message record types
mod conversation;
pub use conversation::{
    split_visuals, AddMessageParams, CoachRuntimeContext, ConversationParticipant,
    ConversationRecord, ConversationSummary, ConversationTurnId, MessageFeedbackRecord,
    MessageRecord, ParticipantRole, UpsertMessageFeedbackParams,
    UNVERIFIED_CAPABILITY_CLAIM_FINISH_REASON, WITHHELD_REPLY_FINISH_REASON,
};

// Security audit event types
mod audit;
pub use audit::{AuditEvent, AuditEventType, AuditSeverity};

// API key types for authentication and rate limiting
mod api_key;
pub use api_key::{
    ApiKey, ApiKeyData, ApiKeyResponse, ApiKeyTier, ApiKeyUsage, ApiKeyUsageStats,
    CreateApiKeyRequest, CreateApiKeyRequestSimple, RateLimitStatus,
};

// A2A protocol data types
pub use a2a::{
    A2AClient, A2APushNotificationConfig, A2ASession, A2ATask, A2AUsage, A2AUsageStats, TaskStatus,
};

/// Usage, dashboard, and quota tracking types.
pub mod usage;

/// Tier-keyed quota configuration (Starter / Professional / Enterprise).
pub mod tier_quota;
pub use tier_quota::{TierQuotaConfig, ENTERPRISE, PROFESSIONAL, STARTER};

/// Stripe-backed subscription domain model.
pub mod subscription;
pub use subscription::{Subscription, SubscriptionStatus};
pub use usage::{
    ConversationTurnLlmCall, ConversationTurnSummary, EmbeddingUsageRecord, InsertEmbeddingUsage,
    InsertLlmUsage, JwtUsage, LlmUsageAggregateRow, LlmUsageDailyRow, LlmUsageRecord, RequestLog,
    ToolUsage, UsageCounterRecord, TURN_SUMMARY_CALL_TYPE,
};

/// Coach (AI persona) data types for custom AI coaching personas
pub mod coaches;
/// Mobility domain types for stretching and yoga
pub mod mobility;
pub use coaches::{
    Coach, CoachAssignment, CoachCategory, CoachHandle, CoachListItem, CoachPrerequisites,
    CoachVersion, CoachVisibility, CreateCoachRequest, CreateSystemCoachRequest, ListCoachesFilter,
    PublishStatus, StoreAdminStats, UpdateCoachRequest,
};
/// Recipe data models for nutrition planning with training-aware meal timing (from dravr-cageux)
pub use dravr_cageux::models::recipes;
pub use mobility::{
    ActivityMuscleMapping, DifficultyLevel, ListStretchingFilter, ListYogaFilter,
    StretchingCategory, StretchingExercise, YogaCategory, YogaPose, YogaPoseType,
};
pub use recipes::{
    DietaryRestriction, IngredientUnit, MacroTargets, MealTiming, Recipe, RecipeConstraints,
    RecipeIngredient, SkillLevel, ValidatedNutrition,
};

/// Multi-channel messaging gateway types
pub mod messaging;

/// Coaching group models for multi-person AI coaching
pub mod groups;
/// Notification screen vocabulary — the app-side destinations a push notification names
pub mod notifications;
pub use groups::{
    CoachingGroup, CreateGroupRequest, GroupAggregateStats, GroupContext, GroupHealthFlag,
    GroupInvite, GroupMember, GroupRole, GroupSummary, GroupSummaryBlock, GroupTranscriptEntry,
    GroupTrend, GroupWeeklyReport, HealthFlagSeverity, JoinGroupRequest, MemberFitnessSnapshot,
    MemberFlag, MemberSummaryCard, NewGroupTranscriptEntry, OvertrainingRiskLevel,
    SummaryDetailLevel, TranscriptSpeaker, UpdateGroupRequest,
};
pub use notifications::NotificationScreen;
