// ABOUTME: Repository trait definitions and DatabaseProvider compound supertrait for database abstraction
// ABOUTME: Defines focused, cohesive repository traits that backends implement directly
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::admin::jwt::JwtSigner;
use pierre_core::admin::models::{
    AdminConfigOverrideRow, AdminToken, AdminTokenUsage, CreateAdminTokenRequest,
    GeneratedAdminToken,
};
use pierre_core::config::FitnessConfig;
use pierre_core::errors::AppResult;
use pierre_core::models::a2a::{
    A2AClient, A2ASession, A2ATask, A2AUsage, A2AUsageStats, TaskStatus,
};
use pierre_core::models::coaches::{
    Coach, CoachAssignment, CoachCategory, CoachListItem, CoachVersion, CreateCoachRequest,
    CreateSystemCoachRequest, ListCoachesFilter, StoreAdminStats, UpdateCoachRequest,
};
use pierre_core::models::groups::{
    CoachingGroup, GroupInvite, GroupMember, GroupRole, GroupSummary, UpdateGroupRequest,
};
use pierre_core::models::mobility::{
    ActivityMuscleMapping, ListStretchingFilter, ListYogaFilter, StretchingExercise, YogaPose,
};

use pierre_core::models::recipes::{MealTiming, Recipe, ValidatedNutrition};
use pierre_core::models::usage::{
    EmbeddingUsageRecord, InsertEmbeddingUsage, InsertLlmUsage, LlmUsageAggregateRow,
    LlmUsageDailyRow,
};
use pierre_core::models::DataSource;
use pierre_core::models::{
    AdaptedInsight, ApiKey, ApiKeyUsage, ApiKeyUsageStats, AuthorizationCode, CoachRuntimeContext,
    CoachingPersona, ConnectionType, ConversationRecord, ConversationSummary,
    CreateUserMcpTokenRequest, FriendConnection, FriendStatus, InsightReaction, OAuth2AuthCode,
    OAuth2Client, OAuth2RefreshToken, OAuth2State, OAuthApp, OAuthClientState, OAuthNotification,
    PrescribedWorkout, ProviderConnection, SharedInsight, Tenant, TenantPlan, TenantToolOverride,
    ToolCatalogEntry, ToolCategory, User, UserMcpToken, UserMcpTokenCreated, UserMcpTokenInfo,
    UserOAuthApp, UserOAuthToken, UserSocialSettings, UserStatus, UserTier,
};
use pierre_core::models::{
    AddMessageParams, ConversationTurnId, JwtUsage, LlmUsageRecord, RequestLog, ToolUsage,
    UsageCounterRecord,
};
use pierre_core::models::{
    AuditEvent, KeyVersion, LlmCredentialRecord, LlmCredentialSummary, MessageRecord, Subscription,
    SubscriptionStatus, TenantId, TenantOAuthCredentials,
};
use pierre_core::models::{
    DailyTrainingState, Dossier, StoredHealthMetrics, StoredRecoveryMetrics, StoredSleepSession,
    UserPhysiologicalProfile, WorkoutTemplate,
};
use pierre_core::pagination::{CursorPage, PaginationParams, StoreSortOrder};
use pierre_core::permissions::impersonation::ImpersonationSession;
use pierre_memory::{ClaimCategory, ClaimStatus, ClaimVerdict, EvidenceStrength, VerdictLayer};
use serde_json::Value;
use std::collections::HashMap;
use uuid::Uuid;

use serde::{Deserialize, Serialize};

use crate::database::store_listings::{CoachWithListing, StoreListing};
use crate::seed_models::SeedCoachTranslation;
use crate::seed_models::{
    SeedA2AClient, SeedA2AUsage, SeedAdaptedInsight, SeedApiKey, SeedApiKeyUsage, SeedCoach,
    SeedCoachAuthor, SeedCoachRelation, SeedDemoUser, SeedFriendConnection, SeedInsightReaction,
    SeedLlmUsageRecord, SeedProviderConnection, SeedSharedInsight, SeedSocialSettings,
    SeedStoreListing, SeedSyntheticActivity, SeedTenant,
};

// ================================
// Sync Cursor Row Types
// ================================

/// Database row for `sync_state` table
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncCursorRow {
    /// Unique identifier for this sync state record
    pub id: String,
    /// User who owns this sync cursor
    pub user_id: String,
    /// Tenant context for multi-tenant isolation
    pub tenant_id: String,
    /// Provider being synced (e.g., "whoop", "garmin")
    pub provider: String,
    /// Data type being synced (e.g., "sleep", "recovery", "health")
    pub data_type: String,
    /// Provider-specific cursor token for pagination
    pub cursor_value: Option<String>,
    /// When the last sync completed (RFC 3339)
    pub last_sync_at: Option<String>,
    /// Current sync status (pending, completed, failed)
    pub last_sync_status: String,
    /// Total records synced in the last batch
    pub records_synced: i64,
    /// Error message from the last failed sync
    pub error_message: Option<String>,
    /// Number of consecutive retry attempts
    pub retry_count: i64,
    /// When to attempt the next retry (RFC 3339)
    pub next_retry_at: Option<String>,
}

/// Connected user info for sync scheduling
#[derive(Debug, Clone)]
pub struct ConnectedUserRow {
    /// User identifier
    pub user_id: String,
    /// Tenant identifier
    pub tenant_id: String,
}

// ================================
// Repository Trait Definitions
// ================================

/// User account management repository
#[async_trait]
pub trait UserRepository: Send + Sync {
    /// Create a new user account
    async fn create(&self, user: &User) -> AppResult<Uuid>;
    /// Get user by ID, scoped to a specific tenant for multi-tenant isolation
    async fn get(&self, user_id: Uuid, tenant_id: TenantId) -> AppResult<Option<User>>;
    /// Get user by ID without tenant scoping (for system-level operations)
    async fn get_global(&self, user_id: Uuid) -> AppResult<Option<User>>;
    /// Get user by email address
    async fn get_by_email(&self, email: &str) -> AppResult<Option<User>>;
    /// Get user by email (required - fails if not found)
    async fn get_by_email_required(&self, email: &str) -> AppResult<User>;
    /// Get user by Firebase UID
    async fn get_by_firebase_uid(&self, firebase_uid: &str) -> AppResult<Option<User>>;
    /// Update user's last active timestamp
    async fn update_last_active(&self, user_id: Uuid) -> AppResult<()>;
    /// Get total number of users
    async fn count(&self) -> AppResult<i64>;
    /// Get users by status (pending, active, suspended), optionally scoped to a tenant
    async fn get_by_status(
        &self,
        status: &str,
        tenant_id: Option<TenantId>,
    ) -> AppResult<Vec<User>>;
    /// Get users by status with cursor-based pagination
    async fn get_by_status_cursor(
        &self,
        status: &str,
        params: &PaginationParams,
    ) -> AppResult<CursorPage<User>>;
    /// Update user status and approval information
    async fn update_status(
        &self,
        user_id: Uuid,
        new_status: UserStatus,
        approved_by: Option<Uuid>,
    ) -> AppResult<User>;
    /// Set admin status on a user, updating both `is_admin` flag and role column.
    /// When granting admin, users keep `SuperAdmin` role if they already have it; otherwise set to `Admin`.
    /// When revoking admin, role is reset to `User`. Super-admins cannot be demoted via this method.
    async fn set_admin_status(&self, user_id: Uuid, is_admin: bool) -> AppResult<User>;
    /// List all users with `is_admin = true`, ordered by email
    async fn list_admins(&self) -> AppResult<Vec<User>>;
    /// Update user's `tenant_id` to link them to a tenant (`tenant_id` should be UUID string)
    async fn update_tenant_id(&self, user_id: Uuid, tenant_id: TenantId) -> AppResult<()>;
    /// Update user's password hash
    async fn update_password(&self, user_id: Uuid, password_hash: &str) -> AppResult<()>;
    /// Update user's display name
    async fn update_display_name(&self, user_id: Uuid, display_name: &str) -> AppResult<User>;
    /// Delete a user and all associated data
    async fn delete(&self, user_id: Uuid) -> AppResult<()>;
    /// Get the first admin user by creation date
    async fn get_first_admin_user(&self) -> AppResult<Option<User>>;
    /// Check if a user has synthetic activities seeded
    async fn has_synthetic_activities(&self, user_id: Uuid) -> AppResult<bool>;
    /// Update user's analytics consent preference
    async fn update_analytics_consent(&self, user_id: Uuid, enabled: bool) -> AppResult<()>;
    /// Update the user's preferred locale (BCP-47 short code, e.g. `"fr"`, `"en"`).
    ///
    /// Called by the user-profile PATCH endpoint. The column has `NOT NULL
    /// DEFAULT 'fr'` so an unset user always resolves to French; this method
    /// overrides that default with an explicit choice.
    async fn update_locale(&self, user_id: Uuid, locale: &str) -> AppResult<()>;
    /// Set the user's personal default coach (nullable — pass `None` to clear).
    ///
    /// Called by `/coach select` in DM conversations. The column has a FK on
    /// `coaches(id)` with `ON DELETE SET NULL`, so a deleted coach cleanly
    /// detaches instead of orphaning the user row.
    async fn set_default_coach(&self, user_id: Uuid, coach_id: Option<&str>) -> AppResult<()>;
    /// Set the user's coaching persona (output-format / cadence preference).
    ///
    /// Called by the post-auth onboarding screen and the Settings UI. The
    /// `coaching_persona` column has `NOT NULL DEFAULT 'casual'` so an
    /// unmigrated user always resolves to the least-restrictive style;
    /// this method overrides that default with an explicit choice.
    async fn set_coaching_persona(&self, user_id: Uuid, persona: CoachingPersona) -> AppResult<()>;
    /// Set the user's billing tier (Starter / Professional / Enterprise).
    ///
    /// Called by Stripe webhook handlers on `customer.subscription.updated`
    /// and by the admin `POST /api/admin/users/{id}/tier` route. The CHECK
    /// constraint on `users.tier` is enforced at write time.
    async fn set_tier(&self, user_id: Uuid, tier: UserTier) -> AppResult<User>;
}

/// OAuth token storage repository (tenant-scoped, includes OAuth apps and sync tracking)
#[async_trait]
pub trait OAuthTokenRepository: Send + Sync {
    /// Store or update user OAuth token for a tenant-provider combination
    async fn upsert_token(&self, token: &UserOAuthToken) -> AppResult<()>;
    /// Get user OAuth token for a specific tenant-provider combination
    async fn get_token(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) -> AppResult<Option<UserOAuthToken>>;
    /// Get all OAuth tokens for a user, optionally scoped to a specific tenant
    async fn get_tokens(
        &self,
        user_id: Uuid,
        tenant_id: Option<TenantId>,
    ) -> AppResult<Vec<UserOAuthToken>>;
    /// Get all OAuth tokens for a tenant-provider combination
    async fn get_tenant_provider_tokens(
        &self,
        tenant_id: TenantId,
        provider: &str,
    ) -> AppResult<Vec<UserOAuthToken>>;
    /// Delete user OAuth token for a tenant-provider combination
    async fn delete_token(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) -> AppResult<()>;
    /// Delete all OAuth tokens for a user within a tenant scope
    async fn delete_tokens(&self, user_id: Uuid, tenant_id: TenantId) -> AppResult<()>;
    /// Update OAuth token expiration and refresh info
    async fn refresh_token(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        access_token: &str,
        refresh_token: Option<&str>,
        expires_at: Option<DateTime<Utc>>,
    ) -> AppResult<()>;
    /// Store user OAuth app credentials (`client_id`, `client_secret`)
    async fn store_user_oauth_app(
        &self,
        user_id: Uuid,
        provider: &str,
        client_id: &str,
        client_secret: &str,
        redirect_uri: &str,
    ) -> AppResult<()>;
    /// Get user OAuth app credentials for a provider
    async fn get_user_oauth_app(
        &self,
        user_id: Uuid,
        provider: &str,
    ) -> AppResult<Option<UserOAuthApp>>;
    /// List all OAuth app providers configured for a user
    async fn list_user_oauth_apps(&self, user_id: Uuid) -> AppResult<Vec<UserOAuthApp>>;
    /// Remove user OAuth app credentials for a provider
    async fn remove_user_oauth_app(&self, user_id: Uuid, provider: &str) -> AppResult<()>;
    /// Get last sync timestamp for a provider within a specific tenant
    async fn get_provider_last_sync(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) -> AppResult<Option<DateTime<Utc>>>;
    /// Update last sync timestamp for a provider within a specific tenant
    async fn update_provider_last_sync(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        sync_time: DateTime<Utc>,
    ) -> AppResult<()>;
}

/// API key management repository
#[async_trait]
pub trait ApiKeyRepository: Send + Sync {
    /// Create a new API key
    async fn create(&self, api_key: &ApiKey) -> AppResult<()>;
    /// Get API key by its prefix and hash
    async fn get_by_prefix(&self, prefix: &str, hash: &str) -> AppResult<Option<ApiKey>>;
    /// Get all API keys for a user
    async fn get_for_user(&self, user_id: Uuid) -> AppResult<Vec<ApiKey>>;
    /// Update API key last used timestamp
    async fn update_last_used(&self, api_key_id: &str) -> AppResult<()>;
    /// Deactivate an API key
    async fn deactivate(&self, api_key_id: &str, user_id: Uuid) -> AppResult<()>;
    /// Get API key by ID, optionally scoped to a specific user for ownership enforcement
    async fn get_by_id(&self, api_key_id: &str, user_id: Option<Uuid>)
        -> AppResult<Option<ApiKey>>;
    /// Get API keys with optional filters
    async fn get_filtered(
        &self,
        user_email: Option<&str>,
        active_only: bool,
        limit: Option<i32>,
        offset: Option<i32>,
    ) -> AppResult<Vec<ApiKey>>;
    /// Clean up expired API keys
    async fn cleanup_expired(&self) -> AppResult<u64>;
    /// Get expired API keys
    async fn get_expired(&self) -> AppResult<Vec<ApiKey>>;
}

/// Usage tracking and analytics repository
#[async_trait]
pub trait UsageRepository: Send + Sync {
    /// Record API key usage
    async fn record_api_key(&self, usage: &ApiKeyUsage) -> AppResult<()>;
    /// Get current usage count for an API key
    async fn get_api_key_current(&self, api_key_id: &str) -> AppResult<u32>;
    /// Get usage statistics for an API key
    async fn get_api_key_stats(
        &self,
        api_key_id: &str,
        start_date: DateTime<Utc>,
        end_date: DateTime<Utc>,
    ) -> AppResult<ApiKeyUsageStats>;
    /// Record JWT token usage for rate limiting and analytics
    async fn record_jwt_usage(&self, usage: &JwtUsage) -> AppResult<()>;
    /// Get current JWT usage count for rate limiting (current month)
    async fn get_jwt_current_usage(&self, user_id: Uuid) -> AppResult<u32>;
    /// Get request logs with filtering options
    async fn get_request_logs(
        &self,
        user_id: Option<Uuid>,
        api_key_id: Option<&str>,
        start_time: Option<DateTime<Utc>>,
        end_time: Option<DateTime<Utc>>,
        status_filter: Option<&str>,
        tool_filter: Option<&str>,
    ) -> AppResult<Vec<RequestLog>>;
    /// Get system statistics, optionally scoped to a tenant
    async fn get_system_stats(&self, tenant_id: Option<TenantId>) -> AppResult<(u64, u64)>;
    /// Get top tools analysis for dashboard
    async fn get_top_tools_analysis(
        &self,
        user_id: Uuid,
        start_time: DateTime<Utc>,
        end_time: DateTime<Utc>,
    ) -> AppResult<Vec<ToolUsage>>;
}

/// A2A (Agent-to-Agent) client and session management repository
#[async_trait]
pub trait A2ARepository: Send + Sync {
    /// Create a new A2A client
    async fn create_client(
        &self,
        client: &A2AClient,
        client_secret: &str,
        api_key_id: &str,
    ) -> AppResult<String>;
    /// Get A2A client by ID
    async fn get_client(&self, client_id: &str) -> AppResult<Option<A2AClient>>;
    /// Get A2A client by API key ID
    async fn get_client_by_api_key_id(&self, api_key_id: &str) -> AppResult<Option<A2AClient>>;
    /// Get A2A client by name
    async fn get_client_by_name(&self, name: &str) -> AppResult<Option<A2AClient>>;
    /// List all A2A clients for a user
    async fn list_clients(&self, user_id: &Uuid) -> AppResult<Vec<A2AClient>>;
    /// Deactivate an A2A client
    async fn deactivate_client(&self, client_id: &str) -> AppResult<()>;
    /// Get client credentials for authentication
    async fn get_client_credentials(&self, client_id: &str) -> AppResult<Option<(String, String)>>;
    /// Invalidate all active sessions for a client
    async fn invalidate_client_sessions(&self, client_id: &str) -> AppResult<()>;
    /// Deactivate all API keys associated with a client
    async fn deactivate_client_api_keys(&self, client_id: &str) -> AppResult<()>;
    /// Create a new A2A session
    async fn create_session(
        &self,
        client_id: &str,
        user_id: Option<&Uuid>,
        granted_scopes: &[String],
        expires_in_hours: i64,
    ) -> AppResult<String>;
    /// Get A2A session by token
    async fn get_session(&self, session_token: &str) -> AppResult<Option<A2ASession>>;
    /// Update A2A session activity timestamp
    async fn update_session_activity(&self, session_token: &str) -> AppResult<()>;
    /// Get active sessions for a specific client
    async fn get_active_sessions(&self, client_id: &str) -> AppResult<Vec<A2ASession>>;
    /// Create a new A2A task
    async fn create_task(
        &self,
        client_id: &str,
        session_id: Option<&str>,
        task_type: &str,
        input_data: &Value,
    ) -> AppResult<String>;
    /// Get A2A task by ID
    async fn get_task(&self, task_id: &str) -> AppResult<Option<A2ATask>>;
    /// List A2A tasks for a client with optional filtering
    async fn list_tasks(
        &self,
        client_id: Option<&str>,
        status_filter: Option<&TaskStatus>,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> AppResult<Vec<A2ATask>>;
    /// Update A2A task status
    async fn update_task_status(
        &self,
        task_id: &str,
        status: &TaskStatus,
        result: Option<&Value>,
        error: Option<&str>,
    ) -> AppResult<()>;
    /// Record A2A usage for analytics
    async fn record_usage(&self, usage: &A2AUsage) -> AppResult<()>;
    /// Get current A2A usage count for a client
    async fn get_client_current_usage(&self, client_id: &str) -> AppResult<u32>;
    /// Get A2A usage statistics for a client
    async fn get_usage_stats(
        &self,
        client_id: &str,
        start_date: DateTime<Utc>,
        end_date: DateTime<Utc>,
    ) -> AppResult<A2AUsageStats>;
    /// Get A2A client usage history
    async fn get_client_usage_history(
        &self,
        client_id: &str,
        days: u32,
    ) -> AppResult<Vec<(DateTime<Utc>, u32, u32)>>;
}

/// User profiles, goals, and configuration repository
#[async_trait]
pub trait ProfileRepository: Send + Sync {
    /// Upsert user profile data
    async fn upsert_profile(&self, user_id: Uuid, profile_data: Value) -> AppResult<()>;
    /// Get user profile data
    async fn get_profile(&self, user_id: Uuid) -> AppResult<Option<Value>>;
    /// Create a new goal for a user
    async fn create_goal(&self, user_id: Uuid, goal_data: Value) -> AppResult<String>;
    /// Get all goals for a user
    async fn get_goals(&self, user_id: Uuid) -> AppResult<Vec<Value>>;
    /// Update progress on a goal, scoped to the owning user
    async fn update_goal_progress(
        &self,
        goal_id: &str,
        user_id: Uuid,
        current_value: f64,
    ) -> AppResult<()>;
    /// Get user configuration data
    async fn get_configuration(&self, user_id: &str) -> AppResult<Option<String>>;
    /// Save user configuration data
    async fn save_configuration(&self, user_id: &str, config_json: &str) -> AppResult<()>;
}

/// AI-generated insights storage repository
#[async_trait]
pub trait InsightRepository: Send + Sync {
    /// Store an AI-generated insight
    async fn store(&self, user_id: Uuid, insight_data: Value) -> AppResult<String>;
    /// Get insights for a user
    async fn get_for_user(
        &self,
        user_id: Uuid,
        insight_type: Option<&str>,
        limit: Option<u32>,
    ) -> AppResult<Vec<Value>>;
}

/// Admin token management repository
#[async_trait]
pub trait AdminRepository: Send + Sync {
    /// Create a new admin token
    async fn create_token(
        &self,
        request: &CreateAdminTokenRequest,
        admin_jwt_secret: &str,
        jwks_manager: &dyn JwtSigner,
    ) -> AppResult<GeneratedAdminToken>;
    /// Get admin token by ID
    async fn get_token_by_id(&self, token_id: &str) -> AppResult<Option<AdminToken>>;
    /// Get admin token by prefix for fast lookup
    async fn get_token_by_prefix(&self, token_prefix: &str) -> AppResult<Option<AdminToken>>;
    /// List all admin tokens (super admin only)
    async fn list_tokens(&self, include_inactive: bool) -> AppResult<Vec<AdminToken>>;
    /// Deactivate admin token
    async fn deactivate_token(&self, token_id: &str) -> AppResult<()>;
    /// Update admin token last used timestamp
    async fn update_token_last_used(
        &self,
        token_id: &str,
        ip_address: Option<&str>,
    ) -> AppResult<()>;
    /// Record admin token usage for audit trail
    async fn record_token_usage(&self, usage: &AdminTokenUsage) -> AppResult<()>;
    /// Get admin token usage history
    async fn get_token_usage_history(
        &self,
        token_id: &str,
        start_date: DateTime<Utc>,
        end_date: DateTime<Utc>,
    ) -> AppResult<Vec<AdminTokenUsage>>;
    /// Record API key provisioning by admin
    async fn record_provisioned_key(
        &self,
        admin_token_id: &str,
        api_key_id: &str,
        user_email: &str,
        tier: &str,
        rate_limit_requests: u32,
        rate_limit_period: &str,
    ) -> AppResult<()>;
    /// Get admin provisioned keys history
    async fn get_provisioned_keys(
        &self,
        admin_token_id: Option<&str>,
        start_date: DateTime<Utc>,
        end_date: DateTime<Utc>,
    ) -> AppResult<Vec<Value>>;
}

/// Multi-tenant management repository
#[async_trait]
pub trait TenantRepository: Send + Sync {
    /// Create a new tenant
    async fn create(&self, tenant: &Tenant) -> AppResult<()>;
    /// Get tenant by ID
    async fn get_by_id(&self, tenant_id: TenantId) -> AppResult<Tenant>;
    /// Get tenant by slug
    async fn get_by_slug(&self, slug: &str) -> AppResult<Tenant>;
    /// List tenants for a user
    async fn list_for_user(&self, user_id: Uuid) -> AppResult<Vec<Tenant>>;
    /// Store tenant OAuth credentials
    async fn store_oauth_credentials(&self, credentials: &TenantOAuthCredentials) -> AppResult<()>;
    /// Get tenant OAuth providers
    async fn get_oauth_providers(
        &self,
        tenant_id: TenantId,
    ) -> AppResult<Vec<TenantOAuthCredentials>>;
    /// Get tenant OAuth credentials for specific provider
    async fn get_oauth_credentials(
        &self,
        tenant_id: TenantId,
        provider: &str,
    ) -> AppResult<Option<TenantOAuthCredentials>>;
    /// Create OAuth application for MCP clients
    async fn create_oauth_app(&self, app: &OAuthApp) -> AppResult<()>;
    /// Get OAuth app by client ID
    async fn get_oauth_app_by_client_id(&self, client_id: &str) -> AppResult<OAuthApp>;
    /// List OAuth apps for a user
    async fn list_oauth_apps_for_user(&self, user_id: Uuid) -> AppResult<Vec<OAuthApp>>;
    /// Get all tenants for key rotation check
    async fn get_all(&self) -> AppResult<Vec<Tenant>>;
    /// Get user role for a specific tenant
    async fn get_user_role(&self, user_id: Uuid, tenant_id: TenantId) -> AppResult<Option<String>>;
    /// Set the tenant's billing plan (Starter / Professional / Enterprise).
    ///
    /// Called by Stripe webhook handlers when a subscription state change
    /// implies a plan-tier flip on the tenant. Owner-driven plan changes
    /// always cascade through this method so audit logging stays uniform.
    async fn set_plan(&self, tenant_id: TenantId, plan: &str) -> AppResult<Tenant>;
}

/// OAuth 2.0 server repository (RFC 7591)
#[async_trait]
pub trait OAuth2ServerRepository: Send + Sync {
    /// Store OAuth 2.0 client registration
    async fn store_client(&self, client: &OAuth2Client) -> AppResult<()>;
    /// Get OAuth 2.0 client by `client_id`
    async fn get_client(&self, client_id: &str) -> AppResult<Option<OAuth2Client>>;
    /// Store OAuth 2.0 authorization code
    async fn store_auth_code(&self, auth_code: &OAuth2AuthCode) -> AppResult<()>;
    /// Get OAuth 2.0 authorization code
    async fn get_auth_code(&self, code: &str) -> AppResult<Option<OAuth2AuthCode>>;
    /// Update OAuth 2.0 authorization code (mark as used)
    async fn update_auth_code(&self, auth_code: &OAuth2AuthCode) -> AppResult<()>;
    /// Store OAuth 2.0 refresh token
    async fn store_refresh_token(&self, refresh_token: &OAuth2RefreshToken) -> AppResult<()>;
    /// Get OAuth 2.0 refresh token
    async fn get_refresh_token(&self, token: &str) -> AppResult<Option<OAuth2RefreshToken>>;
    /// Revoke OAuth 2.0 refresh token
    async fn revoke_refresh_token(&self, token: &str) -> AppResult<()>;
    /// Atomically consume OAuth 2.0 authorization code (check-and-set in single operation)
    async fn consume_auth_code(
        &self,
        code: &str,
        client_id: &str,
        redirect_uri: &str,
        now: DateTime<Utc>,
    ) -> AppResult<Option<OAuth2AuthCode>>;
    /// Atomically consume OAuth 2.0 refresh token (check-and-revoke in single operation)
    async fn consume_refresh_token(
        &self,
        token: &str,
        client_id: &str,
        now: DateTime<Utc>,
    ) -> AppResult<Option<OAuth2RefreshToken>>;
    /// Look up a refresh token by its value (without `client_id` constraint)
    async fn get_refresh_token_by_value(
        &self,
        token: &str,
    ) -> AppResult<Option<OAuth2RefreshToken>>;
    /// Store authorization code
    async fn store_authorization_code(
        &self,
        code: &str,
        client_id: &str,
        redirect_uri: &str,
        scope: &str,
        user_id: Uuid,
    ) -> AppResult<()>;
    /// Get authorization code data
    async fn get_authorization_code(&self, code: &str) -> AppResult<AuthorizationCode>;
    /// Delete authorization code (after use)
    async fn delete_authorization_code(&self, code: &str) -> AppResult<()>;
    /// Store `OAuth2` state for CSRF protection
    async fn store_state(&self, state: &OAuth2State) -> AppResult<()>;
    /// Consume `OAuth2` state (atomically check and mark as used)
    async fn consume_state(
        &self,
        state_value: &str,
        client_id: &str,
        now: DateTime<Utc>,
    ) -> AppResult<Option<OAuth2State>>;
}

/// Security, key rotation, and audit repository
#[async_trait]
pub trait SecurityRepository: Send + Sync {
    /// Save RSA keypair to database for persistence across restarts
    async fn save_rsa_keypair(
        &self,
        kid: &str,
        private_key_pem: &str,
        public_key_pem: &str,
        created_at: DateTime<Utc>,
        is_active: bool,
        key_size_bits: i32,
    ) -> AppResult<()>;
    /// Load all RSA keypairs from database
    async fn load_rsa_keypairs(
        &self,
    ) -> AppResult<Vec<(String, String, String, DateTime<Utc>, bool)>>;
    /// Update active status of RSA keypair
    async fn update_rsa_keypair_active_status(&self, kid: &str, is_active: bool) -> AppResult<()>;
    /// Store key version metadata
    async fn store_key_version(&self, version: &KeyVersion) -> AppResult<()>;
    /// Get all key versions for a tenant
    async fn get_key_versions(&self, tenant_id: Option<TenantId>) -> AppResult<Vec<KeyVersion>>;
    /// Get current active key version for a tenant
    async fn get_current_key_version(
        &self,
        tenant_id: Option<TenantId>,
    ) -> AppResult<Option<KeyVersion>>;
    /// Update key version status (activate/deactivate)
    async fn update_key_version_status(
        &self,
        tenant_id: Option<TenantId>,
        version: u32,
        is_active: bool,
    ) -> AppResult<()>;
    /// Delete old key versions
    async fn delete_old_key_versions(
        &self,
        tenant_id: Option<TenantId>,
        keep_count: u32,
    ) -> AppResult<u64>;
    /// Store audit event
    async fn store_audit_event(&self, event: &AuditEvent) -> AppResult<()>;
    /// Get audit events with filters
    async fn get_audit_events(
        &self,
        tenant_id: Option<TenantId>,
        event_type: Option<&str>,
        limit: Option<u32>,
    ) -> AppResult<Vec<AuditEvent>>;
    /// Get or create system secret (generates if not exists)
    async fn get_or_create_system_secret(&self, secret_type: &str) -> AppResult<String>;
    /// Get existing system secret
    async fn get_system_secret(&self, secret_type: &str) -> AppResult<String>;
    /// Update system secret (for rotation)
    async fn update_system_secret(&self, secret_type: &str, new_value: &str) -> AppResult<()>;
    /// Encrypt data with AAD (Additional Authenticated Data)
    ///
    /// # Errors
    /// Returns an error if encryption fails (invalid key, nonce generation failure)
    fn encrypt_data_with_aad(&self, data: &str, aad: &str) -> AppResult<String>;
    /// Decrypt data with AAD
    ///
    /// # Errors
    /// Returns an error if decryption fails (invalid data, AAD mismatch, tampered data)
    fn decrypt_data_with_aad(&self, encrypted: &str, aad: &str) -> AppResult<String>;
}

/// OAuth notification repository
#[async_trait]
pub trait NotificationRepository: Send + Sync {
    /// Store OAuth completion notification for MCP resource delivery
    async fn store(
        &self,
        user_id: Uuid,
        provider: &str,
        success: bool,
        message: &str,
        expires_at: Option<&str>,
    ) -> AppResult<String>;
    /// Get unread OAuth notifications for a user
    async fn get_unread(&self, user_id: Uuid) -> AppResult<Vec<OAuthNotification>>;
    /// Mark OAuth notification as read
    async fn mark_read(&self, notification_id: &str, user_id: Uuid) -> AppResult<bool>;
    /// Mark all OAuth notifications as read for a user
    async fn mark_all_read(&self, user_id: Uuid) -> AppResult<u64>;
    /// Get all OAuth notifications for a user (read and unread)
    async fn get_all(&self, user_id: Uuid, limit: Option<i64>)
        -> AppResult<Vec<OAuthNotification>>;
}

/// Fitness configuration management repository
#[async_trait]
pub trait FitnessConfigRepository: Send + Sync {
    /// Save tenant-level fitness configuration
    async fn save_tenant_config(
        &self,
        tenant_id: TenantId,
        configuration_name: &str,
        config: &FitnessConfig,
    ) -> AppResult<String>;
    /// Save user-specific fitness configuration
    async fn save_user_config(
        &self,
        tenant_id: TenantId,
        user_id: &str,
        configuration_name: &str,
        config: &FitnessConfig,
    ) -> AppResult<String>;
    /// Get tenant-level fitness configuration
    async fn get_tenant_config(
        &self,
        tenant_id: TenantId,
        configuration_name: &str,
    ) -> AppResult<Option<FitnessConfig>>;
    /// Get user-specific fitness configuration
    async fn get_user_config(
        &self,
        tenant_id: TenantId,
        user_id: &str,
        configuration_name: &str,
    ) -> AppResult<Option<FitnessConfig>>;
    /// List all tenant-level fitness configuration names
    async fn list_tenant_configurations(&self, tenant_id: TenantId) -> AppResult<Vec<String>>;
    /// List all user-specific fitness configuration names
    async fn list_user_configurations(
        &self,
        tenant_id: TenantId,
        user_id: &str,
    ) -> AppResult<Vec<String>>;
    /// Delete fitness configuration (tenant or user-specific)
    async fn delete_config(
        &self,
        tenant_id: TenantId,
        user_id: Option<&str>,
        configuration_name: &str,
    ) -> AppResult<bool>;
}

/// Chat conversation and message management repository
#[async_trait]
pub trait ChatRepository: Send + Sync {
    /// Create a new chat conversation.
    ///
    /// `coach_id` references a coach in the `coaches` table; the coach's
    /// `system_prompt` is the canonical persona source and is resolved at
    /// runtime via [`CoachesRepository::get_coach_runtime_context`].
    async fn create_conversation(
        &self,
        user_id: &str,
        tenant_id: TenantId,
        title: &str,
        model: &str,
        coach_id: Option<&str>,
    ) -> AppResult<ConversationRecord>;
    /// Get a conversation by ID with user/tenant isolation
    async fn get_conversation(
        &self,
        conversation_id: &str,
        user_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<Option<ConversationRecord>>;
    /// List conversations for a user with pagination
    async fn list_conversations(
        &self,
        user_id: &str,
        tenant_id: TenantId,
        limit: i64,
        offset: i64,
    ) -> AppResult<Vec<ConversationSummary>>;
    /// Update conversation title
    async fn update_conversation_title(
        &self,
        conversation_id: &str,
        user_id: &str,
        tenant_id: TenantId,
        title: &str,
    ) -> AppResult<bool>;
    /// Delete a conversation and its messages
    async fn delete_conversation(
        &self,
        conversation_id: &str,
        user_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<bool>;
    /// Add a message to a conversation (verifies user owns the conversation)
    async fn add_message(&self, params: &AddMessageParams<'_>) -> AppResult<MessageRecord>;
    /// Get all messages for a conversation (verifies user owns the conversation)
    async fn get_messages(
        &self,
        conversation_id: &str,
        user_id: &str,
    ) -> AppResult<Vec<MessageRecord>>;
    /// Get recent messages for a conversation (verifies user owns the conversation)
    async fn get_recent_messages(
        &self,
        conversation_id: &str,
        user_id: &str,
        limit: i64,
    ) -> AppResult<Vec<MessageRecord>>;
    /// Get message count for a conversation (verifies user owns the conversation)
    async fn get_message_count(&self, conversation_id: &str, user_id: &str) -> AppResult<i64>;
    /// Count total conversations for a user in a tenant
    async fn count_conversations(&self, user_id: &str, tenant_id: TenantId) -> AppResult<i64>;
    /// Delete all conversations for a user
    async fn delete_all_user_conversations(
        &self,
        user_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<i64>;

    /// Get recently updated conversations across all tenants (admin view)
    ///
    /// Returns the last `limit` conversations ordered by `updated_at` descending.
    /// Includes the associated `user_id` for display. Used by the admin activity dashboard.
    async fn get_recent_conversations_admin(
        &self,
        limit: i64,
    ) -> AppResult<Vec<ConversationRecord>>;

    /// Count conversations updated since a given timestamp (admin view, cross-tenant)
    async fn count_active_conversations_since(&self, since: &str) -> AppResult<i64>;

    /// Attach a coach session id to an existing conversation row (Tier 4
    /// cross-channel continuity). Tenant-scoped; returns `false` if the
    /// conversation does not exist.
    async fn set_conversation_session_id(
        &self,
        conversation_id: &str,
        session_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<bool>;
}

// ============================================================================
// Coaching harness memory repository (Tier 0 foundations)
// ============================================================================

/// Parameters for a [`HarnessMemoryRepository::upsert_user_fact`] call.
///
/// Grouped into a struct to avoid a trait method with seven positional
/// arguments.
pub struct UpsertUserFactParams<'a> {
    /// Tenant that owns the fact.
    pub tenant_id: TenantId,
    /// User the fact is about.
    pub user_id: &'a str,
    /// Coach the fact belongs to, or `None` for a user-wide fact.
    pub coach_id: Option<&'a str>,
    /// Scope bucket.
    pub scope: pierre_memory::MemoryScope,
    /// Semantic kind.
    pub kind: pierre_memory::FactKind,
    /// Subject phrase (typically "you" or an entity name).
    pub subject: &'a str,
    /// Predicate phrase.
    pub predicate: &'a str,
    /// Object phrase.
    pub object: &'a str,
    /// Confidence in `[0.0, 1.0]`.
    pub confidence: f32,
    /// Source message id for provenance.
    pub source_msg_id: Option<&'a str>,
    /// Optional embedding vector (little-endian f32 at the DB layer).
    pub embedding: Option<&'a [f32]>,
}

/// Parameters for persisting a compaction block.
pub struct InsertCompactionBlockParams<'a> {
    /// Tenant that owns the block.
    pub tenant_id: TenantId,
    /// Conversation the block belongs to.
    pub conversation_id: &'a str,
    /// Summary text that replaces the compacted turns.
    pub summary: &'a str,
    /// Token count of the summary.
    pub summary_tokens: i32,
    /// Approximate token count of the raw turns the block replaces.
    pub original_tokens: i32,
    /// First (oldest) compacted message id, inclusive.
    pub first_message_id: &'a str,
    /// Last (newest) compacted message id, inclusive.
    pub last_message_id: &'a str,
}

/// Parameters for creating a coach note via the harness memory tools.
pub struct InsertCoachNoteParams<'a> {
    /// Tenant that owns the note.
    pub tenant_id: TenantId,
    /// User the note is about.
    pub user_id: &'a str,
    /// Coach that authored the note.
    pub coach_id: &'a str,
    /// Conversation the note originated in, if any.
    pub conversation_id: Option<&'a str>,
    /// Scope bucket.
    pub scope: pierre_memory::MemoryScope,
    /// Free-form content.
    pub content: &'a str,
    /// Optional embedding for recall.
    pub embedding: Option<&'a [f32]>,
}

/// Parameters for scheduling a coach followup.
pub struct InsertCoachFollowupParams<'a> {
    /// Tenant that owns the followup.
    pub tenant_id: TenantId,
    /// User the followup targets.
    pub user_id: &'a str,
    /// Coach that made the promise.
    pub coach_id: &'a str,
    /// Conversation the promise was made in, if any.
    pub conversation_id: Option<&'a str>,
    /// Reminder content.
    pub content: &'a str,
    /// When the followup should surface, if a specific time was promised.
    pub due_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Coaching harness memory repository (Tier 0 foundations).
///
/// Groups the small CRUD surface for every memory primitive behind a single
/// trait. The schemas land dormant in Tier 0; services come online in
/// Tier 1 (compaction), Tier 2 (facts + recall), Tier 3 (notes + followups),
/// Tier 4 (sessions). The trait is tight on purpose — new use cases should
/// extend it with named parameter structs rather than accreting positional
/// arguments.
#[async_trait]
pub trait HarnessMemoryRepository: Send + Sync {
    // --- compaction blocks ---

    /// Persist a compaction block covering a range of conversation messages.
    async fn insert_compaction_block(
        &self,
        params: &InsertCompactionBlockParams<'_>,
    ) -> AppResult<pierre_memory::CompactionBlock>;

    /// List compaction blocks for a conversation in creation order.
    async fn list_compaction_blocks(
        &self,
        conversation_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<Vec<pierre_memory::CompactionBlock>>;

    // --- user facts ---

    /// Insert or update a user fact. Callers are responsible for merging
    /// contradictory facts before calling this method.
    async fn upsert_user_fact(
        &self,
        params: &UpsertUserFactParams<'_>,
    ) -> AppResult<pierre_memory::UserFact>;

    /// List user facts for recall. Filters by user, optional coach, and
    /// optional kind; callers apply vector similarity on the returned set.
    async fn list_user_facts(
        &self,
        tenant_id: TenantId,
        user_id: &str,
        coach_id: Option<&str>,
        kind: Option<pierre_memory::FactKind>,
        limit: i64,
    ) -> AppResult<Vec<pierre_memory::UserFact>>;

    /// Delete a fact by id for GDPR-style "forget" flows.
    async fn delete_user_fact(
        &self,
        fact_id: &str,
        tenant_id: TenantId,
        user_id: &str,
    ) -> AppResult<bool>;

    /// Tenant-wide aggregate snapshot of the memory extraction worker's output.
    ///
    /// The worker is a fire-and-forget background task, so its health is
    /// derived from the rows it has actually produced in `user_facts`
    /// rather than instrumenting the worker itself.
    async fn count_user_facts_metrics(
        &self,
        tenant_id: TenantId,
    ) -> AppResult<pierre_memory::UserFactMetrics>;

    // --- coach notes ---

    /// Persist a coach-authored note.
    async fn insert_coach_note(
        &self,
        params: &InsertCoachNoteParams<'_>,
    ) -> AppResult<pierre_memory::CoachNote>;

    /// List coach notes for a user, newest first.
    async fn list_coach_notes(
        &self,
        tenant_id: TenantId,
        user_id: &str,
        coach_id: &str,
        limit: i64,
    ) -> AppResult<Vec<pierre_memory::CoachNote>>;

    /// Tenant-wide coach-note audit log for the admin compliance tab.
    ///
    /// Returns notes newest-first, clamped to `limit`, spanning all
    /// users and coaches. The admin UI uses this for a flat audit trail
    /// so compliance reviewers can see every note a coach wrote about a
    /// user without having to pick a (user, coach) pair first.
    async fn list_coach_notes_for_tenant(
        &self,
        tenant_id: TenantId,
        limit: i64,
    ) -> AppResult<Vec<pierre_memory::CoachNote>>;

    // --- coach followups ---

    /// Schedule a coach followup.
    async fn insert_coach_followup(
        &self,
        params: &InsertCoachFollowupParams<'_>,
    ) -> AppResult<pierre_memory::CoachFollowup>;

    /// List pending followups for a user/coach pair. Caller decides how to
    /// render them in the next prompt.
    async fn list_pending_followups(
        &self,
        tenant_id: TenantId,
        user_id: &str,
        coach_id: &str,
    ) -> AppResult<Vec<pierre_memory::CoachFollowup>>;

    /// Tenant-wide pending followup queue for the admin triage tab.
    ///
    /// Returns rows ordered by due date ascending (nulls last), then
    /// creation order, clamped to `limit`. Unlike
    /// [`Self::list_pending_followups`], this call spans all users and
    /// coaches so the admin can see stale promises across the platform.
    async fn list_pending_followups_for_tenant(
        &self,
        tenant_id: TenantId,
        limit: i64,
    ) -> AppResult<Vec<pierre_memory::CoachFollowup>>;

    /// Mark a followup as delivered after the coach has acted on it.
    async fn mark_followup_delivered(
        &self,
        followup_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<bool>;

    /// Cancel a pending followup — admin clears a stale promise that
    /// should never reach the coach. Returns `false` if the followup was
    /// already delivered or cancelled, `true` on a successful transition.
    async fn cancel_followup(&self, followup_id: &str, tenant_id: TenantId) -> AppResult<bool>;

    // --- coach sessions ---

    /// Open a new coaching session. Returns the existing active session if
    /// one already exists for the `(user, coach)` pair so callers can call
    /// this idempotently.
    async fn get_or_open_coach_session(
        &self,
        tenant_id: TenantId,
        user_id: &str,
        coach_id: &str,
    ) -> AppResult<pierre_memory::CoachSession>;

    /// Mark the most recent activity on a session to feed "continue where
    /// you left off" UI.
    async fn touch_coach_session(&self, session_id: &str, tenant_id: TenantId) -> AppResult<()>;

    /// Archive a session (coach unassigned or retired).
    async fn archive_coach_session(&self, session_id: &str, tenant_id: TenantId)
        -> AppResult<bool>;
}

// ============================================================================
// Claim verdict repository (Tier 5.5 bullshit detector)
// ============================================================================

/// Parameters for inserting a claim verdict via [`ClaimVerdictRepository`].
pub struct InsertClaimVerdictParams<'a> {
    /// Tenant that owns the verdict.
    pub tenant_id: TenantId,
    /// User the claim was said to.
    pub user_id: &'a str,
    /// Coach persona that authored the claim, if resolvable.
    pub coach_id: Option<&'a str>,
    /// Conversation the claim came from, if in-dispatch.
    pub conversation_id: Option<&'a str>,
    /// Message the claim was extracted from, if available.
    pub message_id: Option<&'a str>,
    /// Raw claim text.
    pub claim_text: &'a str,
    /// Category assigned by the extractor.
    pub category: ClaimCategory,
    /// Final pipeline status.
    pub status: ClaimStatus,
    /// Evidence strength backing the verdict.
    pub evidence_strength: EvidenceStrength,
    /// Pipeline confidence in `[0.0, 1.0]`.
    pub confidence: f32,
    /// Which layer produced the verdict.
    pub layer_fired: VerdictLayer,
    /// Optional user-facing rationale.
    pub explanation: Option<&'a str>,
    /// Optional evidence references (DOIs/PMIDs, comma-separated).
    pub evidence_refs: Option<&'a str>,
}

/// Per-status counters rolled up over a verdict scan window.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct VerdictStatusBreakdown {
    /// Claims the pipeline marked `supported`.
    pub supported: i64,
    /// Claims the pipeline marked `unsupported`.
    pub unsupported: i64,
    /// Claims the pipeline marked `contradicted`.
    pub contradicted: i64,
    /// Claims the rhetoric filter dropped as rhetorical.
    pub rhetorical: i64,
    /// Claims the pipeline could not confidently verify either way.
    pub unverifiable: i64,
}

/// One day's verdict counts, keyed by the UTC calendar date `YYYY-MM-DD`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct VerdictDailyBucket {
    /// UTC calendar date `YYYY-MM-DD`.
    pub date: String,
    /// Status breakdown for that day.
    pub counts: VerdictStatusBreakdown,
}

/// Calibration summary emitted by
/// [`ClaimVerdictRepository::aggregate_verdict_stats`].
///
/// Drives the admin "Verdict calibration" panel in the eval harness
/// tab — shows the live Tier 5.5 verdict mix and how it drifts day to
/// day over the requested window.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct VerdictCalibrationStats {
    /// Window lower bound (inclusive), RFC3339 UTC.
    pub window_start: String,
    /// Number of days scanned.
    pub window_days: i64,
    /// Totals over the full window.
    pub totals: VerdictStatusBreakdown,
    /// Per-day breakdown, oldest first.
    pub daily: Vec<VerdictDailyBucket>,
}

/// Tier 5.5 claim verdict repository — persists post-LLM detector output.
#[async_trait]
pub trait ClaimVerdictRepository: Send + Sync {
    /// Persist a new claim verdict.
    async fn insert_claim_verdict(
        &self,
        params: &InsertClaimVerdictParams<'_>,
    ) -> AppResult<ClaimVerdict>;

    /// List verdicts for a conversation in chronological order (oldest first).
    async fn list_verdicts_for_conversation(
        &self,
        conversation_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<Vec<ClaimVerdict>>;

    /// List the most recent verdicts for a tenant, newest first. Used by the
    /// admin "flagged claims" dashboard.
    async fn list_recent_verdicts(
        &self,
        tenant_id: TenantId,
        limit: i64,
    ) -> AppResult<Vec<ClaimVerdict>>;

    /// Aggregate verdict counts over the last `window_days` for a tenant.
    ///
    /// Returns both the full-window totals and a per-day breakdown so
    /// the admin calibration panel can show drift. `window_days` is
    /// clamped to `1..=365` by callers.
    async fn aggregate_verdict_stats(
        &self,
        tenant_id: TenantId,
        window_days: i64,
    ) -> AppResult<VerdictCalibrationStats>;
}

/// User MCP token management repository
#[async_trait]
pub trait UserMcpTokenRepository: Send + Sync {
    /// Create a new user MCP token for AI client authentication
    async fn create_token(
        &self,
        user_id: Uuid,
        request: &CreateUserMcpTokenRequest,
    ) -> AppResult<UserMcpTokenCreated>;
    /// Validate a user MCP token and return the associated user ID
    async fn validate_token(&self, token_value: &str) -> AppResult<Uuid>;
    /// List all MCP tokens for a user
    async fn list_tokens(&self, user_id: Uuid) -> AppResult<Vec<UserMcpTokenInfo>>;
    /// Revoke a user MCP token
    async fn revoke_token(&self, token_id: &str, user_id: Uuid) -> AppResult<()>;
    /// Get a user MCP token by ID
    async fn get_token(&self, token_id: &str, user_id: Uuid) -> AppResult<Option<UserMcpToken>>;
    /// Cleanup expired user MCP tokens (mark as revoked)
    async fn cleanup_expired_tokens(&self) -> AppResult<u64>;
}

/// Impersonation session management repository
#[async_trait]
pub trait ImpersonationRepository: Send + Sync {
    /// Create a new impersonation session for audit trail
    async fn create_session(&self, session: &ImpersonationSession) -> AppResult<()>;
    /// Get impersonation session by ID
    async fn get_session(&self, session_id: &str) -> AppResult<Option<ImpersonationSession>>;
    /// Get active impersonation session where user is impersonator or target
    async fn get_active_session(&self, user_id: Uuid) -> AppResult<Option<ImpersonationSession>>;
    /// End an impersonation session
    async fn end_session(&self, session_id: &str) -> AppResult<()>;
    /// End all active impersonation sessions for an impersonator
    async fn end_all_sessions(&self, impersonator_id: Uuid) -> AppResult<u64>;
    /// List impersonation sessions with optional filters
    async fn list_sessions(
        &self,
        impersonator_id: Option<Uuid>,
        target_user_id: Option<Uuid>,
        active_only: bool,
        limit: u32,
    ) -> AppResult<Vec<ImpersonationSession>>;
}

/// LLM credential management repository
#[async_trait]
pub trait LlmCredentialRepository: Send + Sync {
    /// Store LLM credentials (user-specific or tenant-level)
    async fn store_credentials(&self, record: &LlmCredentialRecord) -> AppResult<()>;
    /// Get LLM credentials for a specific provider
    async fn get_credentials(
        &self,
        tenant_id: TenantId,
        user_id: Option<Uuid>,
        provider: &str,
    ) -> AppResult<Option<LlmCredentialRecord>>;
    /// List all LLM credentials for a tenant (for admin UI)
    async fn list_credentials(&self, tenant_id: TenantId) -> AppResult<Vec<LlmCredentialSummary>>;
    /// Delete LLM credentials
    async fn delete_credentials(
        &self,
        tenant_id: TenantId,
        user_id: Option<Uuid>,
        provider: &str,
    ) -> AppResult<bool>;
    /// Get admin config override value by key (for system-wide LLM API keys)
    async fn get_admin_config_override(
        &self,
        config_key: &str,
        tenant_id: Option<TenantId>,
    ) -> AppResult<Option<String>>;

    /// List every `admin_config_overrides` row for a given category.
    /// Used by the `PricingRegistry` startup loader to populate
    /// `cat_llm_pricing` overrides on top of the compile-time table.
    async fn list_admin_config_overrides_by_category(
        &self,
        category: &str,
    ) -> AppResult<Vec<AdminConfigOverrideRow>>;
}

/// Provider-agnostic subscription persistence.
///
/// Webhook handlers upsert by `(provider, provider_customer_id)`;
/// read paths query by user, tenant, or `(provider, *)` identifier.
#[async_trait]
pub trait SubscriptionsRepository: Send + Sync {
    /// Insert a new subscription row or update the existing row that
    /// shares its `(provider, provider_customer_id)` key. Returns the
    /// freshly written row.
    async fn upsert_subscription(&self, subscription: &Subscription) -> AppResult<Subscription>;
    /// Look up the most recently updated subscription for a user.
    async fn get_subscription_by_user(&self, user_id: Uuid) -> AppResult<Option<Subscription>>;
    /// Look up the most recently updated subscription for a tenant.
    async fn get_subscription_by_tenant(
        &self,
        tenant_id: TenantId,
    ) -> AppResult<Option<Subscription>>;
    /// Look up a subscription by its provider-side subscription identifier.
    async fn get_subscription_by_provider_subscription_id(
        &self,
        provider: &str,
        provider_subscription_id: &str,
    ) -> AppResult<Option<Subscription>>;
    /// Look up a subscription by its provider-side customer identifier.
    async fn get_subscription_by_provider_customer_id(
        &self,
        provider: &str,
        provider_customer_id: &str,
    ) -> AppResult<Option<Subscription>>;
    /// List every subscription with the given lifecycle status.
    /// Used by admin filter views and the dunning sweep.
    async fn list_subscriptions_by_status(
        &self,
        status: SubscriptionStatus,
    ) -> AppResult<Vec<Subscription>>;

    /// Returns true when the given `(provider, event_id)` has already
    /// been processed. The webhook handler skips dispatch on `true`,
    /// making the entire pipeline safe against provider retries.
    async fn is_billing_event_processed(&self, provider: &str, event_id: &str) -> AppResult<bool>;
    /// Mark a `(provider, event_id)` as processed. Call this AFTER the
    /// event dispatch path succeeds — never before — so a partial failure
    /// allows the webhook to retry with the full body.
    async fn mark_billing_event_processed(
        &self,
        provider: &str,
        event_id: &str,
        event_type: &str,
    ) -> AppResult<()>;
}

/// Provider connection management repository
#[async_trait]
pub trait ProviderConnectionRepository: Send + Sync {
    /// Register a provider connection (upsert)
    async fn register_connection(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        connection_type: &ConnectionType,
        metadata: Option<&str>,
    ) -> AppResult<()>;
    /// Remove a provider connection
    async fn remove_connection(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) -> AppResult<()>;
    /// Get all provider connections for a user
    async fn get_for_user(
        &self,
        user_id: Uuid,
        tenant_id: Option<TenantId>,
    ) -> AppResult<Vec<ProviderConnection>>;
    /// Check if a specific provider is connected for a user (cross-tenant)
    async fn is_connected(&self, user_id: Uuid, provider: &str) -> AppResult<bool>;
}

/// Password reset token management repository
#[async_trait]
pub trait PasswordResetRepository: Send + Sync {
    /// Store a password reset token (hashed) for a user
    async fn store_token(
        &self,
        user_id: Uuid,
        token_hash: &str,
        created_by: &str,
    ) -> AppResult<Uuid>;
    /// Store a password reset token with a custom TTL (in minutes)
    ///
    /// Used for self-service password reset codes that expire faster (15 min)
    /// than admin-issued tokens (1 hour).
    async fn store_token_with_ttl(
        &self,
        user_id: Uuid,
        token_hash: &str,
        created_by: &str,
        ttl_minutes: i64,
    ) -> AppResult<Uuid>;
    /// Consume a password reset token by its hash
    async fn consume_token(&self, token_hash: &str) -> AppResult<Uuid>;
    /// Invalidate all unused reset tokens for a user
    async fn invalidate_tokens(&self, user_id: Uuid) -> AppResult<()>;
    /// Count recent reset tokens for a user (for rate limiting)
    ///
    /// Returns the number of tokens created for the user since the given timestamp,
    /// regardless of whether they have been used or expired.
    async fn count_recent_tokens(&self, user_id: Uuid, since: DateTime<Utc>) -> AppResult<i64>;
}

/// Persistent backing store row for the dravr-meteo weather cache.
///
/// Geographic + temporal bucket: lat/lng in centi-degrees (~1.1 km),
/// timestamp floored to the hour. The `provider` column scopes the
/// cache so `OpenMeteo` and `OpenWeatherMap` entries don't collide if a
/// vendor swap is in flight.
#[derive(Debug, Clone, PartialEq)]
pub struct WeatherCacheEntry {
    /// `round(latitude * 100)` — ~1.1 km bucket at the equator.
    pub lat_centi: i32,
    /// `round(longitude * 100)`.
    pub lng_centi: i32,
    /// `floor(unix_timestamp_secs / 3600)`.
    pub hour_unix: i64,
    /// Vendor name, e.g. `"openmeteo"` or `"openweathermap"`.
    pub provider: String,
    /// Ambient temperature in degrees Celsius.
    pub temperature_celsius: f32,
    /// Relative humidity, 0–100.
    pub humidity_percentage: Option<f32>,
    /// Wind speed in km/h.
    pub wind_speed_kmh: Option<f32>,
    /// Free-text condition summary (e.g. `"snow"`, `"clear sky"`).
    pub conditions: String,
}

/// Persistent storage for the dravr-meteo weather cache.
///
/// Not tenant-scoped — weather data is geographic and shared across
/// tenants by design. The `pierre-server::intelligence::weather_cache_adapter`
/// module bridges this trait to dravr-meteo's `WeatherCacheStore` trait.
#[async_trait]
pub trait WeatherCacheRepository: Send + Sync {
    /// Look up a sample by geographic + temporal bucket and provider.
    /// Returns `None` on miss.
    async fn get(
        &self,
        lat_centi: i32,
        lng_centi: i32,
        hour_unix: i64,
        provider: &str,
    ) -> AppResult<Option<WeatherCacheEntry>>;

    /// Persist (or replace) an entry. Upsert semantics — newer writes win.
    async fn put(&self, entry: WeatherCacheEntry) -> AppResult<()>;
}

/// OAuth client-side state management repository
#[async_trait]
pub trait OAuthClientStateRepository: Send + Sync {
    /// Store OAuth client-side state for CSRF protection and PKCE verifier storage
    async fn store_oauth_client_state(&self, state: &OAuthClientState) -> AppResult<()>;
    /// Consume OAuth client state atomically (verify and mark as used)
    async fn consume_oauth_client_state(
        &self,
        state_value: &str,
        provider: &str,
        now: DateTime<Utc>,
    ) -> AppResult<Option<OAuthClientState>>;
}

/// Tool selection and per-tenant configuration repository
#[async_trait]
pub trait ToolSelectionRepository: Send + Sync {
    /// Get the complete tool catalog
    async fn get_tool_catalog(&self) -> AppResult<Vec<ToolCatalogEntry>>;
    /// Get a specific tool catalog entry by name
    async fn get_tool_catalog_entry(&self, tool_name: &str) -> AppResult<Option<ToolCatalogEntry>>;
    /// Get tools filtered by category
    async fn get_tools_by_category(
        &self,
        category: ToolCategory,
    ) -> AppResult<Vec<ToolCatalogEntry>>;
    /// Get tools available for a specific plan level
    async fn get_tools_by_min_plan(&self, plan: TenantPlan) -> AppResult<Vec<ToolCatalogEntry>>;
    /// Get all tool overrides for a tenant
    async fn get_overrides(&self, tenant_id: TenantId) -> AppResult<Vec<TenantToolOverride>>;
    /// Get a specific tool override for a tenant
    async fn get_override(
        &self,
        tenant_id: TenantId,
        tool_name: &str,
    ) -> AppResult<Option<TenantToolOverride>>;
    /// Create or update a tool override for a tenant
    async fn upsert_override(
        &self,
        tenant_id: TenantId,
        tool_name: &str,
        is_enabled: bool,
        enabled_by_user_id: Option<Uuid>,
        reason: Option<String>,
    ) -> AppResult<TenantToolOverride>;
    /// Delete a tool override (revert to catalog default)
    async fn delete_override(&self, tenant_id: TenantId, tool_name: &str) -> AppResult<bool>;
    /// Count enabled tools for a tenant
    async fn count_enabled_tools(&self, tenant_id: TenantId) -> AppResult<usize>;

    /// Insert or update a tool catalog entry (used by startup catalog sync)
    async fn upsert_tool_catalog_entry(&self, entry: &ToolCatalogEntry) -> AppResult<()>;
    /// Delete a tool catalog entry by tool name (removes phantom entries)
    async fn delete_tool_catalog_entry(&self, tool_name: &str) -> AppResult<bool>;
}

/// LLM usage tracking repository for cost analysis and quota enforcement
#[async_trait]
pub trait LlmUsageRepository: Send + Sync {
    /// Insert a new LLM usage record
    async fn insert_llm_usage(&self, params: &InsertLlmUsage<'_>) -> AppResult<LlmUsageRecord>;

    /// Insert a new embedding-usage record. Embedding calls live in
    /// their own table so embedding volume never inflates chat-token
    /// billing aggregates.
    async fn insert_embedding_usage(
        &self,
        params: &InsertEmbeddingUsage<'_>,
    ) -> AppResult<EmbeddingUsageRecord>;

    /// Query aggregated LLM usage grouped by `provider`, `model`, and `call_type`
    async fn get_llm_usage_aggregates(
        &self,
        tenant_id: &str,
        since: &str,
    ) -> AppResult<Vec<LlmUsageAggregateRow>>;

    /// Query daily LLM usage time series for consumption charts
    async fn get_llm_usage_daily_series(
        &self,
        tenant_id: &str,
        since: &str,
    ) -> AppResult<Vec<LlmUsageDailyRow>>;

    /// Per-user variant of [`Self::get_llm_usage_aggregates`].
    /// Drives the admin `GET /api/admin/users/{id}/usage` endpoint
    /// so per-user billing surfaces (Usage tab, invoice preview)
    /// don't have to sum tenant aggregates client-side.
    async fn get_llm_usage_aggregates_by_user(
        &self,
        user_id: &str,
        since: &str,
    ) -> AppResult<Vec<LlmUsageAggregateRow>>;

    /// Per-user variant of [`Self::get_llm_usage_daily_series`].
    /// Drives the admin `GET /api/admin/users/{id}/cost-timeseries`
    /// endpoint that the Usage tab uses to render daily cost gauges.
    async fn get_llm_usage_daily_series_by_user(
        &self,
        user_id: &str,
        since: &str,
    ) -> AppResult<Vec<LlmUsageDailyRow>>;

    /// Get the most recent LLM usage records across all tenants (admin view)
    ///
    /// Returns the last `limit` LLM calls ordered by creation time descending.
    /// Used by the admin real-time activity dashboard.
    async fn get_recent_llm_calls_admin(&self, limit: i64) -> AppResult<Vec<LlmUsageRecord>>;

    /// Count LLM calls created since a given timestamp (admin view, cross-tenant)
    async fn count_llm_calls_since(&self, since: &str) -> AppResult<i64>;

    /// Sum LLM usage since a given timestamp (admin view, cross-tenant).
    ///
    /// Returns a tuple of (`total_calls`, `total_tokens`) for pricing calculations.
    async fn sum_llm_usage_since(&self, since: &str) -> AppResult<(i64, i64)>;

    /// Fetch every LLM usage record attributed to the given conversation
    /// turn, ordered by `created_at` ascending.
    ///
    /// Returns an empty vector when no rows match.
    async fn find_llm_usage_by_turn_id(
        &self,
        turn_id: ConversationTurnId,
    ) -> AppResult<Vec<LlmUsageRecord>>;

    /// Sum `cost_usd` for a tenant over an inclusive period. Used by the
    /// monthly overage cron to drive Stripe Meter event reporting.
    async fn sum_cost_usd_for_tenant_period(
        &self,
        tenant_id: TenantId,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> AppResult<f64>;
}

/// Usage counter repository for rate limiting and quota enforcement
#[async_trait]
pub trait UsageCounterRepository: Send + Sync {
    /// Atomically increment a usage counter via upsert
    async fn increment_counter(
        &self,
        tenant_id: &str,
        user_id: &str,
        counter_key: &str,
        period: &str,
        amount: i64,
    ) -> AppResult<UsageCounterRecord>;

    /// Get the current value of a counter (returns 0 if not found)
    async fn get_counter(
        &self,
        tenant_id: &str,
        user_id: &str,
        counter_key: &str,
        period: &str,
    ) -> AppResult<UsageCounterRecord>;

    /// Delete counters older than the given period cutoff
    ///
    /// System-level housekeeping: intentionally operates across ALL tenants.
    /// Called only from background pruning tasks, not user-facing endpoints.
    async fn delete_old_counters(&self, period_before: &str) -> AppResult<u64>;
}

// ================================
// Domain-specific repository traits with dedicated manager implementations
// ================================

/// Recipe storage and management repository (tenant-scoped)
#[async_trait]
pub trait RecipeRepository: Send + Sync {
    /// Create a new recipe
    async fn create(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        recipe: &Recipe,
    ) -> AppResult<String>;
    /// Get recipe by ID
    async fn get_by_id(
        &self,
        recipe_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Option<Recipe>>;
    /// List recipes with optional filtering
    async fn list(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        meal_timing: Option<MealTiming>,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> AppResult<Vec<Recipe>>;
    /// Update an existing recipe
    async fn update(
        &self,
        recipe_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
        recipe: &Recipe,
    ) -> AppResult<bool>;
    /// Delete a recipe
    async fn delete(&self, recipe_id: &str, user_id: Uuid, tenant_id: TenantId) -> AppResult<bool>;
    /// Update cached nutrition data for a recipe
    async fn update_nutrition_cache(
        &self,
        recipe_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
        nutrition: &ValidatedNutrition,
    ) -> AppResult<bool>;
    /// Search recipes by text query
    async fn search(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        query: &str,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> AppResult<Vec<Recipe>>;
    /// Count recipes
    async fn count(&self, user_id: Uuid, tenant_id: TenantId) -> AppResult<u32>;
}

/// Coaches (custom AI personas) storage and management repository (tenant-scoped)
#[async_trait]
pub trait CoachesRepository: Send + Sync {
    /// Create a new coach
    async fn create(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        request: &CreateCoachRequest,
    ) -> AppResult<Coach>;
    /// Get coach by ID
    async fn get_by_id(
        &self,
        coach_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Option<Coach>>;
    /// List coaches with optional filtering
    async fn list(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        filter: &ListCoachesFilter,
    ) -> AppResult<Vec<CoachListItem>>;
    /// Apply per-locale translation overlays to a list of coaches.
    ///
    /// Reads `coach_translations` for `locale` and overlays
    /// `title`/`description`/`purpose`/`instructions` on each matching coach
    /// in-place. Coaches without a matching translation row keep their
    /// canonical English copy. Called after `list` when a channel locale has
    /// been resolved.
    ///
    /// Fast-path: `locale == "en"` returns immediately without touching the
    /// database. English lives on the canonical `coaches` row itself; a
    /// `coach_translations` row with `locale = "en"` is only interesting when
    /// an operator wants to override the canonical, which is out of scope for
    /// Phase 1.
    async fn apply_translations(
        &self,
        coaches: &mut [CoachListItem],
        locale: &str,
    ) -> AppResult<()>;
    /// Update an existing coach
    async fn update(
        &self,
        coach_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
        request: &UpdateCoachRequest,
    ) -> AppResult<Option<Coach>>;
    /// Delete a coach
    async fn delete(&self, coach_id: &str, user_id: Uuid, tenant_id: TenantId) -> AppResult<bool>;
    /// Record a usage event for a coach interaction
    async fn record_usage(
        &self,
        coach_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<bool>;
    /// Toggle favorite status for a coach
    async fn toggle_favorite(
        &self,
        coach_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Option<bool>>;
    /// Search coachs by text query
    async fn search(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        query: &str,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> AppResult<Vec<Coach>>;
    /// Count coachs
    async fn count(&self, user_id: Uuid, tenant_id: TenantId) -> AppResult<u32>;

    // --- User methods ---

    /// Fork a coach into a user-owned copy
    async fn fork_coach(
        &self,
        source_coach_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Coach>;
    /// Activate a coach for the user
    async fn activate_coach(
        &self,
        coach_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Option<Coach>>;
    /// Deactivate the user's currently active coach
    async fn deactivate_coach(&self, user_id: Uuid, tenant_id: TenantId) -> AppResult<bool>;
    /// Get the user's currently active coach
    async fn get_active_coach(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Option<Coach>>;

    /// Find a coach by content hash for import deduplication
    async fn find_by_content_hash(
        &self,
        content_hash: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Option<Coach>>;

    // --- Admin methods ---

    /// Create a system-level coach
    async fn create_system_coach(
        &self,
        admin_user_id: Uuid,
        tenant_id: TenantId,
        request: &CreateSystemCoachRequest,
    ) -> AppResult<Coach>;
    /// List all system coaches for a tenant
    async fn list_system_coaches(&self, tenant_id: TenantId) -> AppResult<Vec<Coach>>;
    /// Get a system coach by ID within a tenant
    async fn get_system_coach(
        &self,
        coach_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<Option<Coach>>;
    /// Get a system coach by ID regardless of tenant
    async fn get_system_coach_any_tenant(&self, coach_id: &str) -> AppResult<Option<Coach>>;
    /// Update a system coach
    async fn update_system_coach(
        &self,
        coach_id: &str,
        tenant_id: TenantId,
        request: &UpdateCoachRequest,
    ) -> AppResult<Option<Coach>>;
    /// Delete a system coach
    async fn delete_system_coach(&self, coach_id: &str, tenant_id: TenantId) -> AppResult<bool>;

    // --- Assignment methods ---

    /// Get user preferences for a coach (`is_favorite`, `is_hidden`, `usage_count`, `last_used_at`)
    async fn get_user_preferences(
        &self,
        coach_id: &str,
        user_id: Uuid,
    ) -> AppResult<(bool, bool, u32, Option<DateTime<Utc>>)>;
    /// Assign a coach to a user
    async fn assign_coach(
        &self,
        coach_id: &str,
        user_id: Uuid,
        assigned_by: Uuid,
    ) -> AppResult<bool>;
    /// Unassign a coach from a user
    async fn unassign_coach(&self, coach_id: &str, user_id: Uuid) -> AppResult<bool>;
    /// List all assignments for a coach
    async fn list_assignments(&self, coach_id: &str) -> AppResult<Vec<CoachAssignment>>;
    /// List assignments for a coach within a tenant
    async fn list_assignments_for_tenant(
        &self,
        coach_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<Vec<CoachAssignment>>;
    /// Hide a coach from the user's view
    async fn hide_coach(&self, coach_id: &str, user_id: Uuid) -> AppResult<bool>;
    /// Show a previously hidden coach
    async fn show_coach(&self, coach_id: &str, user_id: Uuid) -> AppResult<bool>;
    /// List coaches hidden by a user
    async fn list_hidden_coaches(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Vec<Coach>>;

    // --- Version methods ---

    /// Create a new version snapshot for a coach
    async fn create_version(
        &self,
        coach_id: &str,
        user_id: Uuid,
        change_summary: Option<&str>,
    ) -> AppResult<i32>;
    /// Get version history for a coach
    async fn get_versions(
        &self,
        coach_id: &str,
        tenant_id: TenantId,
        limit: u32,
    ) -> AppResult<Vec<CoachVersion>>;
    /// Get a specific version of a coach
    async fn get_version(
        &self,
        coach_id: &str,
        version: i32,
        tenant_id: TenantId,
    ) -> AppResult<Option<CoachVersion>>;
    /// Revert a coach to a previous version
    async fn revert_to_version(
        &self,
        coach_id: &str,
        version: i32,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Coach>;
    /// Get the current version number for a coach
    async fn get_current_version(&self, coach_id: &str) -> AppResult<i32>;

    /// Resolve the full runtime context (system prompt, startup query, data
    /// requirements, tool-iteration override) for a coach attached to a
    /// conversation.
    ///
    /// Tenant-scoped: returns the coach if it belongs to the caller's tenant
    /// or is a system coach. Returns `None` if no matching coach is found.
    async fn get_coach_runtime_context(
        &self,
        coach_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<Option<CoachRuntimeContext>>;
}

/// Mobility (stretching exercises and yoga poses) read-only repository
#[async_trait]
pub trait MobilityRepository: Send + Sync {
    /// Get a stretching exercise by ID
    async fn get_stretching_exercise(&self, id: &str) -> AppResult<Option<StretchingExercise>>;
    /// List stretching exercises with optional filtering
    async fn list_stretching_exercises(
        &self,
        filter: &ListStretchingFilter,
    ) -> AppResult<Vec<StretchingExercise>>;
    /// Search stretching exercises by text query
    async fn search_stretching_exercises(
        &self,
        query: &str,
        limit: Option<u32>,
    ) -> AppResult<Vec<StretchingExercise>>;
    /// Get stretches recommended for a specific activity type
    async fn get_stretches_for_activity(
        &self,
        activity_type: &str,
        limit: Option<u32>,
    ) -> AppResult<Vec<StretchingExercise>>;
    /// Get a yoga pose by ID
    async fn get_yoga_pose(&self, id: &str) -> AppResult<Option<YogaPose>>;
    /// List yoga poses with optional filtering
    async fn list_yoga_poses(&self, filter: &ListYogaFilter) -> AppResult<Vec<YogaPose>>;
    /// Search yoga poses by text query
    async fn search_yoga_poses(&self, query: &str, limit: Option<u32>) -> AppResult<Vec<YogaPose>>;
    /// Get yoga poses recommended for a recovery context
    async fn get_poses_for_recovery(
        &self,
        recovery_context: &str,
        limit: Option<u32>,
    ) -> AppResult<Vec<YogaPose>>;
    /// Get muscle mapping for a specific activity type
    async fn get_activity_muscle_mapping(
        &self,
        activity_type: &str,
    ) -> AppResult<Option<ActivityMuscleMapping>>;
    /// List all activity-to-muscle mappings
    async fn list_activity_muscle_mappings(&self) -> AppResult<Vec<ActivityMuscleMapping>>;
}

/// Social features repository for friend connections and shared insights
#[async_trait]
pub trait SocialRepository: Send + Sync {
    /// Create a new friend connection request
    async fn create_friend_connection(&self, connection: &FriendConnection) -> AppResult<Uuid>;
    /// Get a friend connection by ID
    async fn get_friend_connection(&self, id: Uuid) -> AppResult<Option<FriendConnection>>;
    /// Get the friend connection between two users (if any)
    async fn get_friend_connection_between(
        &self,
        user_a: Uuid,
        user_b: Uuid,
    ) -> AppResult<Option<FriendConnection>>;
    /// Update friend connection status (accept, reject, block)
    async fn update_friend_connection_status(
        &self,
        id: Uuid,
        user_id: Uuid,
        status: FriendStatus,
    ) -> AppResult<()>;
    /// Get all accepted friends for a user
    async fn get_friends(&self, user_id: Uuid) -> AppResult<Vec<FriendConnection>>;
    /// Get pending incoming friend requests
    async fn get_pending_friend_requests(&self, user_id: Uuid) -> AppResult<Vec<FriendConnection>>;
    /// Get outgoing friend requests sent by the user
    async fn get_sent_friend_requests(&self, user_id: Uuid) -> AppResult<Vec<FriendConnection>>;
    /// Check whether two users are friends
    async fn are_friends(&self, user_a: Uuid, user_b: Uuid) -> AppResult<bool>;
    /// Delete a friend connection
    async fn delete_friend_connection(&self, id: Uuid, user_id: Uuid) -> AppResult<bool>;
    /// Get social settings for a user, creating defaults if not found
    async fn get_or_create_social_settings(&self, user_id: Uuid) -> AppResult<UserSocialSettings>;
    /// Get social settings for a user (returns None if not set)
    async fn get_social_settings(&self, user_id: Uuid) -> AppResult<Option<UserSocialSettings>>;
    /// Create or update social settings for a user
    async fn upsert_social_settings(&self, settings: &UserSocialSettings) -> AppResult<()>;
    /// Share an insight with friends
    async fn create_shared_insight(&self, insight: &SharedInsight) -> AppResult<Uuid>;
    /// Get a shared insight by ID, scoped to user
    async fn get_shared_insight(&self, id: Uuid, user_id: Uuid)
        -> AppResult<Option<SharedInsight>>;
    /// Get the friends feed (shared insights from friends)
    async fn get_friends_feed(
        &self,
        user_id: Uuid,
        limit: u32,
        offset: u32,
    ) -> AppResult<Vec<SharedInsight>>;
    /// Get insights shared by a specific user
    async fn get_user_shared_insights(
        &self,
        user_id: Uuid,
        limit: u32,
        offset: u32,
    ) -> AppResult<Vec<SharedInsight>>;
    /// Delete a shared insight
    async fn delete_shared_insight(&self, id: Uuid, user_id: Uuid) -> AppResult<bool>;
    /// Create or update a reaction to an insight
    async fn upsert_insight_reaction(&self, reaction: &InsightReaction) -> AppResult<()>;
    /// Get a specific user's reaction to an insight
    async fn get_insight_reaction(
        &self,
        insight_id: Uuid,
        user_id: Uuid,
    ) -> AppResult<Option<InsightReaction>>;
    /// Delete a reaction to an insight
    async fn delete_insight_reaction(&self, insight_id: Uuid, user_id: Uuid) -> AppResult<bool>;
    /// Get all reactions to an insight
    async fn get_insight_reactions(&self, insight_id: Uuid) -> AppResult<Vec<InsightReaction>>;
    /// Create an adapted version of a shared insight
    async fn create_adapted_insight(&self, insight: &AdaptedInsight) -> AppResult<Uuid>;
    /// Get an adapted insight by ID
    async fn get_adapted_insight(&self, id: Uuid) -> AppResult<Option<AdaptedInsight>>;
    /// Get a user's adaptation of a specific source insight
    async fn get_user_adaptation(
        &self,
        source_insight_id: Uuid,
        user_id: Uuid,
    ) -> AppResult<Option<AdaptedInsight>>;
    /// Get all adapted insights for a user
    async fn get_user_adapted_insights(
        &self,
        user_id: Uuid,
        limit: u32,
        offset: u32,
    ) -> AppResult<Vec<AdaptedInsight>>;
    /// Update whether an adapted insight was helpful
    async fn update_adapted_insight_helpful(
        &self,
        id: Uuid,
        user_id: Uuid,
        was_helpful: bool,
    ) -> AppResult<bool>;
    /// Search for discoverable users by query
    async fn search_discoverable_users(
        &self,
        query: &str,
        exclude_user_id: Uuid,
        limit: u32,
    ) -> AppResult<Vec<(Uuid, String, Option<String>)>>;
    /// Get total friend count for a user
    async fn get_friend_count(&self, user_id: Uuid) -> AppResult<i64>;
}

/// Parameters for upserting a messaging channel configuration
pub struct UpsertChannelConfigParams<'a> {
    /// Unique identifier for this config
    pub id: &'a str,
    /// Owning tenant
    pub tenant_id: TenantId,
    /// Channel type (whatsapp, messenger, discord, slack, telegram)
    pub channel_type: &'a str,
    /// Access token / account SID
    pub api_key: Option<&'a str>,
    /// Auth token / API secret
    pub api_secret: Option<&'a str>,
    /// Signing secret for webhook verification
    pub webhook_secret: Option<&'a str>,
    /// Meta webhook verify token (distinct from `webhook_secret` to avoid leaking HMAC key)
    pub verify_token: Option<&'a str>,
    /// Platform-specific account identifier
    pub account_id: Option<&'a str>,
    /// Phone number (WhatsApp/SMS)
    pub phone_number: Option<&'a str>,
    /// Bot token (Discord/Telegram)
    pub bot_token: Option<&'a str>,
    /// Whether this channel is active
    pub is_active: bool,
}

/// Parameters for creating a messaging session
pub struct CreateSessionParams<'a> {
    /// Unique session identifier
    pub id: &'a str,
    /// Pierre user ID
    pub user_id: &'a str,
    /// Owning tenant
    pub tenant_id: TenantId,
    /// Channel type
    pub channel_type: &'a str,
    /// Channel-native user identifier
    pub channel_user_id: &'a str,
    /// Channel-native conversation/thread identifier
    pub channel_conversation_id: Option<&'a str>,
    /// Pierre conversation identifier
    pub pierre_conversation_id: Option<&'a str>,
}

/// Parameters for inserting a messaging message
pub struct InsertMessageParams<'a> {
    /// Unique message identifier
    pub id: &'a str,
    /// Owning tenant
    pub tenant_id: TenantId,
    /// Session this message belongs to
    pub session_id: &'a str,
    /// Direction: "inbound" or "outbound"
    pub direction: &'a str,
    /// Channel type
    pub channel_type: &'a str,
    /// Channel-native message ID (idempotency key)
    pub channel_message_id: &'a str,
    /// Sender identifier
    pub sender_id: &'a str,
    /// Content type (text, media, location, card)
    pub content_type: &'a str,
    /// Text body or serialized content
    pub content_body: Option<&'a str>,
    /// Correlation identifier for request tracking
    pub correlation_id: &'a str,
    /// Original webhook JSON for audit
    pub raw_payload: Option<&'a str>,
}

/// Parameters for creating a pending link state
pub struct CreateLinkStateParams<'a> {
    /// Unique state identifier
    pub id: &'a str,
    /// Owning tenant
    pub tenant_id: TenantId,
    /// Pierre user requesting the link (None for webhook-initiated flows)
    pub user_id: Option<&'a str>,
    /// Target channel type
    pub channel_type: &'a str,
    /// Cryptographically random verification code
    pub code: &'a str,
    /// Linking method (`deep_link` or `oauth`)
    pub method: &'a str,
    /// Sender's platform ID (set by webhook handler for channel-initiated flows)
    pub channel_user_id: Option<&'a str>,
    /// Display name from platform (for login page greeting)
    pub sender_name: Option<&'a str>,
    /// Expiration timestamp (RFC 3339)
    pub expires_at: &'a str,
}

/// Parameters for creating a permanent channel link
pub struct CreateChannelLinkParams<'a> {
    /// Unique link identifier
    pub id: &'a str,
    /// Owning tenant
    pub tenant_id: TenantId,
    /// Pierre user identifier
    pub user_id: &'a str,
    /// Channel type
    pub channel_type: &'a str,
    /// Channel-specific user identifier
    pub channel_user_id: &'a str,
    /// Human-readable display name from the platform
    pub display_name: Option<&'a str>,
}

/// Multi-channel messaging gateway repository
///
/// Manages channel configurations, sessions, messages with idempotency,
/// delivery receipts, and outbound retry queue entries.
#[async_trait]
pub trait MessagingRepository: Send + Sync {
    // ── Channel Configs ──

    /// Upsert a channel configuration (one per tenant + channel type)
    async fn upsert_channel_config(&self, params: &UpsertChannelConfigParams<'_>) -> AppResult<()>;

    /// Get a channel configuration by tenant and channel type
    async fn get_channel_config(
        &self,
        tenant_id: TenantId,
        channel_type: &str,
    ) -> AppResult<Option<Value>>;

    /// List all active channel configurations for a tenant
    async fn list_channel_configs(&self, tenant_id: TenantId) -> AppResult<Vec<Value>>;

    /// Get all active configs for a channel type across all tenants.
    ///
    /// Cross-tenant query justified for webhook authentication: the inbound webhook
    /// carries no Pierre auth token, so we must try each tenant's signing secret
    /// to identify the caller.
    async fn get_configs_by_channel_type(&self, channel_type: &str) -> AppResult<Vec<Value>>;

    /// Delete a channel configuration
    async fn delete_channel_config(
        &self,
        tenant_id: TenantId,
        channel_type: &str,
    ) -> AppResult<bool>;

    // ── Sessions ──

    /// Create a messaging session linking a channel user to a Pierre conversation
    async fn create_session(&self, params: &CreateSessionParams<'_>) -> AppResult<()>;

    /// Look up a session by channel identity (tenant + channel type + channel user ID)
    async fn get_session_by_channel_identity(
        &self,
        tenant_id: TenantId,
        channel_type: &str,
        channel_user_id: &str,
    ) -> AppResult<Option<Value>>;

    /// Update the last message timestamp on a session
    async fn touch_session(&self, session_id: &str) -> AppResult<()>;

    /// Repoint a session at a fresh Pierre conversation.
    ///
    /// Used by the self-heal path when a session's `pierre_conversation_id`
    /// is NULL (the referenced conversation was deleted and the FK
    /// `ON DELETE SET NULL` fired) or the conversation has otherwise become
    /// unreachable. Creates no conversation itself — the caller provides a
    /// fresh conversation id.
    async fn set_session_conversation(
        &self,
        session_id: &str,
        pierre_conversation_id: &str,
    ) -> AppResult<()>;

    // ── Messages ──

    /// Store an inbound or outbound message (idempotent via `channel_message_id`)
    async fn insert_message(&self, params: &InsertMessageParams<'_>) -> AppResult<bool>;

    /// Get messages for a session, ordered by creation time
    async fn get_session_messages(
        &self,
        session_id: &str,
        tenant_id: TenantId,
        limit: i64,
        offset: i64,
    ) -> AppResult<Vec<Value>>;

    // ── Delivery Receipts ──

    /// Record a delivery status update for an outbound message
    async fn insert_delivery_receipt(
        &self,
        id: &str,
        tenant_id: TenantId,
        message_id: &str,
        channel_message_id: Option<&str>,
        status: &str,
    ) -> AppResult<()>;

    // ── Outbound Queue ──

    /// Enqueue an outbound message for delivery
    async fn enqueue_outbound(
        &self,
        id: &str,
        message_id: &str,
        tenant_id: TenantId,
        user_id: Option<&str>,
        channel_type: &str,
        payload: &str,
    ) -> AppResult<()>;

    /// Get pending or retryable outbound messages
    async fn get_pending_outbound(&self, tenant_id: TenantId, limit: i64) -> AppResult<Vec<Value>>;

    /// Get pending/retryable outbound entries across all tenants for background processing.
    ///
    /// Cross-tenant query justified for the background retry worker: it must process
    /// outbound messages for all tenants without knowing tenant IDs in advance.
    async fn get_all_pending_outbound(&self, limit: i64) -> AppResult<Vec<Value>>;

    /// Update outbound queue entry after a send attempt
    async fn update_outbound_status(
        &self,
        id: &str,
        status: &str,
        attempt_count: i32,
        next_retry_at: Option<&str>,
    ) -> AppResult<()>;

    // ── Channel Linking ──

    /// Store a pending link state (verification code with 10-minute TTL)
    async fn create_link_state(&self, params: &CreateLinkStateParams<'_>) -> AppResult<()>;

    /// Atomically consume a link state by verification code and `tenant_id`.
    ///
    /// Uses `UPDATE ... SET used = 1 WHERE code = ? AND tenant_id = ? AND used = 0 AND expires_at > now`,
    /// then checks `rows_affected` to ensure one-time use.
    async fn consume_link_state(&self, code: &str, tenant_id: TenantId) -> AppResult<Value>;

    /// Read-only lookup of a link state by code for rendering the login page.
    ///
    /// Returns the link state data if the code exists, is not expired, and has not been used.
    /// Does NOT consume the code.
    async fn get_link_state(&self, code: &str) -> AppResult<Option<Value>>;

    /// Atomically complete a webhook-initiated link state by setting its `user_id`.
    ///
    /// Only succeeds if the code exists, is not expired, is not used, and has no `user_id` set.
    /// On success, marks the code as used and returns the link state data.
    async fn complete_link_state(&self, code: &str, user_id: &str) -> AppResult<Value>;

    /// Create a permanent channel link mapping user to channel identity
    async fn create_channel_link(&self, params: &CreateChannelLinkParams<'_>) -> AppResult<()>;

    /// Look up a channel link by channel identity (for inbound webhook user resolution)
    async fn get_channel_link(
        &self,
        tenant_id: TenantId,
        channel_type: &str,
        channel_user_id: &str,
    ) -> AppResult<Option<Value>>;

    /// List all channel links for a user
    async fn list_user_channel_links(
        &self,
        tenant_id: TenantId,
        user_id: &str,
    ) -> AppResult<Vec<Value>>;

    /// Delete a channel link (unlink a channel)
    async fn delete_channel_link(
        &self,
        tenant_id: TenantId,
        user_id: &str,
        channel_type: &str,
    ) -> AppResult<bool>;

    /// Read the optional per-channel-link locale override.
    ///
    /// Returns `Some("en")` when the user has explicitly set a locale for
    /// this specific channel, `None` when they inherit their `users.locale`.
    /// Resolution order in `messaging_ingress` is: this value → `users.locale`
    /// → `DEFAULT_LOCALE`.
    async fn get_channel_link_locale(
        &self,
        tenant_id: TenantId,
        channel_type: &str,
        channel_user_id: &str,
    ) -> AppResult<Option<String>>;

    /// Set or clear the per-channel-link locale override.
    ///
    /// Pass `None` to clear the override and inherit from `users.locale`.
    /// Pass `Some("en")`/`Some("fr")`/etc. to pin the channel to a specific
    /// locale regardless of the user-level setting.
    async fn set_channel_link_locale(
        &self,
        tenant_id: TenantId,
        user_id: &str,
        channel_type: &str,
        locale: Option<&str>,
    ) -> AppResult<()>;

    /// Logout a channel sender: delete their channel link, sessions, and OTP states.
    /// Identified by channel identity (`sender_id`), not `user_id`.
    async fn logout_channel_sender(
        &self,
        tenant_id: TenantId,
        channel_type: &str,
        sender_id: &str,
    ) -> AppResult<()>;

    // ── In-Chat OTP Linking ──

    /// Look up an active in-chat OTP linking flow by channel identity.
    /// Returns the link state if one exists with `otp_step` set, `used = 0`, and not expired.
    async fn get_active_otp_link_state(
        &self,
        tenant_id: TenantId,
        channel_type: &str,
        channel_user_id: &str,
    ) -> AppResult<Option<Value>>;

    /// Advance the OTP flow: set email and OTP hash, transition to `awaiting_otp`.
    async fn set_otp_on_link_state(&self, id: &str, email: &str, otp_hash: &str) -> AppResult<()>;

    /// Increment OTP attempt counter and return the new count (brute-force protection).
    async fn increment_otp_attempts(&self, id: &str) -> AppResult<i32>;

    /// Invalidate any active OTP link states for a sender (cleanup before new flow).
    async fn invalidate_otp_link_states(
        &self,
        tenant_id: TenantId,
        channel_type: &str,
        channel_user_id: &str,
    ) -> AppResult<()>;
}

// ================================
// Seeder Repository (seed-only database operations)
// ================================

/// Tables that seeders are allowed to reset (prevent arbitrary table access)
#[derive(Debug, Clone, Copy)]
pub enum SeedTable {
    /// `users` table
    Users,
    /// `api_keys` table
    ApiKeys,
    /// `a2a_clients` table
    A2AClients,
    /// `llm_usage` table
    LlmUsage,
    /// `api_key_usage` table
    ApiKeyUsage,
    /// `a2a_usage` table
    A2AUsage,
    /// `synthetic_activities` table
    SyntheticActivities,
    /// `friend_connections` table
    FriendConnections,
    /// `user_social_settings` table
    UserSocialSettings,
    /// `shared_insights` table
    SharedInsights,
    /// `insight_reactions` table
    InsightReactions,
    /// `adapted_insights` table
    AdaptedInsights,
    /// `stretching_exercises` table
    StretchingExercises,
    /// `yoga_poses` table
    YogaPoses,
    /// `activity_muscle_mapping` table
    ActivityMuscleMapping,
}

impl SeedTable {
    /// Get the SQL table name
    #[must_use]
    pub const fn table_name(&self) -> &'static str {
        match self {
            Self::Users => "users",
            Self::ApiKeys => "api_keys",
            Self::A2AClients => "a2a_clients",
            Self::LlmUsage => "llm_usage",
            Self::ApiKeyUsage => "api_key_usage",
            Self::A2AUsage => "a2a_usage",
            Self::SyntheticActivities => "synthetic_activities",
            Self::FriendConnections => "friend_connections",
            Self::UserSocialSettings => "user_social_settings",
            Self::SharedInsights => "shared_insights",
            Self::InsightReactions => "insight_reactions",
            Self::AdaptedInsights => "adapted_insights",
            Self::StretchingExercises => "stretching_exercises",
            Self::YogaPoses => "yoga_poses",
            Self::ActivityMuscleMapping => "activity_muscle_mapping",
        }
    }
}

/// Repository trait for seed-only database operations.
///
/// Used by seeder binaries to populate demo/test data.
/// Not used by the main server application. Provides write operations
/// for tables that only have read-only repository traits in the main app.
#[async_trait]
pub trait SeederRepository: Send + Sync {
    // ---- Generic operations ----

    /// Delete all rows from a seed table
    async fn seed_reset_table(&self, table: SeedTable) -> AppResult<u64>;

    /// Count rows in a seed table
    async fn seed_count_table(&self, table: SeedTable) -> AppResult<i64>;

    // ---- User lookup (shared across seeders) ----

    /// Get the first admin user (`super_admin` or admin role)
    async fn seed_get_admin_user(&self) -> AppResult<Option<User>>;

    /// Get the `tenant_id` for a user
    async fn seed_get_user_tenant(&self, user_id: Uuid) -> AppResult<Option<String>>;

    /// Find a user by email address (returns full User if found)
    async fn seed_find_user_by_email(&self, email: &str) -> AppResult<Option<User>>;

    /// Get IDs of all non-admin users ordered by creation date
    async fn seed_get_non_admin_user_ids(&self) -> AppResult<Vec<Uuid>>;

    /// Count non-admin users
    async fn seed_count_non_admin_users(&self) -> AppResult<i64>;

    // ---- Mobility seeder (stretching, yoga, activity mappings) ----

    /// Upsert a stretching exercise (insert or replace on conflict)
    async fn seed_upsert_stretching_exercise(&self, exercise: &StretchingExercise)
        -> AppResult<()>;

    /// Upsert a yoga pose (insert or replace on conflict)
    async fn seed_upsert_yoga_pose(&self, pose: &YogaPose) -> AppResult<()>;

    /// Upsert an activity-muscle mapping (insert or replace on conflict)
    async fn seed_upsert_activity_mapping(&self, mapping: &ActivityMuscleMapping) -> AppResult<()>;

    // ---- LLM usage seeder ----

    /// Delete LLM usage records for a specific tenant
    async fn seed_delete_llm_usage_by_tenant(&self, tenant_id: Uuid) -> AppResult<u64>;

    /// Insert a single LLM usage record
    async fn seed_insert_llm_usage(&self, record: &SeedLlmUsageRecord) -> AppResult<()>;

    // ---- Synthetic activities seeder ----

    /// Delete synthetic activities for a specific user
    async fn seed_delete_synthetic_by_user(&self, user_id: Uuid) -> AppResult<u64>;

    /// Insert a synthetic activity record
    async fn seed_insert_synthetic_activity(
        &self,
        activity: &SeedSyntheticActivity,
    ) -> AppResult<()>;

    /// Upsert a provider connection (insert or update on conflict)
    async fn seed_upsert_provider_connection(&self, conn: &SeedProviderConnection)
        -> AppResult<()>;

    // ---- Social data seeder ----

    /// Reset all social data tables (deletes in foreign key order)
    async fn seed_reset_social_data(&self) -> AppResult<()>;

    /// Insert social settings for a user if not already present, returns true if inserted
    async fn seed_upsert_social_settings(&self, settings: &SeedSocialSettings) -> AppResult<bool>;

    /// Insert a friend connection if one doesn't already exist between the users
    async fn seed_insert_friend_connection_if_absent(
        &self,
        conn: &SeedFriendConnection,
    ) -> AppResult<bool>;

    /// Insert a shared insight record
    async fn seed_insert_shared_insight(&self, insight: &SeedSharedInsight) -> AppResult<()>;

    /// Get all shared insight IDs
    async fn seed_get_shared_insight_ids(&self) -> AppResult<Vec<Uuid>>;

    /// Insert a reaction if one doesn't already exist for this user+insight
    async fn seed_insert_reaction_if_absent(
        &self,
        reaction: &SeedInsightReaction,
    ) -> AppResult<bool>;

    /// Get shared insights with their author IDs: `Vec<(insight_id, author_user_id)>`
    async fn seed_get_shared_insights_with_authors(&self) -> AppResult<Vec<(Uuid, Uuid)>>;

    /// Insert an adapted insight if one doesn't already exist for this user+source
    async fn seed_insert_adapted_insight_if_absent(
        &self,
        adapted: &SeedAdaptedInsight,
    ) -> AppResult<bool>;

    /// Get shared insight IDs not authored by the given user (limited)
    async fn seed_get_shared_insights_not_by_user(
        &self,
        user_id: Uuid,
        limit: i64,
    ) -> AppResult<Vec<Uuid>>;

    // ---- Demo data seeder ----

    /// Check if a user exists by email, returning their ID if found
    async fn seed_check_user_exists(&self, email: &str) -> AppResult<Option<Uuid>>;

    /// Insert a demo user row
    async fn seed_insert_demo_user(&self, user: &SeedDemoUser) -> AppResult<()>;

    /// Insert a tenant row
    async fn seed_insert_tenant(&self, tenant: &SeedTenant) -> AppResult<()>;

    /// Insert a tenant-user junction row
    async fn seed_insert_tenant_user(
        &self,
        id: Uuid,
        tenant_id: Uuid,
        user_id: Uuid,
        now: DateTime<Utc>,
    ) -> AppResult<()>;

    /// Update the `tenant_id` column on a user row
    async fn seed_update_user_tenant(&self, user_id: Uuid, tenant_id: Uuid) -> AppResult<()>;

    /// Check if an API key exists by name, returning its ID if found
    async fn seed_check_api_key_by_name(&self, name: &str) -> AppResult<Option<Uuid>>;

    /// Insert an API key
    async fn seed_insert_api_key(&self, key: &SeedApiKey) -> AppResult<()>;

    /// Check if an A2A client exists by name, returning its ID if found
    async fn seed_check_a2a_client_by_name(&self, name: &str) -> AppResult<Option<Uuid>>;

    /// Insert an A2A client
    async fn seed_insert_a2a_client(&self, client: &SeedA2AClient) -> AppResult<()>;

    /// Insert an API key usage record
    async fn seed_insert_api_key_usage(&self, usage: &SeedApiKeyUsage) -> AppResult<()>;

    /// Insert an A2A usage record
    async fn seed_insert_a2a_usage(&self, usage: &SeedA2AUsage) -> AppResult<()>;

    // ---- Coach seeder ----

    /// Find a coach by slug and tenant, returning `(id, content_hash)` if found
    async fn seed_find_coach_by_slug(
        &self,
        slug: &str,
        tenant_id: &str,
    ) -> AppResult<Option<(String, Option<String>)>>;

    /// Insert a coach record
    async fn seed_insert_coach(&self, coach: &SeedCoach) -> AppResult<()>;

    /// Update an existing coach record
    async fn seed_update_coach(&self, coach: &SeedCoach) -> AppResult<()>;

    /// Insert a coach relation if it doesn't already exist, returns true if inserted
    async fn seed_insert_coach_relation_if_absent(
        &self,
        relation: &SeedCoachRelation,
    ) -> AppResult<bool>;

    /// Upsert a coach author profile, returning the author ID
    ///
    /// Creates the `coach_authors` row if absent (idempotent by `user_id + tenant_id` unique).
    /// Returns the `coach_authors.id` for use as `store_listings.author_id`.
    async fn seed_upsert_coach_author(&self, author: &SeedCoachAuthor) -> AppResult<String>;

    /// Insert a store listing if it doesn't already exist, returns true if inserted
    async fn seed_insert_store_listing_if_absent(
        &self,
        listing: &SeedStoreListing,
    ) -> AppResult<bool>;

    /// Upsert a `coach_translations` row for `(coach_id, locale)`.
    ///
    /// Replaces existing translation content so re-running the seeder after a
    /// file edit brings the row back in sync. `source_sha` captures the first
    /// 16 hex chars of `sha256(en.md)` at translation time; the loader uses it
    /// later to detect drift when English content changes.
    async fn seed_upsert_coach_translation(
        &self,
        translation: &SeedCoachTranslation,
    ) -> AppResult<()>;
}

// ================================

/// Store listings for the coach marketplace (cross-tenant browsing, install/uninstall)
#[async_trait]
pub trait StoreListingsRepository: Send + Sync {
    /// Submit a coach for Store review (creates listing if needed)
    async fn submit_for_review(
        &self,
        coach_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<StoreListing>;
    /// Get a store listing by coach ID
    async fn get_listing(&self, coach_id: &str) -> AppResult<Option<StoreListing>>;
    /// Approve a coach and publish to the Store
    async fn approve_coach(
        &self,
        coach_id: &str,
        tenant_id: TenantId,
        admin_user_id: Option<Uuid>,
    ) -> AppResult<CoachWithListing>;
    /// Reject a coach with a reason
    async fn reject_coach(
        &self,
        coach_id: &str,
        tenant_id: TenantId,
        admin_user_id: Option<Uuid>,
        reason: &str,
    ) -> AppResult<CoachWithListing>;
    /// Unpublish a coach (revert from published to draft)
    async fn unpublish_coach(
        &self,
        coach_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<CoachWithListing>;
    /// Get coaches pending admin review
    async fn get_pending_review_coaches(
        &self,
        tenant_id: TenantId,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> AppResult<Vec<CoachWithListing>>;
    /// Get coaches that have been rejected
    async fn get_rejected_coaches(
        &self,
        tenant_id: TenantId,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> AppResult<Vec<CoachWithListing>>;
    /// Get store admin statistics
    async fn get_store_admin_stats(&self, tenant_id: TenantId) -> AppResult<StoreAdminStats>;
    /// Get author email for a coach
    async fn get_author_email(&self, user_id: Uuid) -> AppResult<Option<String>>;
    /// Get published coaches for the Store (cross-tenant)
    async fn get_published_coaches(
        &self,
        category: Option<CoachCategory>,
        sort_by: Option<&str>,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> AppResult<Vec<CoachWithListing>>;
    /// Get published coaches with cursor-based pagination
    async fn get_published_coaches_cursor(
        &self,
        category: Option<CoachCategory>,
        sort_by: StoreSortOrder,
        limit: u32,
        cursor: Option<&str>,
    ) -> AppResult<CursorPage<CoachWithListing>>;
    /// Search published coaches by title/description/tags
    async fn search_published_coaches(
        &self,
        query: &str,
        limit: Option<u32>,
    ) -> AppResult<Vec<CoachWithListing>>;
    /// Get a single published coach by ID (cross-tenant)
    async fn get_published_coach(&self, coach_id: &str) -> AppResult<Option<CoachWithListing>>;
    /// Get category counts for published coaches
    async fn get_category_counts(&self) -> AppResult<HashMap<CoachCategory, i64>>;
    /// Increment install count for a coach's store listing
    async fn increment_install_count(&self, coach_id: &str) -> AppResult<()>;
    /// Decrement install count for a coach's store listing
    async fn decrement_install_count(&self, coach_id: &str) -> AppResult<()>;
    /// Install a coach from the Store (creates user's copy)
    async fn install_from_store(
        &self,
        source_coach_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Coach>;
    /// Uninstall a coach (deletes user's copy, returns source coach ID)
    async fn uninstall_coach(
        &self,
        coach_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<String>;
    /// Get user's installed coaches from the Store
    async fn get_installed_coaches(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Vec<Coach>>;
    /// Create or ensure a store listing exists for a coach
    async fn ensure_listing(&self, coach_id: &str, tenant_id: TenantId) -> AppResult<StoreListing>;
}

// ================================
// Coaching Group Repository
// ================================

/// Coaching group storage, membership management, and invite tracking
#[async_trait]
pub trait CoachingGroupRepository: Send + Sync {
    // -- Group CRUD --

    /// Create a new coaching group
    async fn create_group(
        &self,
        tenant_id: TenantId,
        group: &CoachingGroup,
    ) -> AppResult<CoachingGroup>;

    /// Get a group by ID with tenant isolation
    async fn get_group(
        &self,
        group_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<Option<CoachingGroup>>;

    /// List groups the user belongs to (as member, admin, or owner).
    /// Membership-based lookup — no tenant filter since members join cross-tenant.
    async fn list_groups_for_user(&self, user_id: Uuid) -> AppResult<Vec<GroupSummary>>;

    /// List groups that use a specific coach persona
    async fn list_groups_for_coach(
        &self,
        coach_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<Vec<CoachingGroup>>;

    /// Update a coaching group
    async fn update_group(
        &self,
        group_id: &str,
        tenant_id: TenantId,
        request: &UpdateGroupRequest,
    ) -> AppResult<Option<CoachingGroup>>;

    /// Soft-delete a coaching group (sets `is_active` = false)
    async fn delete_group(&self, group_id: &str, tenant_id: TenantId) -> AppResult<bool>;

    // -- Membership --

    /// Add a member to a group
    async fn add_member(&self, member: &GroupMember) -> AppResult<GroupMember>;

    /// Remove a member from a group (soft removal via `left_at` timestamp).
    /// No tenant filter — members join cross-tenant via invite codes.
    async fn remove_member(&self, group_id: &str, user_id: Uuid) -> AppResult<bool>;

    /// Get member by `group_id` + `user_id` (unique constraint, no tenant filter needed)
    async fn get_member(&self, group_id: &str, user_id: Uuid) -> AppResult<Option<GroupMember>>;

    /// List active members of a group.
    /// No tenant filter — members join cross-tenant via invite codes.
    async fn list_members(&self, group_id: &str) -> AppResult<Vec<GroupMember>>;

    /// Update a member's role.
    /// No tenant filter — admins manage cross-tenant members.
    async fn update_member_role(
        &self,
        group_id: &str,
        user_id: Uuid,
        role: GroupRole,
    ) -> AppResult<bool>;

    /// Update a member's peer sharing consent.
    /// No tenant filter — members update their own consent cross-tenant.
    async fn update_peer_sharing_consent(
        &self,
        group_id: &str,
        user_id: Uuid,
        consent: bool,
    ) -> AppResult<bool>;

    /// Count active members in a group.
    /// No tenant filter — members join cross-tenant via invite codes.
    async fn count_members(&self, group_id: &str) -> AppResult<i64>;

    // -- Invites --

    /// Create a group invite
    async fn create_invite(&self, invite: &GroupInvite) -> AppResult<GroupInvite>;

    /// Look up an invite by its code (cross-tenant for join flow)
    async fn get_invite_by_code(&self, code: &str) -> AppResult<Option<GroupInvite>>;

    /// Increment the use count of an invite
    async fn increment_invite_use_count(&self, invite_id: &str) -> AppResult<bool>;

    /// Deactivate an invite. Invite IDs are globally unique — no tenant filter needed.
    async fn deactivate_invite(&self, invite_id: &str) -> AppResult<bool>;

    /// List invites for a group.
    /// No tenant filter — cross-tenant admins view invites by `group_id`.
    async fn list_invites(&self, group_id: &str) -> AppResult<Vec<GroupInvite>>;

    // -- Context queries --

    /// Find groups a user belongs to that use a specific coach.
    /// No tenant filter — groups span tenants via cross-tenant membership.
    async fn find_groups_for_user_and_coach(
        &self,
        user_id: Uuid,
        coach_id: &str,
    ) -> AppResult<Vec<CoachingGroup>>;

    /// Count groups owned by a user (for tier limit enforcement)
    async fn count_groups_for_owner(&self, owner_id: Uuid, tenant_id: TenantId) -> AppResult<i64>;
}

// ================================
// Health persistence repository traits
// ================================

/// Data source (device/provider) tracking repository
#[async_trait]
pub trait DataSourceRepository: Send + Sync {
    /// Upsert a data source record (insert or update on conflict)
    async fn upsert_data_source(
        &self,
        tenant_id: &TenantId,
        source: &DataSource,
    ) -> AppResult<String>;
    /// Get a data source by ID
    async fn get_data_source(&self, id: &str) -> AppResult<Option<DataSource>>;
    /// List data sources for a user
    async fn list_data_sources(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
    ) -> AppResult<Vec<DataSource>>;
    /// List data sources for a user filtered by provider
    async fn list_data_sources_by_provider(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        provider: &str,
    ) -> AppResult<Vec<DataSource>>;
    /// Delete a data source
    async fn delete_data_source(&self, id: &str) -> AppResult<()>;
}

/// Sleep session persistence repository
#[async_trait]
pub trait SleepRepository: Send + Sync {
    /// Insert or update a sleep session
    async fn upsert_sleep_session(
        &self,
        tenant_id: &TenantId,
        session: &StoredSleepSession,
    ) -> AppResult<String>;
    /// Get sleep sessions for a user within a date range
    async fn get_sleep_sessions(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> AppResult<Vec<StoredSleepSession>>;
    /// Get the most recent sleep session for a user
    async fn get_latest_sleep_session(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
    ) -> AppResult<Option<StoredSleepSession>>;
    /// Delete sleep sessions for a user and provider
    async fn delete_sleep_sessions(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        provider: &str,
    ) -> AppResult<u64>;
}

/// Recovery metrics persistence repository
#[async_trait]
pub trait RecoveryRepository: Send + Sync {
    /// Insert or update recovery metrics for a date
    async fn upsert_recovery_metrics(
        &self,
        tenant_id: &TenantId,
        metrics: &StoredRecoveryMetrics,
    ) -> AppResult<String>;
    /// Get recovery metrics for a user within a date range
    async fn get_recovery_metrics(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> AppResult<Vec<StoredRecoveryMetrics>>;
    /// Get the most recent recovery metrics for a user
    async fn get_latest_recovery(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
    ) -> AppResult<Option<StoredRecoveryMetrics>>;
}

/// Health snapshot persistence repository
#[async_trait]
pub trait HealthSnapshotRepository: Send + Sync {
    /// Insert or update a health snapshot for a date
    async fn upsert_health_snapshot(
        &self,
        tenant_id: &TenantId,
        snapshot: &StoredHealthMetrics,
    ) -> AppResult<String>;
    /// Get health snapshots for a user within a date range
    async fn get_health_snapshots(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> AppResult<Vec<StoredHealthMetrics>>;
    /// Get the most recent health snapshot for a user
    async fn get_latest_health_snapshot(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
    ) -> AppResult<Option<StoredHealthMetrics>>;
}

// ================================
// Sync Cursor Repository
// ================================

/// Sync cursor repository for CDC-based incremental sync tracking
#[async_trait]
pub trait SyncCursorRepository: Send + Sync {
    /// Get the sync cursor for a user+provider+`data_type`
    async fn get_sync_cursor(
        &self,
        user_id: &str,
        tenant_id: &TenantId,
        provider: &str,
        data_type: &str,
    ) -> AppResult<Option<SyncCursorRow>>;

    /// Upsert (insert or update) a sync cursor
    async fn upsert_sync_cursor(&self, cursor: &SyncCursorRow) -> AppResult<()>;

    /// List all connected users for a given provider
    async fn list_connected_provider_users(
        &self,
        provider: &str,
    ) -> AppResult<Vec<ConnectedUserRow>>;
}

// ================================
// Endurance typed physiological profile + dossier repos
// ================================

/// Typed CRUD for [`UserPhysiologicalProfile`] backed by the
/// `user_physiological_profiles` table.
///
/// Row layout: see `migrations/20260430000003_user_profile_endurance_fields.sql`
/// (`SQLite`) and `migrations_pg/20260430000003_user_profile_endurance_fields.sql`
/// (`PostgreSQL`).
///
/// Every method scopes by `tenant_id` to satisfy the multi-tenant isolation
/// invariant in CLAUDE.md.
#[async_trait]
pub trait UserPhysiologicalProfileRepository: Send + Sync {
    /// Insert or update the profile row for `(tenant_id, user_id)`.
    ///
    /// `profile.user_id` must match `user_id`; the implementation rejects
    /// mismatches with [`pierre_core::errors::AppError`] to prevent
    /// cross-user writes from a confused caller.
    async fn upsert_user_physiological_profile(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
        profile: &UserPhysiologicalProfile,
    ) -> AppResult<()>;

    /// Fetch the profile for `(tenant_id, user_id)`. Returns `None` when
    /// the user has no row yet.
    async fn get_user_physiological_profile(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
    ) -> AppResult<Option<UserPhysiologicalProfile>>;
}

/// Read-time composer for the Endurance [`Dossier`] aggregate.
///
/// Per the locked architectural decision the dossier is **not** persisted as
/// its own row — the implementation pulls from the existing tables
/// (`user_physiological_profiles` for physiology + zones, `user_profiles`
/// JSON column for goals / nutrition / equipment) and assembles the
/// aggregate per request. Cache invalidation is therefore unnecessary on
/// the dossier itself; only the underlying tables need cache hooks.
#[async_trait]
pub trait DossierRepository: Send + Sync {
    /// Compose the dossier for `(tenant_id, user_id)`.
    ///
    /// Returns an empty dossier shell (all slots `None` / empty) when the
    /// user has no underlying rows so the API endpoint can return a 200
    /// rather than a 404 for fresh accounts.
    async fn compose_dossier(&self, tenant_id: TenantId, user_id: Uuid) -> AppResult<Dossier>;
}

/// Audit-trail CRUD for the `prescribed_workouts` table.
///
/// Each row records a single prescription pushed to a provider calendar
/// (Endurance Phase 5). The table is append-only from the API path; the
/// repo exposes upsert by id (so retries can update the
/// `provider_event_id`) plus a tenant-scoped recent-list read.
#[async_trait]
pub trait PrescribedWorkoutRepository: Send + Sync {
    /// Insert or update a prescription audit row.
    async fn upsert_prescribed_workout(&self, prescribed: &PrescribedWorkout) -> AppResult<()>;

    /// List the most recent `limit` prescriptions for a (`tenant_id`, `user_id`).
    async fn list_prescribed_workouts(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
        limit: u32,
    ) -> AppResult<Vec<PrescribedWorkout>>;
}

/// CRUD for user-authored Endurance workout templates.
///
/// The six compiled-in cornerstones live in `workout_templates/*.toml` and
/// are loaded by `pierre_server::services::workout_library`. This repo only
/// owns rows the user authored at runtime: every persisted row therefore
/// carries `tenant_id` and `user_id` (the migration tolerates `NULL` for
/// historical reasons but the trait rejects either as `None` so the table
/// never duplicates the read-only cornerstone library).
#[async_trait]
pub trait WorkoutTemplateRepository: Send + Sync {
    /// Insert or update a user-authored workout template.
    ///
    /// `template.tenant_id` and `template.user_id` MUST both be `Some` — the
    /// implementation returns an [`AppError::invalid_input`] otherwise.
    async fn upsert_workout_template(&self, template: &WorkoutTemplate) -> AppResult<()>;

    /// List all user-authored templates for (`tenant_id`, `user_id`),
    /// ordered by `updated_at` descending.
    async fn list_user_workout_templates(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
    ) -> AppResult<Vec<WorkoutTemplate>>;

    /// Look up a single user-authored template by slug. Returns `None`
    /// when no row matches the (`tenant_id`, `user_id`, `slug`) tuple.
    async fn get_user_workout_template(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
        slug: &str,
    ) -> AppResult<Option<WorkoutTemplate>>;
}

/// CRUD for the `route_summaries` cache table.
///
/// Stores parsed-GPX terrain + climbs JSON keyed by `(tenant_id, user_id,
/// activity_id)` so the route endpoint can skip re-parsing when the
/// underlying GPX hash matches. Cache freshness check is the
/// caller's responsibility — `get_route_summary` returns `None` when the
/// row is missing OR when the supplied `expected_hash` does not match.
#[async_trait]
pub trait RouteSummaryRepository: Send + Sync {
    /// Insert or update the cached terrain + climbs JSON for an activity.
    async fn upsert_route_summary(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
        activity_id: &str,
        gpx_hash: &str,
        terrain_summary_json: &str,
        climbs_json: &str,
    ) -> AppResult<()>;

    /// Fetch the cached entry. Returns `None` when the row is missing or
    /// when `expected_hash` does not match the stored `gpx_hash`.
    async fn get_route_summary(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
        activity_id: &str,
        expected_hash: &str,
    ) -> AppResult<Option<(String, String)>>;
}

/// Daily training-state rollup CRUD backing the `training_history` table.
///
/// Each row captures one day of derived training metrics (CTL/ATL/TSB/ACWR/
/// monotony/strain/`ramp_rate`/`daily_load`) for a single (`tenant_id`, `user_id`).
/// Computation is the responsibility of
/// [`pierre_intelligence::training_history_compute`]; this repo only persists.
#[async_trait]
pub trait TrainingHistoryRepository: Send + Sync {
    /// Insert or update a single day's row.
    async fn upsert_training_history_day(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
        state: &DailyTrainingState,
    ) -> AppResult<()>;

    /// Insert or update many days at once. Implementations may batch the
    /// underlying writes; callers must not assume atomicity across rows.
    async fn upsert_training_history_batch(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
        states: &[DailyTrainingState],
    ) -> AppResult<()>;

    /// Fetch all rows in `[from, to]` (inclusive on both ends) in
    /// chronological order (oldest first).
    async fn get_training_history(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
        from: chrono::NaiveDate,
        to: chrono::NaiveDate,
    ) -> AppResult<Vec<DailyTrainingState>>;

    /// Fetch the most recent row for the user, if any.
    async fn latest_training_history(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
    ) -> AppResult<Option<DailyTrainingState>>;
}

// ================================
// Database lifecycle trait (connection + migration only)
// ================================

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
