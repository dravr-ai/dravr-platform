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
//! like Strava and Garmin.
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
//! - `SportType`: Enumeration of supported activity types

// Domain modules
pub mod a2a;
mod athlete;
/// The difficulty-calibration interview's topic table and next-topic policy.
pub mod calibration;
/// How often the coach checks in with an athlete on its own (carnet#831).
pub mod check_in;
/// A member's provider read through their group coach's session: the
/// `delegated_connections` row and its lifecycle vocabulary.
pub mod delegated_connection;
/// Endurance athlete dossier composed at read time from physiology, goals,
/// zones and nutrition slots.
pub mod dossier;
/// The fuel-product bank: catalogue products, versioned label facts, the
/// review queue, what an athlete carries, and the plausibility gate.
pub mod fuel;
/// The athlete's gear ledger: declared shoes, per-activity attribution and
/// the default pair a new activity accrues to.
pub mod gear;
/// The profile-recorded window of an athlete's most recent fixed-list walk.
pub mod guided_window;
mod health;
/// Measured-vs-estimated provenance carried by a physiological value.
pub mod measurement;
mod nutrition;
mod oauth;
/// Guided-flow state + the two next-topic policies (coverage, calibration list).
pub mod onboarding;
/// The canonical six fitness-adapted health pillars (single source of truth
/// for per-user context, pillar-tagged facts, and the OKF bundle).
pub mod pillar;
/// What a server-initiated agent post is for: its claim row's purpose.
pub mod platform_post;
mod provider_thresholds;
/// The season walk's topic table and next-topic policy.
pub mod season;
mod sleep;
/// The coarse sport family a provider sport type belongs to.
pub mod sport_family;
mod tenant;
mod tool_selection;
/// Endurance daily training-state rollup (`DailyTrainingState`) backing
/// the `training_history` table and the training-history tools.
pub mod training_history;
/// The athlete's unit system (metric or imperial) and the one rule that resolves it.
pub mod units;
mod user;
/// Endurance calendar-ledger shapes (`PlannedSession`, `CalendarKey`,
/// `PrescribedWorkout`) backing the `prescribed_workouts` table; the workout
/// template itself is the periodization kernel's.
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
    Activity, ActivityBuilder, ActivityComment, Feel, HeartRateZone, Lap, PowerZone, SegmentEffort,
    Split, TimeSeriesData,
};
pub use dravr_cageux::models::sport;
pub use dravr_cageux::models::sport::SportType;
// A session a provider's calendar plans for the athlete, in the periodization
// grammar — read back from TrainingPeaks and Intervals.icu, from any calendar
// provider that declares the planned-workouts capability.
pub use dravr_cageux::models::planned_workout::{PlannedWorkout, PlannedWorkoutBuilder};
// Form banding lives in the sports-science engine (dravr-cageux) so every
// surface reads the same edges; see `FormBand` for why raw TSB is never banded.
pub use dravr_cageux::training_load::FormBand;

/// One reading of an athlete's form — the single serializer every TSB surface
/// renders through, so a bare absolute number cannot reach an agent again.
pub mod form_reading;
pub use form_reading::{FormInterpretation, FormReading};

mod sport_type_alias;
pub use sport_type_alias::{resolve_sport_type, sport_family_head, sport_matches_family};

mod sport_profile;
pub use sport_profile::SportProfile;

// Sleep domain
pub use sleep::{SleepSession, SleepStage, SleepStageType};

// Health domain
pub use health::{HealthMetrics, RecoveryMetrics};

// Data source domain (from dravr-equilibre)
pub use data_source::{DataSource, DeviceType};

// Provider data refresh domain
pub use refresh::{
    DataFreshness, ProviderFreshness, RefreshConfig, RefreshStatus, ScheduledRefreshConfig,
    SmartScheduleWeights,
};

// Stored health models for persistence (from dravr-equilibre)
pub use dravr_equilibre::{
    StoredHealthMetrics, StoredRecoveryMetrics, StoredSleepSession, SyncResult, SyncStatus,
};

// One record per night or date across providers, primary source first and
// each metric it lacks taken from the next source that has it.
pub use dravr_equilibre::{
    merge_health_metrics, merge_recovery_metrics, merge_sleep_sessions, Merged,
    SleepStage as StoredSleepStage, SleepStageType as StoredSleepStageType,
};

// Nutrition domain
pub use nutrition::{FoodItem, MealEntry, MealType, NutritionLog};

// Athlete domain
pub use athlete::{Athlete, PeriodTotals, Stats, StatsCoverage};
pub use delegated_connection::{
    DelegatedConnection, DelegationEndReason, DelegationStatus, RosterAccess, RosterAthlete,
};
pub use units::UnitSystem;

pub use check_in::CheckInCadence;

// Physiology provenance domain
pub use measurement::{
    MeasurementKind, MetricProvenance, ProvenancedValue, ThresholdProvenance,
    ATHLETE_REPORTED_ORIGIN,
};
pub use provider_thresholds::{ProviderThreshold, ProviderThresholds};

// User domain
pub use user::{
    default_locale, locale_from_accept_language, normalize_email, page_locale, roster_display_name,
    supported_ui_locale, CoachingPersona, ColorScheme, OperatorRosterGrant, PhysiologicalSex,
    PreApprovedEmail, User, UserId, UserInvitation, UserPhysiologicalProfile, UserReference,
    UserReferenceKind, UserStatus, UserTier, FEDERATED_ONLY_PASSWORD_HASH, SUPPORTED_LOCALES,
};

// Endurance zones + dossier + training-history + workout-template domain
pub use calibration::{CalibrationConditions, CalibrationTopic, LONG_SESSION_MIN};
pub use dossier::{Dossier, DossierFact};
pub use fuel::{
    check_plausibility, fuel_natural_key, image_sha256, normalize_gtin, proposal_source_hash,
    AthleteFuelItem, CatalogueFuelListing, CatalogueFuelProduct, FactBasis, FactProvenance,
    FactSource, FieldOverride, FieldProvenance, FuelApprovalOutcome, FuelFactOverrides, FuelFacts,
    FuelForm, FuelItemCarried, FuelItemEntry, FuelKeyHolder, FuelProduct, FuelProductProposal,
    FuelProductVersion, FuelProposalImageMeta, FuelRejectReason, NewFuelProposal, PendingFuelItem,
    PlausibilityViolation, ProposalSource, ProposalSourceImage, ProposalStatus, ILLEGIBLE_FLAG,
    MAX_CAFFEINE_MG_PER_SERVING,
    MAX_FUEL_ITEMS_PER_ATHLETE, MAX_FUEL_PRODUCT_LOOKUP, MAX_GLUCOSE_FRUCTOSE_RATIO,
    MAX_SODIUM_MG_PER_SERVING,
};
pub use gear::{
    default_pair, parse_shoe_sport, shoe_sport_slug, ActivityGearKey, AthleteGear,
    AttributionSource, GearAttribution, GearDeclaration, GearKind,
    DEFAULT_SHOE_WARN_DISTANCE_METERS, MAX_GEAR_PER_ATHLETE,
};
pub use guided_window::GuidedWindow;
pub use onboarding::{
    CoverageMap, CoverageTarget, GuidedFlow, LoadSnapshot, OnboardingState, TopicSlug,
    TopicVisibility, WalkAudience, COMPLETION_RELEASE_WINDOW_MINUTES, MAX_PROBE_ATTEMPTS,
};
pub use pillar::Pillar;
pub use platform_post::PostPurpose;
pub use season::{SeasonConditions, SeasonTopic};
pub use sport_family::SportFamily;
pub use training_history::{DailyTrainingKey, DailyTrainingState};
pub use workout_template::{
    CalendarEventRef, CalendarEventSource, CalendarKey, PlannedSession, PlannedSessionKind,
    PrescribedWorkout,
};
pub use zones::{HrZoneSet, PowerZoneSet, ZoneDistribution};

// Periodization kernel (dravr-cageux). The workout template and its intensity
// vocabulary live there, next to the flavours, season skeletons and selection
// table that reference them. The flat names stay resolvable at
// `pierre_core::models::X` so every platform crate keeps one import path for
// the types it used before the move; the module re-export reaches the rest.
/// The whole periodization kernel — every catalogue type (`Flavour`,
/// `SkeletonTemplate`, `SelectionTable`, `WorkoutParams`, `PhaseFit`,
/// `WorkoutFilter`, the vocabulary enums, `CatalogueError`) reachable as
/// `pierre_core::models::periodization::X`, so a platform crate never adds
/// its own `dravr-cageux` dependency to reach one.
pub use dravr_cageux::periodization;
pub use dravr_cageux::periodization::{
    FuelingProtocol, IntensityDistribution, RelativeIntensity, WorkoutStep, WorkoutTargetZones,
    WorkoutTemplate,
};

// OAuth domain
pub use oauth::{
    connection_needs_reauth, refresh_due, AuthRequest, AuthResponse, ConnectionStatus,
    ConnectionType, DecryptedToken, EncryptedToken, OAuthAppCredentials, OAuthNotification,
    ProviderAccountRole, ProviderConnection, ReauthMark, StravaPoolApp, StravaSeatHolder,
    StravaSeatReclaimWarning, StravaTokenApp, UserOAuthApp, UserOAuthToken, UserSession,
    API_KEY_TOKEN_TYPE,
};

// OAuth client state for provider authorization flows
mod oauth_client;
pub use oauth_client::OAuthClientState;

// Tenant domain
pub use tenant::{
    LlmCredentialRecord, LlmCredentialSummary, OAuthClientGrant, Tenant, TenantId,
    TenantOAuthCredentials,
};

// Tool selection domain
pub use tool_selection::{
    CategorySummary, EffectiveTool, SetToolOverrideRequest, TenantPlan, TenantToolOverride,
    ToolAvailabilitySummary, ToolCatalogEntry, ToolCategory, ToolEnablementSource,
};

// OAuth 2.0 server persistence models
mod oauth2_server;
pub use oauth2_server::{
    DeviceAuthorization, OAuth2AuthCode, OAuth2Client, OAuth2ClientSweep, OAuth2RefreshToken,
    OAuth2State,
};

// First-party refresh tokens — the credential a device holds between JWTs
mod session_refresh_token;
pub use session_refresh_token::SessionRefreshToken;

// Coach-access requests — a coach asks, a super-admin decides (carnet#738)
mod coach_access;
pub use coach_access::{CoachAccessRequest, CoachAccessStatus};

// A coach's after-the-fact review of the agent's group replies (carnet#847)
mod coach_review;
pub use coach_review::{
    CoachReply, CoachReplyCorrection, CoachReplyFilter, CoachReplyFlag, CoachReplyReview,
    CoachReviewWeek, NewReplyCorrection, NewReplyFlag, ReplyFlagReason, ReplyReviewKind,
    ReplyReviewStamp,
};

// Signup codes — a shareable code that approves whoever enters it (carnet#1128)
mod signup_code;
pub use signup_code::{SignupCode, SignupCodeRedemption, SignupCodeSource};

// iOS App Attest keys — the installs of Dravr's iOS app that Apple attested
mod app_attest_key;
pub use app_attest_key::{AppAttestEnvironment, AppAttestKey};

// User MCP token types for AI client authentication
mod user_mcp_token;
pub use user_mcp_token::{
    CreateUserMcpTokenRequest, UserMcpToken, UserMcpTokenCreated, UserMcpTokenInfo,
    ValidatedUserMcpToken,
};

// Chat conversation and message record types
mod conversation;
// A coach's quote of their athlete's group-room words (carnet#1249)
mod room_quote;
pub use conversation::{
    is_in_app_channel, is_turn_marker_block, split_visuals, AddMessageParams, AgentRuntimeContext,
    ConversationLastMessage, ConversationPage, ConversationParticipant, ConversationRecord,
    ConversationSubject, ConversationSummary, ConversationTurnId, InputSource,
    MessageFeedbackRecord, MessagePage, MessageRecord, ParticipantRole, PersistedAction,
    PersistedReplyBlock, ThreadTopic, TurnOrigin, UpsertMessageFeedbackParams, ACTIONS_BLOCK_TYPE,
    AGENT_WELCOME_FINISH_REASON, CHANNEL_TYPE_MOBILE, CHANNEL_TYPE_WEB, COMMAND_FINISH_REASON,
    DIVIDER_FINISH_REASON, FILTERED_REPLY_FINISH_REASON, STOPPED_TURN_FINISH_REASON,
    STOP_CAVEAT_SEPARATOR, TRUNCATED_REPLY_FINISH_REASON, TURN_AUTHOR_BLOCK_TYPE,
    TURN_STOP_BLOCK_TYPE, UNVERIFIED_CAPABILITY_CLAIM_FINISH_REASON, WITHHELD_REPLY_FINISH_REASON,
};
pub use room_quote::{RoomQuote, ROOM_QUOTE_BLOCK_TYPE};

// API key types for authentication and rate limiting
mod api_key;
pub use api_key::{
    ApiKey, ApiKeyData, ApiKeyResponse, ApiKeyTier, ApiKeyUsage, ApiKeyUsageStats,
    CreateApiKeyRequest, CreateApiKeyRequestSimple,
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
    ConversationTurnLlmCall, ConversationTurnSummary, InsertLlmUsage, JwtMonthlyUsage, JwtUsage,
    LlmUsageAggregateRow, LlmUsageDailyRow, LlmUsageRecord, MonthlyLimitOverride, RequestLog,
    ToolUsage, UsageCounterRecord, WindowUsage, TURN_SUMMARY_CALL_TYPE,
};

/// An agent package's training artefacts — flavour, skeleton, workouts — beside its prompt
pub mod agent_artefacts;
/// Agent (AI persona) data types for custom AI agent personas
pub mod agents;
pub use agent_artefacts::{
    sha256_hex, AgentArtefact, ArtefactKind, PackageArtefact, ParsedArtefact,
};
/// Mobility domain types for stretching and yoga
pub mod mobility;
pub use agents::{
    Agent, AgentAssignment, AgentCategory, AgentHandle, AgentListItem, AgentPrerequisites,
    AgentVersion, AgentVisibility, CreateAgentRequest, CreateSystemAgentRequest, ListAgentsFilter,
    PublishStatus, StoreAdminStats, UpdateAgentRequest,
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
    CoachConsents, CoachingGroup, CreateGroupRequest, FreshMember, GroupAggregateStats,
    GroupContext, GroupHealthFlag, GroupInvite, GroupMember, GroupRole, GroupSummary,
    GroupSummaryBlock, GroupTranscriptEntry, GroupTrend, GroupWeeklyReport, HealthFlagSeverity,
    MemberFitnessSnapshot, MemberFlag, MemberSummaryCard, NewGroupTranscriptEntry,
    OvertrainingRiskLevel, SummaryDetailLevel, TranscriptSpeaker, UpdateGroupRequest,
};
pub use notifications::NotificationScreen;
