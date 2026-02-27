// ABOUTME: Repository trait definitions and DatabaseProvider compound supertrait for database abstraction
// ABOUTME: Defines focused, cohesive repository traits that backends implement directly
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::admin::jwt::JwtSigner;
use pierre_core::admin::models::{
    AdminToken, AdminTokenUsage, CreateAdminTokenRequest, GeneratedAdminToken,
};
use pierre_core::config::FitnessConfig;
use pierre_core::errors::AppResult;
use pierre_core::models::a2a::{
    A2AClient, A2ASession, A2ATask, A2AUsage, A2AUsageStats, TaskStatus,
};
use pierre_core::models::coaches::{
    Coach, CoachListItem, CreateCoachRequest, ListCoachesFilter, UpdateCoachRequest,
};
use pierre_core::models::messaging::{
    ChannelBindingRecord, CreateChannelBindingParams, CreateMessagingConnectionParams,
    MessagingConnectionRecord,
};
use pierre_core::models::mobility::{
    ActivityMuscleMapping, ListStretchingFilter, ListYogaFilter, StretchingExercise, YogaPose,
};
use pierre_core::models::recipes::{MealTiming, Recipe, ValidatedNutrition};
use pierre_core::models::usage::{InsertLlmUsage, LlmUsageAggregateRow, LlmUsageDailyRow};
use pierre_core::models::{
    AdaptedInsight, ApiKey, ApiKeyUsage, ApiKeyUsageStats, AuthorizationCode, ConnectionType,
    ConversationRecord, ConversationSummary, CreateUserMcpTokenRequest, FriendConnection,
    FriendStatus, InsightReaction, OAuth2AuthCode, OAuth2Client, OAuth2RefreshToken, OAuth2State,
    OAuthApp, OAuthClientState, OAuthNotification, ProviderConnection, SharedInsight, Tenant,
    TenantPlan, TenantToolOverride, ToolCatalogEntry, ToolCategory, User, UserMcpToken,
    UserMcpTokenCreated, UserMcpTokenInfo, UserOAuthApp, UserOAuthToken, UserSocialSettings,
    UserStatus,
};
use pierre_core::models::{
    AddMessageParams, JwtUsage, LlmUsageRecord, RequestLog, ToolUsage, UsageCounterRecord,
};
use pierre_core::models::{
    AuditEvent, KeyVersion, LlmCredentialRecord, LlmCredentialSummary, MessageRecord, TenantId,
    TenantOAuthCredentials,
};
use pierre_core::pagination::{CursorPage, PaginationParams};
use pierre_core::permissions::impersonation::ImpersonationSession;
use serde_json::Value;
use uuid::Uuid;

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
    /// Create a new chat conversation
    async fn create_conversation(
        &self,
        user_id: &str,
        tenant_id: TenantId,
        title: &str,
        model: &str,
        system_prompt: Option<&str>,
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
}

/// LLM usage tracking repository for cost analysis and quota enforcement
#[async_trait]
pub trait LlmUsageRepository: Send + Sync {
    /// Insert a new LLM usage record
    async fn insert_llm_usage(&self, params: &InsertLlmUsage<'_>) -> AppResult<LlmUsageRecord>;

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

// ================================
// Messaging integration repository
// ================================

/// Messaging provider connection and channel binding repository (tenant-scoped)
#[async_trait]
pub trait MessagingRepository: Send + Sync {
    /// Create a new messaging connection record
    async fn create_messaging_connection(
        &self,
        params: &CreateMessagingConnectionParams<'_>,
    ) -> AppResult<MessagingConnectionRecord>;

    /// Get a messaging connection by ID (tenant-scoped)
    async fn get_messaging_connection(
        &self,
        id: &str,
        tenant_id: TenantId,
    ) -> AppResult<Option<MessagingConnectionRecord>>;

    /// Find a messaging connection by provider and team ID
    ///
    /// Used by webhook handlers to resolve which tenant/connection a webhook belongs to.
    async fn get_messaging_connection_by_team(
        &self,
        provider: &str,
        team_id: &str,
    ) -> AppResult<Option<MessagingConnectionRecord>>;

    /// List all messaging connections for a tenant
    async fn list_messaging_connections(
        &self,
        tenant_id: TenantId,
    ) -> AppResult<Vec<MessagingConnectionRecord>>;

    /// Delete a messaging connection (tenant-scoped)
    async fn delete_messaging_connection(&self, id: &str, tenant_id: TenantId) -> AppResult<bool>;

    /// Create a new channel binding
    async fn create_channel_binding(
        &self,
        params: &CreateChannelBindingParams<'_>,
    ) -> AppResult<ChannelBindingRecord>;

    /// Find a channel binding by connection ID and channel ID
    ///
    /// Used by webhook handlers to resolve which conversation an incoming message maps to.
    async fn get_channel_binding_by_channel(
        &self,
        connection_id: &str,
        channel_id: &str,
    ) -> AppResult<Option<ChannelBindingRecord>>;

    /// List all channel bindings for a tenant
    async fn list_channel_bindings(
        &self,
        tenant_id: TenantId,
    ) -> AppResult<Vec<ChannelBindingRecord>>;

    /// Delete a channel binding (tenant-scoped)
    async fn delete_channel_binding(&self, id: &str, tenant_id: TenantId) -> AppResult<bool>;
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
