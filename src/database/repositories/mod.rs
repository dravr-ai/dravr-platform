// ABOUTME: Repository trait definitions for database abstraction
// ABOUTME: Breaks down the monolithic DatabaseProvider into focused, cohesive repository traits
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2025 Pierre Fitness Intelligence

use crate::a2a::auth::A2AClient;
use crate::a2a::client::A2ASession;
use crate::a2a::protocol::{A2ATask, TaskStatus};
use crate::admin::jwks::JwksManager;
use crate::admin::models::{
    AdminToken, AdminTokenUsage, CreateAdminTokenRequest, GeneratedAdminToken,
};
use crate::api_keys::{ApiKey, ApiKeyUsage, ApiKeyUsageStats};
use crate::config::fitness::FitnessConfig;
use crate::dashboard_routes::{RequestLog, ToolUsage};
use crate::database::coaches::{Coach, CreateCoachRequest, ListCoachesFilter, UpdateCoachRequest};
use crate::database::mobility::{
    ActivityMuscleMapping, ListStretchingFilter, ListYogaFilter, StretchingExercise, YogaPose,
};
use crate::database::{
    A2AUsage, A2AUsageStats, ConversationRecord, ConversationSummary, CreateUserMcpTokenRequest,
    DatabaseError, MessageRecord, UserMcpToken, UserMcpTokenCreated, UserMcpTokenInfo,
};
use crate::intelligence::recipes::{MealTiming, Recipe, ValidatedNutrition};
use crate::models::{
    AdaptedInsight, AuthorizationCode, ConnectionType, FriendConnection, FriendStatus,
    InsightReaction, OAuthApp, OAuthNotification, ProviderConnection, SharedInsight, Tenant,
    TenantPlan, TenantToolOverride, ToolCatalogEntry, ToolCategory, User, UserOAuthApp,
    UserOAuthToken, UserSocialSettings, UserStatus,
};
use crate::oauth2_client::OAuthClientState;
use crate::oauth2_server::models::{OAuth2AuthCode, OAuth2Client, OAuth2RefreshToken, OAuth2State};
use crate::pagination::{CursorPage, PaginationParams};
use crate::permissions::impersonation::ImpersonationSession;
use crate::rate_limiting::JwtUsage;
use crate::security::audit::AuditEvent;
use crate::security::key_rotation::KeyVersion;
use crate::tenant::llm_manager::{LlmCredentialRecord, LlmCredentialSummary};
use crate::tenant::oauth_manager::TenantOAuthCredentials;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::models::TenantId;
use serde_json::Value;
use uuid::Uuid;

// ================================
// Repository Trait Definitions (21 with blanket impls)
// ================================

/// User account management repository
#[async_trait]
pub trait UserRepository: Send + Sync {
    /// Create a new user account
    async fn create(&self, user: &User) -> Result<Uuid, DatabaseError>;
    /// Get user by ID, scoped to a specific tenant for multi-tenant isolation
    async fn get(&self, user_id: Uuid, tenant_id: TenantId) -> Result<Option<User>, DatabaseError>;
    /// Get user by ID without tenant scoping (for system-level operations)
    async fn get_global(&self, user_id: Uuid) -> Result<Option<User>, DatabaseError>;
    /// Get user by email address
    async fn get_by_email(&self, email: &str) -> Result<Option<User>, DatabaseError>;
    /// Get user by email (required - fails if not found)
    async fn get_by_email_required(&self, email: &str) -> Result<User, DatabaseError>;
    /// Get user by Firebase UID
    async fn get_by_firebase_uid(&self, firebase_uid: &str) -> Result<Option<User>, DatabaseError>;
    /// Update user's last active timestamp
    async fn update_last_active(&self, user_id: Uuid) -> Result<(), DatabaseError>;
    /// Get total number of users
    async fn count(&self) -> Result<i64, DatabaseError>;
    /// Get users by status (pending, active, suspended), optionally scoped to a tenant
    async fn get_by_status(
        &self,
        status: &str,
        tenant_id: Option<TenantId>,
    ) -> Result<Vec<User>, DatabaseError>;
    /// Get users by status with cursor-based pagination
    async fn get_by_status_cursor(
        &self,
        status: &str,
        params: &PaginationParams,
    ) -> Result<CursorPage<User>, DatabaseError>;
    /// Update user status and approval information
    async fn update_status(
        &self,
        user_id: Uuid,
        new_status: UserStatus,
        approved_by: Option<Uuid>,
    ) -> Result<User, DatabaseError>;
    /// Update user's `tenant_id` to link them to a tenant (`tenant_id` should be UUID string)
    async fn update_tenant_id(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> Result<(), DatabaseError>;
    /// Update user's password hash
    async fn update_password(
        &self,
        user_id: Uuid,
        password_hash: &str,
    ) -> Result<(), DatabaseError>;
    /// Update user's display name
    async fn update_display_name(
        &self,
        user_id: Uuid,
        display_name: &str,
    ) -> Result<User, DatabaseError>;
    /// Delete a user and all associated data
    async fn delete(&self, user_id: Uuid) -> Result<(), DatabaseError>;
    /// Get the first admin user by creation date
    async fn get_first_admin_user(&self) -> Result<Option<User>, DatabaseError>;
    /// Check if a user has synthetic activities seeded
    async fn has_synthetic_activities(&self, user_id: Uuid) -> Result<bool, DatabaseError>;
}

/// OAuth token storage repository (tenant-scoped, includes OAuth apps and sync tracking)
#[async_trait]
pub trait OAuthTokenRepository: Send + Sync {
    /// Store or update user OAuth token for a tenant-provider combination
    async fn upsert_token(&self, token: &UserOAuthToken) -> Result<(), DatabaseError>;
    /// Get user OAuth token for a specific tenant-provider combination
    async fn get_token(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) -> Result<Option<UserOAuthToken>, DatabaseError>;
    /// Get all OAuth tokens for a user, optionally scoped to a specific tenant
    async fn get_tokens(
        &self,
        user_id: Uuid,
        tenant_id: Option<TenantId>,
    ) -> Result<Vec<UserOAuthToken>, DatabaseError>;
    /// Get all OAuth tokens for a tenant-provider combination
    async fn get_tenant_provider_tokens(
        &self,
        tenant_id: TenantId,
        provider: &str,
    ) -> Result<Vec<UserOAuthToken>, DatabaseError>;
    /// Delete user OAuth token for a tenant-provider combination
    async fn delete_token(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) -> Result<(), DatabaseError>;
    /// Delete all OAuth tokens for a user within a tenant scope
    async fn delete_tokens(&self, user_id: Uuid, tenant_id: TenantId) -> Result<(), DatabaseError>;
    /// Update OAuth token expiration and refresh info
    async fn refresh_token(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        access_token: &str,
        refresh_token: Option<&str>,
        expires_at: Option<DateTime<Utc>>,
    ) -> Result<(), DatabaseError>;
    /// Store user OAuth app credentials (`client_id`, `client_secret`)
    async fn store_user_oauth_app(
        &self,
        user_id: Uuid,
        provider: &str,
        client_id: &str,
        client_secret: &str,
        redirect_uri: &str,
    ) -> Result<(), DatabaseError>;
    /// Get user OAuth app credentials for a provider
    async fn get_user_oauth_app(
        &self,
        user_id: Uuid,
        provider: &str,
    ) -> Result<Option<UserOAuthApp>, DatabaseError>;
    /// List all OAuth app providers configured for a user
    async fn list_user_oauth_apps(&self, user_id: Uuid)
        -> Result<Vec<UserOAuthApp>, DatabaseError>;
    /// Remove user OAuth app credentials for a provider
    async fn remove_user_oauth_app(
        &self,
        user_id: Uuid,
        provider: &str,
    ) -> Result<(), DatabaseError>;
    /// Get last sync timestamp for a provider within a specific tenant
    async fn get_provider_last_sync(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) -> Result<Option<DateTime<Utc>>, DatabaseError>;
    /// Update last sync timestamp for a provider within a specific tenant
    async fn update_provider_last_sync(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        sync_time: DateTime<Utc>,
    ) -> Result<(), DatabaseError>;
}

/// API key management repository
#[async_trait]
pub trait ApiKeyRepository: Send + Sync {
    /// Create a new API key
    async fn create(&self, api_key: &ApiKey) -> Result<(), DatabaseError>;
    /// Get API key by its prefix and hash
    async fn get_by_prefix(
        &self,
        prefix: &str,
        hash: &str,
    ) -> Result<Option<ApiKey>, DatabaseError>;
    /// Get all API keys for a user
    async fn get_for_user(&self, user_id: Uuid) -> Result<Vec<ApiKey>, DatabaseError>;
    /// Update API key last used timestamp
    async fn update_last_used(&self, api_key_id: &str) -> Result<(), DatabaseError>;
    /// Deactivate an API key
    async fn deactivate(&self, api_key_id: &str, user_id: Uuid) -> Result<(), DatabaseError>;
    /// Get API key by ID, optionally scoped to a specific user for ownership enforcement
    async fn get_by_id(
        &self,
        api_key_id: &str,
        user_id: Option<Uuid>,
    ) -> Result<Option<ApiKey>, DatabaseError>;
    /// Get API keys with optional filters
    async fn get_filtered(
        &self,
        user_email: Option<&str>,
        active_only: bool,
        limit: Option<i32>,
        offset: Option<i32>,
    ) -> Result<Vec<ApiKey>, DatabaseError>;
    /// Clean up expired API keys
    async fn cleanup_expired(&self) -> Result<u64, DatabaseError>;
    /// Get expired API keys
    async fn get_expired(&self) -> Result<Vec<ApiKey>, DatabaseError>;
}

/// Usage tracking and analytics repository
#[async_trait]
pub trait UsageRepository: Send + Sync {
    /// Record API key usage
    async fn record_api_key(&self, usage: &ApiKeyUsage) -> Result<(), DatabaseError>;
    /// Get current usage count for an API key
    async fn get_api_key_current(&self, api_key_id: &str) -> Result<u32, DatabaseError>;
    /// Get usage statistics for an API key
    async fn get_api_key_stats(
        &self,
        api_key_id: &str,
        start_date: DateTime<Utc>,
        end_date: DateTime<Utc>,
    ) -> Result<ApiKeyUsageStats, DatabaseError>;
    /// Record JWT token usage for rate limiting and analytics
    async fn record_jwt_usage(&self, usage: &JwtUsage) -> Result<(), DatabaseError>;
    /// Get current JWT usage count for rate limiting (current month)
    async fn get_jwt_current_usage(&self, user_id: Uuid) -> Result<u32, DatabaseError>;
    /// Get request logs with filtering options
    async fn get_request_logs(
        &self,
        user_id: Option<Uuid>,
        api_key_id: Option<&str>,
        start_time: Option<DateTime<Utc>>,
        end_time: Option<DateTime<Utc>>,
        status_filter: Option<&str>,
        tool_filter: Option<&str>,
    ) -> Result<Vec<RequestLog>, DatabaseError>;
    /// Get system statistics, optionally scoped to a tenant
    async fn get_system_stats(
        &self,
        tenant_id: Option<TenantId>,
    ) -> Result<(u64, u64), DatabaseError>;
    /// Get top tools analysis for dashboard
    async fn get_top_tools_analysis(
        &self,
        user_id: Uuid,
        start_time: DateTime<Utc>,
        end_time: DateTime<Utc>,
    ) -> Result<Vec<ToolUsage>, DatabaseError>;
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
    ) -> Result<String, DatabaseError>;
    /// Get A2A client by ID
    async fn get_client(&self, client_id: &str) -> Result<Option<A2AClient>, DatabaseError>;
    /// Get A2A client by API key ID
    async fn get_client_by_api_key_id(
        &self,
        api_key_id: &str,
    ) -> Result<Option<A2AClient>, DatabaseError>;
    /// Get A2A client by name
    async fn get_client_by_name(&self, name: &str) -> Result<Option<A2AClient>, DatabaseError>;
    /// List all A2A clients for a user
    async fn list_clients(&self, user_id: &Uuid) -> Result<Vec<A2AClient>, DatabaseError>;
    /// Deactivate an A2A client
    async fn deactivate_client(&self, client_id: &str) -> Result<(), DatabaseError>;
    /// Get client credentials for authentication
    async fn get_client_credentials(
        &self,
        client_id: &str,
    ) -> Result<Option<(String, String)>, DatabaseError>;
    /// Invalidate all active sessions for a client
    async fn invalidate_client_sessions(&self, client_id: &str) -> Result<(), DatabaseError>;
    /// Deactivate all API keys associated with a client
    async fn deactivate_client_api_keys(&self, client_id: &str) -> Result<(), DatabaseError>;
    /// Create a new A2A session
    async fn create_session(
        &self,
        client_id: &str,
        user_id: Option<&Uuid>,
        granted_scopes: &[String],
        expires_in_hours: i64,
    ) -> Result<String, DatabaseError>;
    /// Get A2A session by token
    async fn get_session(&self, session_token: &str) -> Result<Option<A2ASession>, DatabaseError>;
    /// Update A2A session activity timestamp
    async fn update_session_activity(&self, session_token: &str) -> Result<(), DatabaseError>;
    /// Get active sessions for a specific client
    async fn get_active_sessions(&self, client_id: &str) -> Result<Vec<A2ASession>, DatabaseError>;
    /// Create a new A2A task
    async fn create_task(
        &self,
        client_id: &str,
        session_id: Option<&str>,
        task_type: &str,
        input_data: &Value,
    ) -> Result<String, DatabaseError>;
    /// Get A2A task by ID
    async fn get_task(&self, task_id: &str) -> Result<Option<A2ATask>, DatabaseError>;
    /// List A2A tasks for a client with optional filtering
    async fn list_tasks(
        &self,
        client_id: Option<&str>,
        status_filter: Option<&TaskStatus>,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> Result<Vec<A2ATask>, DatabaseError>;
    /// Update A2A task status
    async fn update_task_status(
        &self,
        task_id: &str,
        status: &TaskStatus,
        result: Option<&Value>,
        error: Option<&str>,
    ) -> Result<(), DatabaseError>;
    /// Record A2A usage for analytics
    async fn record_usage(&self, usage: &A2AUsage) -> Result<(), DatabaseError>;
    /// Get current A2A usage count for a client
    async fn get_client_current_usage(&self, client_id: &str) -> Result<u32, DatabaseError>;
    /// Get A2A usage statistics for a client
    async fn get_usage_stats(
        &self,
        client_id: &str,
        start_date: DateTime<Utc>,
        end_date: DateTime<Utc>,
    ) -> Result<A2AUsageStats, DatabaseError>;
    /// Get A2A client usage history
    async fn get_client_usage_history(
        &self,
        client_id: &str,
        days: u32,
    ) -> Result<Vec<(DateTime<Utc>, u32, u32)>, DatabaseError>;
}

/// User profiles, goals, and configuration repository
#[async_trait]
pub trait ProfileRepository: Send + Sync {
    /// Upsert user profile data
    async fn upsert_profile(&self, user_id: Uuid, profile_data: Value)
        -> Result<(), DatabaseError>;
    /// Get user profile data
    async fn get_profile(&self, user_id: Uuid) -> Result<Option<Value>, DatabaseError>;
    /// Create a new goal for a user
    async fn create_goal(&self, user_id: Uuid, goal_data: Value) -> Result<String, DatabaseError>;
    /// Get all goals for a user
    async fn get_goals(&self, user_id: Uuid) -> Result<Vec<Value>, DatabaseError>;
    /// Update progress on a goal, scoped to the owning user
    async fn update_goal_progress(
        &self,
        goal_id: &str,
        user_id: Uuid,
        current_value: f64,
    ) -> Result<(), DatabaseError>;
    /// Get user configuration data
    async fn get_configuration(&self, user_id: &str) -> Result<Option<String>, DatabaseError>;
    /// Save user configuration data
    async fn save_configuration(
        &self,
        user_id: &str,
        config_json: &str,
    ) -> Result<(), DatabaseError>;
}

/// AI-generated insights storage repository
#[async_trait]
pub trait InsightRepository: Send + Sync {
    /// Store an AI-generated insight
    async fn store(&self, user_id: Uuid, insight_data: Value) -> Result<String, DatabaseError>;
    /// Get insights for a user
    async fn get_for_user(
        &self,
        user_id: Uuid,
        insight_type: Option<&str>,
        limit: Option<u32>,
    ) -> Result<Vec<Value>, DatabaseError>;
}

/// Admin token management repository
#[async_trait]
pub trait AdminRepository: Send + Sync {
    /// Create a new admin token
    async fn create_token(
        &self,
        request: &CreateAdminTokenRequest,
        admin_jwt_secret: &str,
        jwks_manager: &JwksManager,
    ) -> Result<GeneratedAdminToken, DatabaseError>;
    /// Get admin token by ID
    async fn get_token_by_id(&self, token_id: &str) -> Result<Option<AdminToken>, DatabaseError>;
    /// Get admin token by prefix for fast lookup
    async fn get_token_by_prefix(
        &self,
        token_prefix: &str,
    ) -> Result<Option<AdminToken>, DatabaseError>;
    /// List all admin tokens (super admin only)
    async fn list_tokens(&self, include_inactive: bool) -> Result<Vec<AdminToken>, DatabaseError>;
    /// Deactivate admin token
    async fn deactivate_token(&self, token_id: &str) -> Result<(), DatabaseError>;
    /// Update admin token last used timestamp
    async fn update_token_last_used(
        &self,
        token_id: &str,
        ip_address: Option<&str>,
    ) -> Result<(), DatabaseError>;
    /// Record admin token usage for audit trail
    async fn record_token_usage(&self, usage: &AdminTokenUsage) -> Result<(), DatabaseError>;
    /// Get admin token usage history
    async fn get_token_usage_history(
        &self,
        token_id: &str,
        start_date: DateTime<Utc>,
        end_date: DateTime<Utc>,
    ) -> Result<Vec<AdminTokenUsage>, DatabaseError>;
    /// Record API key provisioning by admin
    async fn record_provisioned_key(
        &self,
        admin_token_id: &str,
        api_key_id: &str,
        user_email: &str,
        tier: &str,
        rate_limit_requests: u32,
        rate_limit_period: &str,
    ) -> Result<(), DatabaseError>;
    /// Get admin provisioned keys history
    async fn get_provisioned_keys(
        &self,
        admin_token_id: Option<&str>,
        start_date: DateTime<Utc>,
        end_date: DateTime<Utc>,
    ) -> Result<Vec<Value>, DatabaseError>;
}

/// Multi-tenant management repository
#[async_trait]
pub trait TenantRepository: Send + Sync {
    /// Create a new tenant
    async fn create(&self, tenant: &Tenant) -> Result<(), DatabaseError>;
    /// Get tenant by ID
    async fn get_by_id(&self, tenant_id: TenantId) -> Result<Tenant, DatabaseError>;
    /// Get tenant by slug
    async fn get_by_slug(&self, slug: &str) -> Result<Tenant, DatabaseError>;
    /// List tenants for a user
    async fn list_for_user(&self, user_id: Uuid) -> Result<Vec<Tenant>, DatabaseError>;
    /// Store tenant OAuth credentials
    async fn store_oauth_credentials(
        &self,
        credentials: &TenantOAuthCredentials,
    ) -> Result<(), DatabaseError>;
    /// Get tenant OAuth providers
    async fn get_oauth_providers(
        &self,
        tenant_id: TenantId,
    ) -> Result<Vec<TenantOAuthCredentials>, DatabaseError>;
    /// Get tenant OAuth credentials for specific provider
    async fn get_oauth_credentials(
        &self,
        tenant_id: TenantId,
        provider: &str,
    ) -> Result<Option<TenantOAuthCredentials>, DatabaseError>;
    /// Create OAuth application for MCP clients
    async fn create_oauth_app(&self, app: &OAuthApp) -> Result<(), DatabaseError>;
    /// Get OAuth app by client ID
    async fn get_oauth_app_by_client_id(&self, client_id: &str) -> Result<OAuthApp, DatabaseError>;
    /// List OAuth apps for a user
    async fn list_oauth_apps_for_user(&self, user_id: Uuid)
        -> Result<Vec<OAuthApp>, DatabaseError>;
    /// Get all tenants for key rotation check
    async fn get_all(&self) -> Result<Vec<Tenant>, DatabaseError>;
    /// Get user role for a specific tenant
    async fn get_user_role(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> Result<Option<String>, DatabaseError>;
}

/// OAuth 2.0 server repository (RFC 7591)
#[async_trait]
pub trait OAuth2ServerRepository: Send + Sync {
    /// Store OAuth 2.0 client registration
    async fn store_client(&self, client: &OAuth2Client) -> Result<(), DatabaseError>;
    /// Get OAuth 2.0 client by `client_id`
    async fn get_client(&self, client_id: &str) -> Result<Option<OAuth2Client>, DatabaseError>;
    /// Store OAuth 2.0 authorization code
    async fn store_auth_code(&self, auth_code: &OAuth2AuthCode) -> Result<(), DatabaseError>;
    /// Get OAuth 2.0 authorization code
    async fn get_auth_code(&self, code: &str) -> Result<Option<OAuth2AuthCode>, DatabaseError>;
    /// Update OAuth 2.0 authorization code (mark as used)
    async fn update_auth_code(&self, auth_code: &OAuth2AuthCode) -> Result<(), DatabaseError>;
    /// Store OAuth 2.0 refresh token
    async fn store_refresh_token(
        &self,
        refresh_token: &OAuth2RefreshToken,
    ) -> Result<(), DatabaseError>;
    /// Get OAuth 2.0 refresh token
    async fn get_refresh_token(
        &self,
        token: &str,
    ) -> Result<Option<OAuth2RefreshToken>, DatabaseError>;
    /// Revoke OAuth 2.0 refresh token
    async fn revoke_refresh_token(&self, token: &str) -> Result<(), DatabaseError>;
    /// Atomically consume OAuth 2.0 authorization code (check-and-set in single operation)
    async fn consume_auth_code(
        &self,
        code: &str,
        client_id: &str,
        redirect_uri: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<OAuth2AuthCode>, DatabaseError>;
    /// Atomically consume OAuth 2.0 refresh token (check-and-revoke in single operation)
    async fn consume_refresh_token(
        &self,
        token: &str,
        client_id: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<OAuth2RefreshToken>, DatabaseError>;
    /// Look up a refresh token by its value (without `client_id` constraint)
    async fn get_refresh_token_by_value(
        &self,
        token: &str,
    ) -> Result<Option<OAuth2RefreshToken>, DatabaseError>;
    /// Store authorization code
    async fn store_authorization_code(
        &self,
        code: &str,
        client_id: &str,
        redirect_uri: &str,
        scope: &str,
        user_id: Uuid,
    ) -> Result<(), DatabaseError>;
    /// Get authorization code data
    async fn get_authorization_code(&self, code: &str) -> Result<AuthorizationCode, DatabaseError>;
    /// Delete authorization code (after use)
    async fn delete_authorization_code(&self, code: &str) -> Result<(), DatabaseError>;
    /// Store `OAuth2` state for CSRF protection
    async fn store_state(&self, state: &OAuth2State) -> Result<(), DatabaseError>;
    /// Consume `OAuth2` state (atomically check and mark as used)
    async fn consume_state(
        &self,
        state_value: &str,
        client_id: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<OAuth2State>, DatabaseError>;
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
    ) -> Result<(), DatabaseError>;
    /// Load all RSA keypairs from database
    async fn load_rsa_keypairs(
        &self,
    ) -> Result<Vec<(String, String, String, DateTime<Utc>, bool)>, DatabaseError>;
    /// Update active status of RSA keypair
    async fn update_rsa_keypair_active_status(
        &self,
        kid: &str,
        is_active: bool,
    ) -> Result<(), DatabaseError>;
    /// Store key version metadata
    async fn store_key_version(&self, version: &KeyVersion) -> Result<(), DatabaseError>;
    /// Get all key versions for a tenant
    async fn get_key_versions(
        &self,
        tenant_id: Option<TenantId>,
    ) -> Result<Vec<KeyVersion>, DatabaseError>;
    /// Get current active key version for a tenant
    async fn get_current_key_version(
        &self,
        tenant_id: Option<TenantId>,
    ) -> Result<Option<KeyVersion>, DatabaseError>;
    /// Update key version status (activate/deactivate)
    async fn update_key_version_status(
        &self,
        tenant_id: Option<TenantId>,
        version: u32,
        is_active: bool,
    ) -> Result<(), DatabaseError>;
    /// Delete old key versions
    async fn delete_old_key_versions(
        &self,
        tenant_id: Option<TenantId>,
        keep_count: u32,
    ) -> Result<u64, DatabaseError>;
    /// Store audit event
    async fn store_audit_event(&self, event: &AuditEvent) -> Result<(), DatabaseError>;
    /// Get audit events with filters
    async fn get_audit_events(
        &self,
        tenant_id: Option<TenantId>,
        event_type: Option<&str>,
        limit: Option<u32>,
    ) -> Result<Vec<AuditEvent>, DatabaseError>;
    /// Get or create system secret (generates if not exists)
    async fn get_or_create_system_secret(&self, secret_type: &str)
        -> Result<String, DatabaseError>;
    /// Get existing system secret
    async fn get_system_secret(&self, secret_type: &str) -> Result<String, DatabaseError>;
    /// Update system secret (for rotation)
    async fn update_system_secret(
        &self,
        secret_type: &str,
        new_value: &str,
    ) -> Result<(), DatabaseError>;
    /// Encrypt data with AAD (Additional Authenticated Data)
    ///
    /// # Errors
    /// Returns an error if encryption fails (invalid key, nonce generation failure)
    fn encrypt_data_with_aad(&self, data: &str, aad: &str) -> Result<String, DatabaseError>;
    /// Decrypt data with AAD
    ///
    /// # Errors
    /// Returns an error if decryption fails (invalid data, AAD mismatch, tampered data)
    fn decrypt_data_with_aad(&self, encrypted: &str, aad: &str) -> Result<String, DatabaseError>;
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
    ) -> Result<String, DatabaseError>;
    /// Get unread OAuth notifications for a user
    async fn get_unread(&self, user_id: Uuid) -> Result<Vec<OAuthNotification>, DatabaseError>;
    /// Mark OAuth notification as read
    async fn mark_read(&self, notification_id: &str, user_id: Uuid) -> Result<bool, DatabaseError>;
    /// Mark all OAuth notifications as read for a user
    async fn mark_all_read(&self, user_id: Uuid) -> Result<u64, DatabaseError>;
    /// Get all OAuth notifications for a user (read and unread)
    async fn get_all(
        &self,
        user_id: Uuid,
        limit: Option<i64>,
    ) -> Result<Vec<OAuthNotification>, DatabaseError>;
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
    ) -> Result<String, DatabaseError>;
    /// Save user-specific fitness configuration
    async fn save_user_config(
        &self,
        tenant_id: TenantId,
        user_id: &str,
        configuration_name: &str,
        config: &FitnessConfig,
    ) -> Result<String, DatabaseError>;
    /// Get tenant-level fitness configuration
    async fn get_tenant_config(
        &self,
        tenant_id: TenantId,
        configuration_name: &str,
    ) -> Result<Option<FitnessConfig>, DatabaseError>;
    /// Get user-specific fitness configuration
    async fn get_user_config(
        &self,
        tenant_id: TenantId,
        user_id: &str,
        configuration_name: &str,
    ) -> Result<Option<FitnessConfig>, DatabaseError>;
    /// List all tenant-level fitness configuration names
    async fn list_tenant_configurations(
        &self,
        tenant_id: TenantId,
    ) -> Result<Vec<String>, DatabaseError>;
    /// List all user-specific fitness configuration names
    async fn list_user_configurations(
        &self,
        tenant_id: TenantId,
        user_id: &str,
    ) -> Result<Vec<String>, DatabaseError>;
    /// Delete fitness configuration (tenant or user-specific)
    async fn delete_config(
        &self,
        tenant_id: TenantId,
        user_id: Option<&str>,
        configuration_name: &str,
    ) -> Result<bool, DatabaseError>;
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
    ) -> Result<ConversationRecord, DatabaseError>;
    /// Get a conversation by ID with user/tenant isolation
    async fn get_conversation(
        &self,
        conversation_id: &str,
        user_id: &str,
        tenant_id: TenantId,
    ) -> Result<Option<ConversationRecord>, DatabaseError>;
    /// List conversations for a user with pagination
    async fn list_conversations(
        &self,
        user_id: &str,
        tenant_id: TenantId,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<ConversationSummary>, DatabaseError>;
    /// Update conversation title
    async fn update_conversation_title(
        &self,
        conversation_id: &str,
        user_id: &str,
        tenant_id: TenantId,
        title: &str,
    ) -> Result<bool, DatabaseError>;
    /// Delete a conversation and its messages
    async fn delete_conversation(
        &self,
        conversation_id: &str,
        user_id: &str,
        tenant_id: TenantId,
    ) -> Result<bool, DatabaseError>;
    /// Add a message to a conversation (verifies user owns the conversation)
    async fn add_message(
        &self,
        conversation_id: &str,
        user_id: &str,
        role: &str,
        content: &str,
        token_count: Option<u32>,
        finish_reason: Option<&str>,
    ) -> Result<MessageRecord, DatabaseError>;
    /// Get all messages for a conversation (verifies user owns the conversation)
    async fn get_messages(
        &self,
        conversation_id: &str,
        user_id: &str,
    ) -> Result<Vec<MessageRecord>, DatabaseError>;
    /// Get recent messages for a conversation (verifies user owns the conversation)
    async fn get_recent_messages(
        &self,
        conversation_id: &str,
        user_id: &str,
        limit: i64,
    ) -> Result<Vec<MessageRecord>, DatabaseError>;
    /// Get message count for a conversation (verifies user owns the conversation)
    async fn get_message_count(
        &self,
        conversation_id: &str,
        user_id: &str,
    ) -> Result<i64, DatabaseError>;
    /// Delete all conversations for a user
    async fn delete_all_user_conversations(
        &self,
        user_id: &str,
        tenant_id: TenantId,
    ) -> Result<i64, DatabaseError>;
}

/// User MCP token management repository
#[async_trait]
pub trait UserMcpTokenRepository: Send + Sync {
    /// Create a new user MCP token for AI client authentication
    async fn create_token(
        &self,
        user_id: Uuid,
        request: &CreateUserMcpTokenRequest,
    ) -> Result<UserMcpTokenCreated, DatabaseError>;
    /// Validate a user MCP token and return the associated user ID
    async fn validate_token(&self, token_value: &str) -> Result<Uuid, DatabaseError>;
    /// List all MCP tokens for a user
    async fn list_tokens(&self, user_id: Uuid) -> Result<Vec<UserMcpTokenInfo>, DatabaseError>;
    /// Revoke a user MCP token
    async fn revoke_token(&self, token_id: &str, user_id: Uuid) -> Result<(), DatabaseError>;
    /// Get a user MCP token by ID
    async fn get_token(
        &self,
        token_id: &str,
        user_id: Uuid,
    ) -> Result<Option<UserMcpToken>, DatabaseError>;
    /// Cleanup expired user MCP tokens (mark as revoked)
    async fn cleanup_expired_tokens(&self) -> Result<u64, DatabaseError>;
}

/// Impersonation session management repository
#[async_trait]
pub trait ImpersonationRepository: Send + Sync {
    /// Create a new impersonation session for audit trail
    async fn create_session(&self, session: &ImpersonationSession) -> Result<(), DatabaseError>;
    /// Get impersonation session by ID
    async fn get_session(
        &self,
        session_id: &str,
    ) -> Result<Option<ImpersonationSession>, DatabaseError>;
    /// Get active impersonation session where user is impersonator or target
    async fn get_active_session(
        &self,
        user_id: Uuid,
    ) -> Result<Option<ImpersonationSession>, DatabaseError>;
    /// End an impersonation session
    async fn end_session(&self, session_id: &str) -> Result<(), DatabaseError>;
    /// End all active impersonation sessions for an impersonator
    async fn end_all_sessions(&self, impersonator_id: Uuid) -> Result<u64, DatabaseError>;
    /// List impersonation sessions with optional filters
    async fn list_sessions(
        &self,
        impersonator_id: Option<Uuid>,
        target_user_id: Option<Uuid>,
        active_only: bool,
        limit: u32,
    ) -> Result<Vec<ImpersonationSession>, DatabaseError>;
}

/// LLM credential management repository
#[async_trait]
pub trait LlmCredentialRepository: Send + Sync {
    /// Store LLM credentials (user-specific or tenant-level)
    async fn store_credentials(&self, record: &LlmCredentialRecord) -> Result<(), DatabaseError>;
    /// Get LLM credentials for a specific provider
    async fn get_credentials(
        &self,
        tenant_id: TenantId,
        user_id: Option<Uuid>,
        provider: &str,
    ) -> Result<Option<LlmCredentialRecord>, DatabaseError>;
    /// List all LLM credentials for a tenant (for admin UI)
    async fn list_credentials(
        &self,
        tenant_id: TenantId,
    ) -> Result<Vec<LlmCredentialSummary>, DatabaseError>;
    /// Delete LLM credentials
    async fn delete_credentials(
        &self,
        tenant_id: TenantId,
        user_id: Option<Uuid>,
        provider: &str,
    ) -> Result<bool, DatabaseError>;
    /// Get admin config override value by key (for system-wide LLM API keys)
    async fn get_admin_config_override(
        &self,
        config_key: &str,
        tenant_id: Option<TenantId>,
    ) -> Result<Option<String>, DatabaseError>;
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
    ) -> Result<(), DatabaseError>;
    /// Remove a provider connection
    async fn remove_connection(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) -> Result<(), DatabaseError>;
    /// Get all provider connections for a user
    async fn get_for_user(
        &self,
        user_id: Uuid,
        tenant_id: Option<TenantId>,
    ) -> Result<Vec<ProviderConnection>, DatabaseError>;
    /// Check if a specific provider is connected for a user (cross-tenant)
    async fn is_connected(&self, user_id: Uuid, provider: &str) -> Result<bool, DatabaseError>;
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
    ) -> Result<Uuid, DatabaseError>;
    /// Consume a password reset token by its hash
    async fn consume_token(&self, token_hash: &str) -> Result<Uuid, DatabaseError>;
    /// Invalidate all unused reset tokens for a user
    async fn invalidate_tokens(&self, user_id: Uuid) -> Result<(), DatabaseError>;
}

/// OAuth client-side state management repository
#[async_trait]
pub trait OAuthClientStateRepository: Send + Sync {
    /// Store OAuth client-side state for CSRF protection and PKCE verifier storage
    async fn store_oauth_client_state(&self, state: &OAuthClientState)
        -> Result<(), DatabaseError>;
    /// Consume OAuth client state atomically (verify and mark as used)
    async fn consume_oauth_client_state(
        &self,
        state_value: &str,
        provider: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<OAuthClientState>, DatabaseError>;
}

/// Tool selection and per-tenant configuration repository
#[async_trait]
pub trait ToolSelectionRepository: Send + Sync {
    /// Get the complete tool catalog
    async fn get_tool_catalog(&self) -> Result<Vec<ToolCatalogEntry>, DatabaseError>;
    /// Get a specific tool catalog entry by name
    async fn get_tool_catalog_entry(
        &self,
        tool_name: &str,
    ) -> Result<Option<ToolCatalogEntry>, DatabaseError>;
    /// Get tools filtered by category
    async fn get_tools_by_category(
        &self,
        category: ToolCategory,
    ) -> Result<Vec<ToolCatalogEntry>, DatabaseError>;
    /// Get tools available for a specific plan level
    async fn get_tools_by_min_plan(
        &self,
        plan: TenantPlan,
    ) -> Result<Vec<ToolCatalogEntry>, DatabaseError>;
    /// Get all tool overrides for a tenant
    async fn get_overrides(
        &self,
        tenant_id: TenantId,
    ) -> Result<Vec<TenantToolOverride>, DatabaseError>;
    /// Get a specific tool override for a tenant
    async fn get_override(
        &self,
        tenant_id: TenantId,
        tool_name: &str,
    ) -> Result<Option<TenantToolOverride>, DatabaseError>;
    /// Create or update a tool override for a tenant
    async fn upsert_override(
        &self,
        tenant_id: TenantId,
        tool_name: &str,
        is_enabled: bool,
        enabled_by_user_id: Option<Uuid>,
        reason: Option<String>,
    ) -> Result<TenantToolOverride, DatabaseError>;
    /// Delete a tool override (revert to catalog default)
    async fn delete_override(
        &self,
        tenant_id: TenantId,
        tool_name: &str,
    ) -> Result<bool, DatabaseError>;
    /// Count enabled tools for a tenant
    async fn count_enabled_tools(&self, tenant_id: TenantId) -> Result<usize, DatabaseError>;
}

// ================================
// Traits WITHOUT DatabaseProvider blanket impls
// These repositories have custom implementations not backed by the god-trait.
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
    ) -> Result<String, DatabaseError>;
    /// Get recipe by ID
    async fn get_by_id(
        &self,
        recipe_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> Result<Option<Recipe>, DatabaseError>;
    /// List recipes with optional filtering
    async fn list(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        meal_timing: Option<MealTiming>,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> Result<Vec<Recipe>, DatabaseError>;
    /// Update an existing recipe
    async fn update(
        &self,
        recipe_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
        recipe: &Recipe,
    ) -> Result<bool, DatabaseError>;
    /// Delete a recipe
    async fn delete(
        &self,
        recipe_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> Result<bool, DatabaseError>;
    /// Update cached nutrition data for a recipe
    async fn update_nutrition_cache(
        &self,
        recipe_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
        nutrition: &ValidatedNutrition,
    ) -> Result<bool, DatabaseError>;
    /// Search recipes by text query
    async fn search(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        query: &str,
        limit: Option<u32>,
    ) -> Result<Vec<Recipe>, DatabaseError>;
    /// Count recipes
    async fn count(&self, user_id: Uuid, tenant_id: TenantId) -> Result<u32, DatabaseError>;
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
    ) -> Result<Coach, DatabaseError>;
    /// Get coach by ID
    async fn get_by_id(
        &self,
        coach_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> Result<Option<Coach>, DatabaseError>;
    /// List coachs with optional filtering
    async fn list(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        filter: &ListCoachesFilter,
    ) -> Result<Vec<Coach>, DatabaseError>;
    /// Update an existing coach
    async fn update(
        &self,
        coach_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
        request: &UpdateCoachRequest,
    ) -> Result<Option<Coach>, DatabaseError>;
    /// Delete a coach
    async fn delete(
        &self,
        coach_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> Result<bool, DatabaseError>;
    /// Record a usage event for a coach interaction
    async fn record_usage(
        &self,
        coach_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> Result<bool, DatabaseError>;
    /// Toggle favorite status for a coach
    async fn toggle_favorite(
        &self,
        coach_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> Result<Option<bool>, DatabaseError>;
    /// Search coachs by text query
    async fn search(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        query: &str,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> Result<Vec<Coach>, DatabaseError>;
    /// Count coachs
    async fn count(&self, user_id: Uuid, tenant_id: TenantId) -> Result<u32, DatabaseError>;
}

/// Mobility (stretching exercises and yoga poses) read-only repository
#[async_trait]
pub trait MobilityRepository: Send + Sync {
    /// Get a stretching exercise by ID
    async fn get_stretching_exercise(
        &self,
        id: &str,
    ) -> Result<Option<StretchingExercise>, DatabaseError>;
    /// List stretching exercises with optional filtering
    async fn list_stretching_exercises(
        &self,
        filter: &ListStretchingFilter,
    ) -> Result<Vec<StretchingExercise>, DatabaseError>;
    /// Search stretching exercises by text query
    async fn search_stretching_exercises(
        &self,
        query: &str,
        limit: Option<u32>,
    ) -> Result<Vec<StretchingExercise>, DatabaseError>;
    /// Get stretches recommended for a specific activity type
    async fn get_stretches_for_activity(
        &self,
        activity_type: &str,
        limit: Option<u32>,
    ) -> Result<Vec<StretchingExercise>, DatabaseError>;
    /// Get a yoga pose by ID
    async fn get_yoga_pose(&self, id: &str) -> Result<Option<YogaPose>, DatabaseError>;
    /// List yoga poses with optional filtering
    async fn list_yoga_poses(
        &self,
        filter: &ListYogaFilter,
    ) -> Result<Vec<YogaPose>, DatabaseError>;
    /// Search yoga poses by text query
    async fn search_yoga_poses(
        &self,
        query: &str,
        limit: Option<u32>,
    ) -> Result<Vec<YogaPose>, DatabaseError>;
    /// Get yoga poses recommended for a recovery context
    async fn get_poses_for_recovery(
        &self,
        recovery_context: &str,
        limit: Option<u32>,
    ) -> Result<Vec<YogaPose>, DatabaseError>;
    /// Get muscle mapping for a specific activity type
    async fn get_activity_muscle_mapping(
        &self,
        activity_type: &str,
    ) -> Result<Option<ActivityMuscleMapping>, DatabaseError>;
    /// List all activity-to-muscle mappings
    async fn list_activity_muscle_mappings(
        &self,
    ) -> Result<Vec<ActivityMuscleMapping>, DatabaseError>;
}

/// Social features repository for friend connections and shared insights
#[async_trait]
pub trait SocialRepository: Send + Sync {
    /// Create a new friend connection request
    async fn create_friend_connection(
        &self,
        connection: &FriendConnection,
    ) -> Result<Uuid, DatabaseError>;
    /// Get a friend connection by ID
    async fn get_friend_connection(
        &self,
        id: Uuid,
    ) -> Result<Option<FriendConnection>, DatabaseError>;
    /// Get the friend connection between two users (if any)
    async fn get_friend_connection_between(
        &self,
        user_a: Uuid,
        user_b: Uuid,
    ) -> Result<Option<FriendConnection>, DatabaseError>;
    /// Update friend connection status (accept, reject, block)
    async fn update_friend_connection_status(
        &self,
        id: Uuid,
        user_id: Uuid,
        status: FriendStatus,
    ) -> Result<(), DatabaseError>;
    /// Get all accepted friends for a user
    async fn get_friends(&self, user_id: Uuid) -> Result<Vec<FriendConnection>, DatabaseError>;
    /// Get pending incoming friend requests
    async fn get_pending_friend_requests(
        &self,
        user_id: Uuid,
    ) -> Result<Vec<FriendConnection>, DatabaseError>;
    /// Get outgoing friend requests sent by the user
    async fn get_sent_friend_requests(
        &self,
        user_id: Uuid,
    ) -> Result<Vec<FriendConnection>, DatabaseError>;
    /// Check whether two users are friends
    async fn are_friends(&self, user_a: Uuid, user_b: Uuid) -> Result<bool, DatabaseError>;
    /// Delete a friend connection
    async fn delete_friend_connection(
        &self,
        id: Uuid,
        user_id: Uuid,
    ) -> Result<bool, DatabaseError>;
    /// Get social settings for a user, creating defaults if not found
    async fn get_or_create_social_settings(
        &self,
        user_id: Uuid,
    ) -> Result<UserSocialSettings, DatabaseError>;
    /// Get social settings for a user (returns None if not set)
    async fn get_social_settings(
        &self,
        user_id: Uuid,
    ) -> Result<Option<UserSocialSettings>, DatabaseError>;
    /// Create or update social settings for a user
    async fn upsert_social_settings(
        &self,
        settings: &UserSocialSettings,
    ) -> Result<(), DatabaseError>;
    /// Share an insight with friends
    async fn create_shared_insight(&self, insight: &SharedInsight) -> Result<Uuid, DatabaseError>;
    /// Get a shared insight by ID, scoped to user
    async fn get_shared_insight(
        &self,
        id: Uuid,
        user_id: Uuid,
    ) -> Result<Option<SharedInsight>, DatabaseError>;
    /// Get the friends feed (shared insights from friends)
    async fn get_friends_feed(
        &self,
        user_id: Uuid,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<SharedInsight>, DatabaseError>;
    /// Get insights shared by a specific user
    async fn get_user_shared_insights(
        &self,
        user_id: Uuid,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<SharedInsight>, DatabaseError>;
    /// Delete a shared insight
    async fn delete_shared_insight(&self, id: Uuid, user_id: Uuid) -> Result<bool, DatabaseError>;
    /// Create or update a reaction to an insight
    async fn upsert_insight_reaction(
        &self,
        reaction: &InsightReaction,
    ) -> Result<(), DatabaseError>;
    /// Get a specific user's reaction to an insight
    async fn get_insight_reaction(
        &self,
        insight_id: Uuid,
        user_id: Uuid,
    ) -> Result<Option<InsightReaction>, DatabaseError>;
    /// Delete a reaction to an insight
    async fn delete_insight_reaction(
        &self,
        insight_id: Uuid,
        user_id: Uuid,
    ) -> Result<bool, DatabaseError>;
    /// Get all reactions to an insight
    async fn get_insight_reactions(
        &self,
        insight_id: Uuid,
    ) -> Result<Vec<InsightReaction>, DatabaseError>;
    /// Create an adapted version of a shared insight
    async fn create_adapted_insight(&self, insight: &AdaptedInsight)
        -> Result<Uuid, DatabaseError>;
    /// Get an adapted insight by ID
    async fn get_adapted_insight(&self, id: Uuid) -> Result<Option<AdaptedInsight>, DatabaseError>;
    /// Get a user's adaptation of a specific source insight
    async fn get_user_adaptation(
        &self,
        source_insight_id: Uuid,
        user_id: Uuid,
    ) -> Result<Option<AdaptedInsight>, DatabaseError>;
    /// Get all adapted insights for a user
    async fn get_user_adapted_insights(
        &self,
        user_id: Uuid,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<AdaptedInsight>, DatabaseError>;
    /// Update whether an adapted insight was helpful
    async fn update_adapted_insight_helpful(
        &self,
        id: Uuid,
        user_id: Uuid,
        was_helpful: bool,
    ) -> Result<bool, DatabaseError>;
    /// Search for discoverable users by query
    async fn search_discoverable_users(
        &self,
        query: &str,
        exclude_user_id: Uuid,
        limit: u32,
    ) -> Result<Vec<(Uuid, String, Option<String>)>, DatabaseError>;
    /// Get total friend count for a user
    async fn get_friend_count(&self, user_id: Uuid) -> Result<i64, DatabaseError>;
}

/// Blanket implementations for all repository traits via `DatabaseProvider`
mod blanket_impls;
