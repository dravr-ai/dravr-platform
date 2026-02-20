// ABOUTME: Blanket implementations of repository traits delegating to domain-specific provider traits
// ABOUTME: Each impl maps the focused repository trait methods to the corresponding domain trait methods
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::provider::{
    A2ADbOps, AdminDbOps, ApiKeyDbOps, ChatDbOps, OAuthDbOps, SecurityDbOps, SocialDbOps,
    TenantDbOps, UsageDbOps, UserDbOps,
};
use crate::repositories::{
    A2ARepository, AdminRepository, ApiKeyRepository, ChatRepository, FitnessConfigRepository,
    ImpersonationRepository, InsightRepository, LlmCredentialRepository, LlmUsageRepository,
    NotificationRepository, OAuth2ServerRepository, OAuthClientStateRepository,
    OAuthTokenRepository, PasswordResetRepository, ProfileRepository, ProviderConnectionRepository,
    SecurityRepository, TenantRepository, ToolSelectionRepository, UsageCounterRepository,
    UsageRepository, UserMcpTokenRepository, UserRepository,
};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::admin::jwt::JwtSigner;
use pierre_core::admin::models::{
    AdminToken, AdminTokenUsage, CreateAdminTokenRequest, GeneratedAdminToken,
};
use pierre_core::config::FitnessConfig;
use pierre_core::errors::database::DatabaseError;
use pierre_core::errors::{AppError, ErrorCode};
use pierre_core::models::a2a::{
    A2AClient, A2ASession, A2ATask, A2AUsage, A2AUsageStats, TaskStatus,
};
use pierre_core::models::usage::{InsertLlmUsage, LlmUsageAggregateRow, LlmUsageDailyRow};
use pierre_core::models::{
    AddMessageParams, ApiKey, ApiKeyUsage, ApiKeyUsageStats, AuditEvent, AuthorizationCode,
    ConnectionType, ConversationRecord, ConversationSummary, CreateUserMcpTokenRequest, JwtUsage,
    KeyVersion, LlmCredentialRecord, LlmCredentialSummary, LlmUsageRecord, MessageRecord,
    OAuth2AuthCode, OAuth2Client, OAuth2RefreshToken, OAuth2State, OAuthApp, OAuthClientState,
    OAuthNotification, ProviderConnection, RequestLog, Tenant, TenantId, TenantOAuthCredentials,
    TenantPlan, TenantToolOverride, ToolCatalogEntry, ToolCategory, ToolUsage, UsageCounterRecord,
    User, UserMcpToken, UserMcpTokenCreated, UserMcpTokenInfo, UserOAuthApp, UserOAuthToken,
    UserStatus,
};
use pierre_core::pagination::{CursorPage, PaginationParams};
use pierre_core::permissions::impersonation::ImpersonationSession;
use serde_json::Value;
use uuid::Uuid;

/// Convert `AppError` to `DatabaseError` preserving not-found semantics.
/// `DatabaseProvider` methods return `AppResult<T>` (using `AppError`), but repository
/// traits return `Result<T, DatabaseError>`. This helper maps `AppError::not_found`
/// to `DatabaseError::NotFound` so 404 status codes propagate correctly through
/// the `From<DatabaseError> for AppError` conversion in route handlers.
fn app_error_to_db(e: AppError) -> DatabaseError {
    if e.code == ErrorCode::ResourceNotFound {
        DatabaseError::NotFound {
            entity_type: "resource",
            entity_id: e.message,
        }
    } else {
        DatabaseError::QueryError {
            context: e.to_string(),
        }
    }
}

#[async_trait]
impl<T: UserDbOps> UserRepository for T {
    async fn create(&self, user: &User) -> Result<Uuid, DatabaseError> {
        UserDbOps::create_user(self, user)
            .await
            .map_err(app_error_to_db)
    }
    async fn get(&self, user_id: Uuid, tenant_id: TenantId) -> Result<Option<User>, DatabaseError> {
        UserDbOps::get_user(self, user_id, tenant_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_global(&self, user_id: Uuid) -> Result<Option<User>, DatabaseError> {
        UserDbOps::get_user_global(self, user_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_by_email(&self, email: &str) -> Result<Option<User>, DatabaseError> {
        UserDbOps::get_user_by_email(self, email)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_by_email_required(&self, email: &str) -> Result<User, DatabaseError> {
        UserDbOps::get_user_by_email_required(self, email)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_by_firebase_uid(&self, firebase_uid: &str) -> Result<Option<User>, DatabaseError> {
        UserDbOps::get_user_by_firebase_uid(self, firebase_uid)
            .await
            .map_err(app_error_to_db)
    }
    async fn update_last_active(&self, user_id: Uuid) -> Result<(), DatabaseError> {
        UserDbOps::update_last_active(self, user_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn count(&self) -> Result<i64, DatabaseError> {
        UserDbOps::get_user_count(self)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_by_status(
        &self,
        status: &str,
        tenant_id: Option<TenantId>,
    ) -> Result<Vec<User>, DatabaseError> {
        UserDbOps::get_users_by_status(self, status, tenant_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_by_status_cursor(
        &self,
        status: &str,
        params: &PaginationParams,
    ) -> Result<CursorPage<User>, DatabaseError> {
        UserDbOps::get_users_by_status_cursor(self, status, params)
            .await
            .map_err(app_error_to_db)
    }
    async fn update_status(
        &self,
        user_id: Uuid,
        new_status: UserStatus,
        approved_by: Option<Uuid>,
    ) -> Result<User, DatabaseError> {
        UserDbOps::update_user_status(self, user_id, new_status, approved_by)
            .await
            .map_err(app_error_to_db)
    }
    async fn update_tenant_id(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> Result<(), DatabaseError> {
        UserDbOps::update_user_tenant_id(self, user_id, tenant_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn update_password(
        &self,
        user_id: Uuid,
        password_hash: &str,
    ) -> Result<(), DatabaseError> {
        UserDbOps::update_user_password(self, user_id, password_hash)
            .await
            .map_err(app_error_to_db)
    }
    async fn update_display_name(
        &self,
        user_id: Uuid,
        display_name: &str,
    ) -> Result<User, DatabaseError> {
        UserDbOps::update_user_display_name(self, user_id, display_name)
            .await
            .map_err(app_error_to_db)
    }
    async fn delete(&self, user_id: Uuid) -> Result<(), DatabaseError> {
        UserDbOps::delete_user(self, user_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_first_admin_user(&self) -> Result<Option<User>, DatabaseError> {
        UserDbOps::get_first_admin_user(self)
            .await
            .map_err(app_error_to_db)
    }
    async fn has_synthetic_activities(&self, user_id: Uuid) -> Result<bool, DatabaseError> {
        UserDbOps::user_has_synthetic_activities(self, user_id)
            .await
            .map_err(app_error_to_db)
    }
}

#[async_trait]
impl<T: OAuthDbOps> OAuthTokenRepository for T {
    async fn upsert_token(&self, token: &UserOAuthToken) -> Result<(), DatabaseError> {
        OAuthDbOps::upsert_user_oauth_token(self, token)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_token(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) -> Result<Option<UserOAuthToken>, DatabaseError> {
        OAuthDbOps::get_user_oauth_token(self, user_id, tenant_id, provider)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_tokens(
        &self,
        user_id: Uuid,
        tenant_id: Option<TenantId>,
    ) -> Result<Vec<UserOAuthToken>, DatabaseError> {
        OAuthDbOps::get_user_oauth_tokens(self, user_id, tenant_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_tenant_provider_tokens(
        &self,
        tenant_id: TenantId,
        provider: &str,
    ) -> Result<Vec<UserOAuthToken>, DatabaseError> {
        OAuthDbOps::get_tenant_provider_tokens(self, tenant_id, provider)
            .await
            .map_err(app_error_to_db)
    }
    async fn delete_token(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) -> Result<(), DatabaseError> {
        OAuthDbOps::delete_user_oauth_token(self, user_id, tenant_id, provider)
            .await
            .map_err(app_error_to_db)
    }
    async fn delete_tokens(&self, user_id: Uuid, tenant_id: TenantId) -> Result<(), DatabaseError> {
        OAuthDbOps::delete_user_oauth_tokens(self, user_id, tenant_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn refresh_token(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        access_token: &str,
        refresh_token: Option<&str>,
        expires_at: Option<DateTime<Utc>>,
    ) -> Result<(), DatabaseError> {
        OAuthDbOps::refresh_user_oauth_token(
            self,
            user_id,
            tenant_id,
            provider,
            access_token,
            refresh_token,
            expires_at,
        )
        .await
        .map_err(app_error_to_db)
    }
    async fn store_user_oauth_app(
        &self,
        user_id: Uuid,
        provider: &str,
        client_id: &str,
        client_secret: &str,
        redirect_uri: &str,
    ) -> Result<(), DatabaseError> {
        OAuthDbOps::store_user_oauth_app(
            self,
            user_id,
            provider,
            client_id,
            client_secret,
            redirect_uri,
        )
        .await
        .map_err(app_error_to_db)
    }
    async fn get_user_oauth_app(
        &self,
        user_id: Uuid,
        provider: &str,
    ) -> Result<Option<UserOAuthApp>, DatabaseError> {
        OAuthDbOps::get_user_oauth_app(self, user_id, provider)
            .await
            .map_err(app_error_to_db)
    }
    async fn list_user_oauth_apps(
        &self,
        user_id: Uuid,
    ) -> Result<Vec<UserOAuthApp>, DatabaseError> {
        OAuthDbOps::list_user_oauth_apps(self, user_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn remove_user_oauth_app(
        &self,
        user_id: Uuid,
        provider: &str,
    ) -> Result<(), DatabaseError> {
        OAuthDbOps::remove_user_oauth_app(self, user_id, provider)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_provider_last_sync(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) -> Result<Option<DateTime<Utc>>, DatabaseError> {
        OAuthDbOps::get_provider_last_sync(self, user_id, tenant_id, provider)
            .await
            .map_err(app_error_to_db)
    }
    async fn update_provider_last_sync(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        sync_time: DateTime<Utc>,
    ) -> Result<(), DatabaseError> {
        OAuthDbOps::update_provider_last_sync(self, user_id, tenant_id, provider, sync_time)
            .await
            .map_err(app_error_to_db)
    }
}

#[async_trait]
impl<T: ApiKeyDbOps> ApiKeyRepository for T {
    async fn create(&self, api_key: &ApiKey) -> Result<(), DatabaseError> {
        ApiKeyDbOps::create_api_key(self, api_key)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_by_prefix(
        &self,
        prefix: &str,
        hash: &str,
    ) -> Result<Option<ApiKey>, DatabaseError> {
        ApiKeyDbOps::get_api_key_by_prefix(self, prefix, hash)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_for_user(&self, user_id: Uuid) -> Result<Vec<ApiKey>, DatabaseError> {
        ApiKeyDbOps::get_user_api_keys(self, user_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn update_last_used(&self, api_key_id: &str) -> Result<(), DatabaseError> {
        ApiKeyDbOps::update_api_key_last_used(self, api_key_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn deactivate(&self, api_key_id: &str, user_id: Uuid) -> Result<(), DatabaseError> {
        ApiKeyDbOps::deactivate_api_key(self, api_key_id, user_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_by_id(
        &self,
        api_key_id: &str,
        user_id: Option<Uuid>,
    ) -> Result<Option<ApiKey>, DatabaseError> {
        ApiKeyDbOps::get_api_key_by_id(self, api_key_id, user_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_filtered(
        &self,
        user_email: Option<&str>,
        active_only: bool,
        limit: Option<i32>,
        offset: Option<i32>,
    ) -> Result<Vec<ApiKey>, DatabaseError> {
        ApiKeyDbOps::get_api_keys_filtered(self, user_email, active_only, limit, offset)
            .await
            .map_err(app_error_to_db)
    }
    async fn cleanup_expired(&self) -> Result<u64, DatabaseError> {
        ApiKeyDbOps::cleanup_expired_api_keys(self)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_expired(&self) -> Result<Vec<ApiKey>, DatabaseError> {
        ApiKeyDbOps::get_expired_api_keys(self)
            .await
            .map_err(app_error_to_db)
    }
}

#[async_trait]
impl<T: UsageDbOps> UsageRepository for T {
    async fn record_api_key(&self, usage: &ApiKeyUsage) -> Result<(), DatabaseError> {
        UsageDbOps::record_api_key_usage(self, usage)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_api_key_current(&self, api_key_id: &str) -> Result<u32, DatabaseError> {
        UsageDbOps::get_api_key_current_usage(self, api_key_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_api_key_stats(
        &self,
        api_key_id: &str,
        start_date: DateTime<Utc>,
        end_date: DateTime<Utc>,
    ) -> Result<ApiKeyUsageStats, DatabaseError> {
        UsageDbOps::get_api_key_usage_stats(self, api_key_id, start_date, end_date)
            .await
            .map_err(app_error_to_db)
    }
    async fn record_jwt_usage(&self, usage: &JwtUsage) -> Result<(), DatabaseError> {
        UsageDbOps::record_jwt_usage(self, usage)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_jwt_current_usage(&self, user_id: Uuid) -> Result<u32, DatabaseError> {
        UsageDbOps::get_jwt_current_usage(self, user_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_request_logs(
        &self,
        user_id: Option<Uuid>,
        api_key_id: Option<&str>,
        start_time: Option<DateTime<Utc>>,
        end_time: Option<DateTime<Utc>>,
        status_filter: Option<&str>,
        tool_filter: Option<&str>,
    ) -> Result<Vec<RequestLog>, DatabaseError> {
        UsageDbOps::get_request_logs(
            self,
            user_id,
            api_key_id,
            start_time,
            end_time,
            status_filter,
            tool_filter,
        )
        .await
        .map_err(app_error_to_db)
    }
    async fn get_system_stats(
        &self,
        tenant_id: Option<TenantId>,
    ) -> Result<(u64, u64), DatabaseError> {
        UsageDbOps::get_system_stats(self, tenant_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_top_tools_analysis(
        &self,
        user_id: Uuid,
        start_time: DateTime<Utc>,
        end_time: DateTime<Utc>,
    ) -> Result<Vec<ToolUsage>, DatabaseError> {
        UsageDbOps::get_top_tools_analysis(self, user_id, start_time, end_time)
            .await
            .map_err(app_error_to_db)
    }
}

#[async_trait]
impl<T: A2ADbOps> A2ARepository for T {
    async fn create_client(
        &self,
        client: &A2AClient,
        client_secret: &str,
        api_key_id: &str,
    ) -> Result<String, DatabaseError> {
        A2ADbOps::create_a2a_client(self, client, client_secret, api_key_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_client(&self, client_id: &str) -> Result<Option<A2AClient>, DatabaseError> {
        A2ADbOps::get_a2a_client(self, client_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_client_by_api_key_id(
        &self,
        api_key_id: &str,
    ) -> Result<Option<A2AClient>, DatabaseError> {
        A2ADbOps::get_a2a_client_by_api_key_id(self, api_key_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_client_by_name(&self, name: &str) -> Result<Option<A2AClient>, DatabaseError> {
        A2ADbOps::get_a2a_client_by_name(self, name)
            .await
            .map_err(app_error_to_db)
    }
    async fn list_clients(&self, user_id: &Uuid) -> Result<Vec<A2AClient>, DatabaseError> {
        A2ADbOps::list_a2a_clients(self, user_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn deactivate_client(&self, client_id: &str) -> Result<(), DatabaseError> {
        A2ADbOps::deactivate_a2a_client(self, client_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_client_credentials(
        &self,
        client_id: &str,
    ) -> Result<Option<(String, String)>, DatabaseError> {
        A2ADbOps::get_a2a_client_credentials(self, client_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn invalidate_client_sessions(&self, client_id: &str) -> Result<(), DatabaseError> {
        A2ADbOps::invalidate_a2a_client_sessions(self, client_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn deactivate_client_api_keys(&self, client_id: &str) -> Result<(), DatabaseError> {
        A2ADbOps::deactivate_client_api_keys(self, client_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn create_session(
        &self,
        client_id: &str,
        user_id: Option<&Uuid>,
        granted_scopes: &[String],
        expires_in_hours: i64,
    ) -> Result<String, DatabaseError> {
        A2ADbOps::create_a2a_session(self, client_id, user_id, granted_scopes, expires_in_hours)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_session(&self, session_token: &str) -> Result<Option<A2ASession>, DatabaseError> {
        A2ADbOps::get_a2a_session(self, session_token)
            .await
            .map_err(app_error_to_db)
    }
    async fn update_session_activity(&self, session_token: &str) -> Result<(), DatabaseError> {
        A2ADbOps::update_a2a_session_activity(self, session_token)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_active_sessions(&self, client_id: &str) -> Result<Vec<A2ASession>, DatabaseError> {
        A2ADbOps::get_active_a2a_sessions(self, client_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn create_task(
        &self,
        client_id: &str,
        session_id: Option<&str>,
        task_type: &str,
        input_data: &Value,
    ) -> Result<String, DatabaseError> {
        A2ADbOps::create_a2a_task(self, client_id, session_id, task_type, input_data)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_task(&self, task_id: &str) -> Result<Option<A2ATask>, DatabaseError> {
        A2ADbOps::get_a2a_task(self, task_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn list_tasks(
        &self,
        client_id: Option<&str>,
        status_filter: Option<&TaskStatus>,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> Result<Vec<A2ATask>, DatabaseError> {
        A2ADbOps::list_a2a_tasks(self, client_id, status_filter, limit, offset)
            .await
            .map_err(app_error_to_db)
    }
    async fn update_task_status(
        &self,
        task_id: &str,
        status: &TaskStatus,
        result: Option<&Value>,
        error: Option<&str>,
    ) -> Result<(), DatabaseError> {
        A2ADbOps::update_a2a_task_status(self, task_id, status, result, error)
            .await
            .map_err(app_error_to_db)
    }
    async fn record_usage(&self, usage: &A2AUsage) -> Result<(), DatabaseError> {
        A2ADbOps::record_a2a_usage(self, usage)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_client_current_usage(&self, client_id: &str) -> Result<u32, DatabaseError> {
        A2ADbOps::get_a2a_client_current_usage(self, client_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_usage_stats(
        &self,
        client_id: &str,
        start_date: DateTime<Utc>,
        end_date: DateTime<Utc>,
    ) -> Result<A2AUsageStats, DatabaseError> {
        A2ADbOps::get_a2a_usage_stats(self, client_id, start_date, end_date)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_client_usage_history(
        &self,
        client_id: &str,
        days: u32,
    ) -> Result<Vec<(DateTime<Utc>, u32, u32)>, DatabaseError> {
        A2ADbOps::get_a2a_client_usage_history(self, client_id, days)
            .await
            .map_err(app_error_to_db)
    }
}

#[async_trait]
impl<T: UserDbOps> ProfileRepository for T {
    async fn upsert_profile(
        &self,
        user_id: Uuid,
        profile_data: Value,
    ) -> Result<(), DatabaseError> {
        UserDbOps::upsert_user_profile(self, user_id, profile_data)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_profile(&self, user_id: Uuid) -> Result<Option<Value>, DatabaseError> {
        UserDbOps::get_user_profile(self, user_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn create_goal(&self, user_id: Uuid, goal_data: Value) -> Result<String, DatabaseError> {
        UserDbOps::create_goal(self, user_id, goal_data)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_goals(&self, user_id: Uuid) -> Result<Vec<Value>, DatabaseError> {
        UserDbOps::get_user_goals(self, user_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn update_goal_progress(
        &self,
        goal_id: &str,
        user_id: Uuid,
        current_value: f64,
    ) -> Result<(), DatabaseError> {
        UserDbOps::update_goal_progress(self, goal_id, user_id, current_value)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_configuration(&self, user_id: &str) -> Result<Option<String>, DatabaseError> {
        UserDbOps::get_user_configuration(self, user_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn save_configuration(
        &self,
        user_id: &str,
        config_json: &str,
    ) -> Result<(), DatabaseError> {
        UserDbOps::save_user_configuration(self, user_id, config_json)
            .await
            .map_err(app_error_to_db)
    }
}

#[async_trait]
impl<T: SocialDbOps> InsightRepository for T {
    async fn store(&self, user_id: Uuid, insight_data: Value) -> Result<String, DatabaseError> {
        SocialDbOps::store_insight(self, user_id, insight_data)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_for_user(
        &self,
        user_id: Uuid,
        insight_type: Option<&str>,
        limit: Option<u32>,
    ) -> Result<Vec<Value>, DatabaseError> {
        SocialDbOps::get_user_insights(self, user_id, insight_type, limit)
            .await
            .map_err(app_error_to_db)
    }
}

#[async_trait]
impl<T: AdminDbOps> AdminRepository for T {
    async fn create_token(
        &self,
        request: &CreateAdminTokenRequest,
        admin_jwt_secret: &str,
        jwks_manager: &dyn JwtSigner,
    ) -> Result<GeneratedAdminToken, DatabaseError> {
        AdminDbOps::create_admin_token(self, request, admin_jwt_secret, jwks_manager)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_token_by_id(&self, token_id: &str) -> Result<Option<AdminToken>, DatabaseError> {
        AdminDbOps::get_admin_token_by_id(self, token_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_token_by_prefix(
        &self,
        token_prefix: &str,
    ) -> Result<Option<AdminToken>, DatabaseError> {
        AdminDbOps::get_admin_token_by_prefix(self, token_prefix)
            .await
            .map_err(app_error_to_db)
    }
    async fn list_tokens(&self, include_inactive: bool) -> Result<Vec<AdminToken>, DatabaseError> {
        AdminDbOps::list_admin_tokens(self, include_inactive)
            .await
            .map_err(app_error_to_db)
    }
    async fn deactivate_token(&self, token_id: &str) -> Result<(), DatabaseError> {
        AdminDbOps::deactivate_admin_token(self, token_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn update_token_last_used(
        &self,
        token_id: &str,
        ip_address: Option<&str>,
    ) -> Result<(), DatabaseError> {
        AdminDbOps::update_admin_token_last_used(self, token_id, ip_address)
            .await
            .map_err(app_error_to_db)
    }
    async fn record_token_usage(&self, usage: &AdminTokenUsage) -> Result<(), DatabaseError> {
        AdminDbOps::record_admin_token_usage(self, usage)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_token_usage_history(
        &self,
        token_id: &str,
        start_date: DateTime<Utc>,
        end_date: DateTime<Utc>,
    ) -> Result<Vec<AdminTokenUsage>, DatabaseError> {
        AdminDbOps::get_admin_token_usage_history(self, token_id, start_date, end_date)
            .await
            .map_err(app_error_to_db)
    }
    async fn record_provisioned_key(
        &self,
        admin_token_id: &str,
        api_key_id: &str,
        user_email: &str,
        tier: &str,
        rate_limit_requests: u32,
        rate_limit_period: &str,
    ) -> Result<(), DatabaseError> {
        AdminDbOps::record_admin_provisioned_key(
            self,
            admin_token_id,
            api_key_id,
            user_email,
            tier,
            rate_limit_requests,
            rate_limit_period,
        )
        .await
        .map_err(app_error_to_db)
    }
    async fn get_provisioned_keys(
        &self,
        admin_token_id: Option<&str>,
        start_date: DateTime<Utc>,
        end_date: DateTime<Utc>,
    ) -> Result<Vec<Value>, DatabaseError> {
        AdminDbOps::get_admin_provisioned_keys(self, admin_token_id, start_date, end_date)
            .await
            .map_err(app_error_to_db)
    }
}

#[async_trait]
impl<T: TenantDbOps> TenantRepository for T {
    async fn create(&self, tenant: &Tenant) -> Result<(), DatabaseError> {
        TenantDbOps::create_tenant(self, tenant)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_by_id(&self, tenant_id: TenantId) -> Result<Tenant, DatabaseError> {
        TenantDbOps::get_tenant_by_id(self, tenant_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_by_slug(&self, slug: &str) -> Result<Tenant, DatabaseError> {
        TenantDbOps::get_tenant_by_slug(self, slug)
            .await
            .map_err(app_error_to_db)
    }
    async fn list_for_user(&self, user_id: Uuid) -> Result<Vec<Tenant>, DatabaseError> {
        TenantDbOps::list_tenants_for_user(self, user_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn store_oauth_credentials(
        &self,
        credentials: &TenantOAuthCredentials,
    ) -> Result<(), DatabaseError> {
        TenantDbOps::store_tenant_oauth_credentials(self, credentials)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_oauth_providers(
        &self,
        tenant_id: TenantId,
    ) -> Result<Vec<TenantOAuthCredentials>, DatabaseError> {
        TenantDbOps::get_tenant_oauth_providers(self, tenant_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_oauth_credentials(
        &self,
        tenant_id: TenantId,
        provider: &str,
    ) -> Result<Option<TenantOAuthCredentials>, DatabaseError> {
        TenantDbOps::get_tenant_oauth_credentials(self, tenant_id, provider)
            .await
            .map_err(app_error_to_db)
    }
    async fn create_oauth_app(&self, app: &OAuthApp) -> Result<(), DatabaseError> {
        TenantDbOps::create_oauth_app(self, app)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_oauth_app_by_client_id(&self, client_id: &str) -> Result<OAuthApp, DatabaseError> {
        TenantDbOps::get_oauth_app_by_client_id(self, client_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn list_oauth_apps_for_user(
        &self,
        user_id: Uuid,
    ) -> Result<Vec<OAuthApp>, DatabaseError> {
        TenantDbOps::list_oauth_apps_for_user(self, user_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_all(&self) -> Result<Vec<Tenant>, DatabaseError> {
        TenantDbOps::get_all_tenants(self)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_user_role(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> Result<Option<String>, DatabaseError> {
        TenantDbOps::get_user_tenant_role(self, user_id, tenant_id)
            .await
            .map_err(app_error_to_db)
    }
}

#[async_trait]
impl<T: OAuthDbOps> OAuth2ServerRepository for T {
    async fn store_client(&self, client: &OAuth2Client) -> Result<(), DatabaseError> {
        OAuthDbOps::store_oauth2_client(self, client)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_client(&self, client_id: &str) -> Result<Option<OAuth2Client>, DatabaseError> {
        OAuthDbOps::get_oauth2_client(self, client_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn store_auth_code(&self, auth_code: &OAuth2AuthCode) -> Result<(), DatabaseError> {
        OAuthDbOps::store_oauth2_auth_code(self, auth_code)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_auth_code(&self, code: &str) -> Result<Option<OAuth2AuthCode>, DatabaseError> {
        OAuthDbOps::get_oauth2_auth_code(self, code)
            .await
            .map_err(app_error_to_db)
    }
    async fn update_auth_code(&self, auth_code: &OAuth2AuthCode) -> Result<(), DatabaseError> {
        OAuthDbOps::update_oauth2_auth_code(self, auth_code)
            .await
            .map_err(app_error_to_db)
    }
    async fn store_refresh_token(
        &self,
        refresh_token: &OAuth2RefreshToken,
    ) -> Result<(), DatabaseError> {
        OAuthDbOps::store_oauth2_refresh_token(self, refresh_token)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_refresh_token(
        &self,
        token: &str,
    ) -> Result<Option<OAuth2RefreshToken>, DatabaseError> {
        OAuthDbOps::get_oauth2_refresh_token(self, token)
            .await
            .map_err(app_error_to_db)
    }
    async fn revoke_refresh_token(&self, token: &str) -> Result<(), DatabaseError> {
        OAuthDbOps::revoke_oauth2_refresh_token(self, token)
            .await
            .map_err(app_error_to_db)
    }
    async fn consume_auth_code(
        &self,
        code: &str,
        client_id: &str,
        redirect_uri: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<OAuth2AuthCode>, DatabaseError> {
        OAuthDbOps::consume_auth_code(self, code, client_id, redirect_uri, now)
            .await
            .map_err(app_error_to_db)
    }
    async fn consume_refresh_token(
        &self,
        token: &str,
        client_id: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<OAuth2RefreshToken>, DatabaseError> {
        OAuthDbOps::consume_refresh_token(self, token, client_id, now)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_refresh_token_by_value(
        &self,
        token: &str,
    ) -> Result<Option<OAuth2RefreshToken>, DatabaseError> {
        OAuthDbOps::get_refresh_token_by_value(self, token)
            .await
            .map_err(app_error_to_db)
    }
    async fn store_authorization_code(
        &self,
        code: &str,
        client_id: &str,
        redirect_uri: &str,
        scope: &str,
        user_id: Uuid,
    ) -> Result<(), DatabaseError> {
        OAuthDbOps::store_authorization_code(self, code, client_id, redirect_uri, scope, user_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_authorization_code(&self, code: &str) -> Result<AuthorizationCode, DatabaseError> {
        OAuthDbOps::get_authorization_code(self, code)
            .await
            .map_err(app_error_to_db)
    }
    async fn delete_authorization_code(&self, code: &str) -> Result<(), DatabaseError> {
        OAuthDbOps::delete_authorization_code(self, code)
            .await
            .map_err(app_error_to_db)
    }
    async fn store_state(&self, state: &OAuth2State) -> Result<(), DatabaseError> {
        OAuthDbOps::store_oauth2_state(self, state)
            .await
            .map_err(app_error_to_db)
    }
    async fn consume_state(
        &self,
        state_value: &str,
        client_id: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<OAuth2State>, DatabaseError> {
        OAuthDbOps::consume_oauth2_state(self, state_value, client_id, now)
            .await
            .map_err(app_error_to_db)
    }
}

#[async_trait]
impl<T: SecurityDbOps> SecurityRepository for T {
    async fn save_rsa_keypair(
        &self,
        kid: &str,
        private_key_pem: &str,
        public_key_pem: &str,
        created_at: DateTime<Utc>,
        is_active: bool,
        key_size_bits: i32,
    ) -> Result<(), DatabaseError> {
        SecurityDbOps::save_rsa_keypair(
            self,
            kid,
            private_key_pem,
            public_key_pem,
            created_at,
            is_active,
            key_size_bits,
        )
        .await
        .map_err(app_error_to_db)
    }
    async fn load_rsa_keypairs(
        &self,
    ) -> Result<Vec<(String, String, String, DateTime<Utc>, bool)>, DatabaseError> {
        <Self as SecurityDbOps>::load_rsa_keypairs(self)
            .await
            .map_err(app_error_to_db)
    }
    async fn update_rsa_keypair_active_status(
        &self,
        kid: &str,
        is_active: bool,
    ) -> Result<(), DatabaseError> {
        SecurityDbOps::update_rsa_keypair_active_status(self, kid, is_active)
            .await
            .map_err(app_error_to_db)
    }
    async fn store_key_version(&self, version: &KeyVersion) -> Result<(), DatabaseError> {
        SecurityDbOps::store_key_version(self, version)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_key_versions(
        &self,
        tenant_id: Option<TenantId>,
    ) -> Result<Vec<KeyVersion>, DatabaseError> {
        SecurityDbOps::get_key_versions(self, tenant_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_current_key_version(
        &self,
        tenant_id: Option<TenantId>,
    ) -> Result<Option<KeyVersion>, DatabaseError> {
        SecurityDbOps::get_current_key_version(self, tenant_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn update_key_version_status(
        &self,
        tenant_id: Option<TenantId>,
        version: u32,
        is_active: bool,
    ) -> Result<(), DatabaseError> {
        SecurityDbOps::update_key_version_status(self, tenant_id, version, is_active)
            .await
            .map_err(app_error_to_db)
    }
    async fn delete_old_key_versions(
        &self,
        tenant_id: Option<TenantId>,
        keep_count: u32,
    ) -> Result<u64, DatabaseError> {
        SecurityDbOps::delete_old_key_versions(self, tenant_id, keep_count)
            .await
            .map_err(app_error_to_db)
    }
    async fn store_audit_event(&self, event: &AuditEvent) -> Result<(), DatabaseError> {
        SecurityDbOps::store_audit_event(self, event)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_audit_events(
        &self,
        tenant_id: Option<TenantId>,
        event_type: Option<&str>,
        limit: Option<u32>,
    ) -> Result<Vec<AuditEvent>, DatabaseError> {
        SecurityDbOps::get_audit_events(self, tenant_id, event_type, limit)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_or_create_system_secret(
        &self,
        secret_type: &str,
    ) -> Result<String, DatabaseError> {
        SecurityDbOps::get_or_create_system_secret(self, secret_type)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_system_secret(&self, secret_type: &str) -> Result<String, DatabaseError> {
        SecurityDbOps::get_system_secret(self, secret_type)
            .await
            .map_err(app_error_to_db)
    }
    async fn update_system_secret(
        &self,
        secret_type: &str,
        new_value: &str,
    ) -> Result<(), DatabaseError> {
        SecurityDbOps::update_system_secret(self, secret_type, new_value)
            .await
            .map_err(app_error_to_db)
    }
    fn encrypt_data_with_aad(&self, data: &str, aad: &str) -> Result<String, DatabaseError> {
        SecurityDbOps::encrypt_data_with_aad(self, data, aad).map_err(app_error_to_db)
    }
    fn decrypt_data_with_aad(&self, encrypted: &str, aad: &str) -> Result<String, DatabaseError> {
        SecurityDbOps::decrypt_data_with_aad(self, encrypted, aad).map_err(app_error_to_db)
    }
}

#[async_trait]
impl<T: SecurityDbOps> NotificationRepository for T {
    async fn store(
        &self,
        user_id: Uuid,
        provider: &str,
        success: bool,
        message: &str,
        expires_at: Option<&str>,
    ) -> Result<String, DatabaseError> {
        SecurityDbOps::store_oauth_notification(
            self, user_id, provider, success, message, expires_at,
        )
        .await
        .map_err(app_error_to_db)
    }
    async fn get_unread(&self, user_id: Uuid) -> Result<Vec<OAuthNotification>, DatabaseError> {
        SecurityDbOps::get_unread_oauth_notifications(self, user_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn mark_read(&self, notification_id: &str, user_id: Uuid) -> Result<bool, DatabaseError> {
        SecurityDbOps::mark_oauth_notification_read(self, notification_id, user_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn mark_all_read(&self, user_id: Uuid) -> Result<u64, DatabaseError> {
        SecurityDbOps::mark_all_oauth_notifications_read(self, user_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_all(
        &self,
        user_id: Uuid,
        limit: Option<i64>,
    ) -> Result<Vec<OAuthNotification>, DatabaseError> {
        SecurityDbOps::get_all_oauth_notifications(self, user_id, limit)
            .await
            .map_err(app_error_to_db)
    }
}

#[async_trait]
impl<T: TenantDbOps> FitnessConfigRepository for T {
    async fn save_tenant_config(
        &self,
        tenant_id: TenantId,
        configuration_name: &str,
        config: &FitnessConfig,
    ) -> Result<String, DatabaseError> {
        TenantDbOps::save_tenant_fitness_config(self, tenant_id, configuration_name, config)
            .await
            .map_err(app_error_to_db)
    }
    async fn save_user_config(
        &self,
        tenant_id: TenantId,
        user_id: &str,
        configuration_name: &str,
        config: &FitnessConfig,
    ) -> Result<String, DatabaseError> {
        TenantDbOps::save_user_fitness_config(self, tenant_id, user_id, configuration_name, config)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_tenant_config(
        &self,
        tenant_id: TenantId,
        configuration_name: &str,
    ) -> Result<Option<FitnessConfig>, DatabaseError> {
        TenantDbOps::get_tenant_fitness_config(self, tenant_id, configuration_name)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_user_config(
        &self,
        tenant_id: TenantId,
        user_id: &str,
        configuration_name: &str,
    ) -> Result<Option<FitnessConfig>, DatabaseError> {
        TenantDbOps::get_user_fitness_config(self, tenant_id, user_id, configuration_name)
            .await
            .map_err(app_error_to_db)
    }
    async fn list_tenant_configurations(
        &self,
        tenant_id: TenantId,
    ) -> Result<Vec<String>, DatabaseError> {
        TenantDbOps::list_tenant_fitness_configurations(self, tenant_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn list_user_configurations(
        &self,
        tenant_id: TenantId,
        user_id: &str,
    ) -> Result<Vec<String>, DatabaseError> {
        TenantDbOps::list_user_fitness_configurations(self, tenant_id, user_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn delete_config(
        &self,
        tenant_id: TenantId,
        user_id: Option<&str>,
        configuration_name: &str,
    ) -> Result<bool, DatabaseError> {
        TenantDbOps::delete_fitness_config(self, tenant_id, user_id, configuration_name)
            .await
            .map_err(app_error_to_db)
    }
}

#[async_trait]
impl<T: ChatDbOps> ChatRepository for T {
    async fn create_conversation(
        &self,
        user_id: &str,
        tenant_id: TenantId,
        title: &str,
        model: &str,
        system_prompt: Option<&str>,
    ) -> Result<ConversationRecord, DatabaseError> {
        ChatDbOps::chat_create_conversation(self, user_id, tenant_id, title, model, system_prompt)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_conversation(
        &self,
        conversation_id: &str,
        user_id: &str,
        tenant_id: TenantId,
    ) -> Result<Option<ConversationRecord>, DatabaseError> {
        ChatDbOps::chat_get_conversation(self, conversation_id, user_id, tenant_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn list_conversations(
        &self,
        user_id: &str,
        tenant_id: TenantId,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<ConversationSummary>, DatabaseError> {
        ChatDbOps::chat_list_conversations(self, user_id, tenant_id, limit, offset)
            .await
            .map_err(app_error_to_db)
    }
    async fn update_conversation_title(
        &self,
        conversation_id: &str,
        user_id: &str,
        tenant_id: TenantId,
        title: &str,
    ) -> Result<bool, DatabaseError> {
        ChatDbOps::chat_update_conversation_title(self, conversation_id, user_id, tenant_id, title)
            .await
            .map_err(app_error_to_db)
    }
    async fn delete_conversation(
        &self,
        conversation_id: &str,
        user_id: &str,
        tenant_id: TenantId,
    ) -> Result<bool, DatabaseError> {
        ChatDbOps::chat_delete_conversation(self, conversation_id, user_id, tenant_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn add_message(
        &self,
        params: &AddMessageParams<'_>,
    ) -> Result<MessageRecord, DatabaseError> {
        ChatDbOps::chat_add_message(self, params)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_messages(
        &self,
        conversation_id: &str,
        user_id: &str,
    ) -> Result<Vec<MessageRecord>, DatabaseError> {
        ChatDbOps::chat_get_messages(self, conversation_id, user_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_recent_messages(
        &self,
        conversation_id: &str,
        user_id: &str,
        limit: i64,
    ) -> Result<Vec<MessageRecord>, DatabaseError> {
        ChatDbOps::chat_get_recent_messages(self, conversation_id, user_id, limit)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_message_count(
        &self,
        conversation_id: &str,
        user_id: &str,
    ) -> Result<i64, DatabaseError> {
        ChatDbOps::chat_get_message_count(self, conversation_id, user_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn count_conversations(
        &self,
        user_id: &str,
        tenant_id: TenantId,
    ) -> Result<i64, DatabaseError> {
        ChatDbOps::chat_count_conversations(self, user_id, tenant_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn delete_all_user_conversations(
        &self,
        user_id: &str,
        tenant_id: TenantId,
    ) -> Result<i64, DatabaseError> {
        ChatDbOps::chat_delete_all_user_conversations(self, user_id, tenant_id)
            .await
            .map_err(app_error_to_db)
    }
}

#[async_trait]
impl<T: AdminDbOps> UserMcpTokenRepository for T {
    async fn create_token(
        &self,
        user_id: Uuid,
        request: &CreateUserMcpTokenRequest,
    ) -> Result<UserMcpTokenCreated, DatabaseError> {
        AdminDbOps::create_user_mcp_token(self, user_id, request)
            .await
            .map_err(app_error_to_db)
    }
    async fn validate_token(&self, token_value: &str) -> Result<Uuid, DatabaseError> {
        AdminDbOps::validate_user_mcp_token(self, token_value)
            .await
            .map_err(app_error_to_db)
    }
    async fn list_tokens(&self, user_id: Uuid) -> Result<Vec<UserMcpTokenInfo>, DatabaseError> {
        AdminDbOps::list_user_mcp_tokens(self, user_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn revoke_token(&self, token_id: &str, user_id: Uuid) -> Result<(), DatabaseError> {
        AdminDbOps::revoke_user_mcp_token(self, token_id, user_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_token(
        &self,
        token_id: &str,
        user_id: Uuid,
    ) -> Result<Option<UserMcpToken>, DatabaseError> {
        AdminDbOps::get_user_mcp_token(self, token_id, user_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn cleanup_expired_tokens(&self) -> Result<u64, DatabaseError> {
        AdminDbOps::cleanup_expired_user_mcp_tokens(self)
            .await
            .map_err(app_error_to_db)
    }
}

#[async_trait]
impl<T: AdminDbOps> ImpersonationRepository for T {
    async fn create_session(&self, session: &ImpersonationSession) -> Result<(), DatabaseError> {
        AdminDbOps::create_impersonation_session(self, session)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_session(
        &self,
        session_id: &str,
    ) -> Result<Option<ImpersonationSession>, DatabaseError> {
        AdminDbOps::get_impersonation_session(self, session_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_active_session(
        &self,
        user_id: Uuid,
    ) -> Result<Option<ImpersonationSession>, DatabaseError> {
        AdminDbOps::get_active_impersonation_session(self, user_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn end_session(&self, session_id: &str) -> Result<(), DatabaseError> {
        AdminDbOps::end_impersonation_session(self, session_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn end_all_sessions(&self, impersonator_id: Uuid) -> Result<u64, DatabaseError> {
        AdminDbOps::end_all_impersonation_sessions(self, impersonator_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn list_sessions(
        &self,
        impersonator_id: Option<Uuid>,
        target_user_id: Option<Uuid>,
        active_only: bool,
        limit: u32,
    ) -> Result<Vec<ImpersonationSession>, DatabaseError> {
        AdminDbOps::list_impersonation_sessions(
            self,
            impersonator_id,
            target_user_id,
            active_only,
            limit,
        )
        .await
        .map_err(app_error_to_db)
    }
}

#[async_trait]
impl<T: TenantDbOps> LlmCredentialRepository for T {
    async fn store_credentials(&self, record: &LlmCredentialRecord) -> Result<(), DatabaseError> {
        TenantDbOps::store_llm_credentials(self, record)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_credentials(
        &self,
        tenant_id: TenantId,
        user_id: Option<Uuid>,
        provider: &str,
    ) -> Result<Option<LlmCredentialRecord>, DatabaseError> {
        TenantDbOps::get_llm_credentials(self, tenant_id, user_id, provider)
            .await
            .map_err(app_error_to_db)
    }
    async fn list_credentials(
        &self,
        tenant_id: TenantId,
    ) -> Result<Vec<LlmCredentialSummary>, DatabaseError> {
        TenantDbOps::list_llm_credentials(self, tenant_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn delete_credentials(
        &self,
        tenant_id: TenantId,
        user_id: Option<Uuid>,
        provider: &str,
    ) -> Result<bool, DatabaseError> {
        TenantDbOps::delete_llm_credentials(self, tenant_id, user_id, provider)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_admin_config_override(
        &self,
        config_key: &str,
        tenant_id: Option<TenantId>,
    ) -> Result<Option<String>, DatabaseError> {
        TenantDbOps::get_admin_config_override(self, config_key, tenant_id)
            .await
            .map_err(app_error_to_db)
    }
}

#[async_trait]
impl<T: OAuthDbOps> ProviderConnectionRepository for T {
    async fn register_connection(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        connection_type: &ConnectionType,
        metadata: Option<&str>,
    ) -> Result<(), DatabaseError> {
        OAuthDbOps::register_provider_connection(
            self,
            user_id,
            tenant_id,
            provider,
            connection_type,
            metadata,
        )
        .await
        .map_err(app_error_to_db)
    }
    async fn remove_connection(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) -> Result<(), DatabaseError> {
        OAuthDbOps::remove_provider_connection(self, user_id, tenant_id, provider)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_for_user(
        &self,
        user_id: Uuid,
        tenant_id: Option<TenantId>,
    ) -> Result<Vec<ProviderConnection>, DatabaseError> {
        OAuthDbOps::get_user_provider_connections(self, user_id, tenant_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn is_connected(&self, user_id: Uuid, provider: &str) -> Result<bool, DatabaseError> {
        OAuthDbOps::is_provider_connected(self, user_id, provider)
            .await
            .map_err(app_error_to_db)
    }
}

#[async_trait]
impl<T: OAuthDbOps> PasswordResetRepository for T {
    async fn store_token(
        &self,
        user_id: Uuid,
        token_hash: &str,
        created_by: &str,
    ) -> Result<Uuid, DatabaseError> {
        OAuthDbOps::store_password_reset_token(self, user_id, token_hash, created_by)
            .await
            .map_err(app_error_to_db)
    }
    async fn consume_token(&self, token_hash: &str) -> Result<Uuid, DatabaseError> {
        OAuthDbOps::consume_password_reset_token(self, token_hash)
            .await
            .map_err(app_error_to_db)
    }
    async fn invalidate_tokens(&self, user_id: Uuid) -> Result<(), DatabaseError> {
        OAuthDbOps::invalidate_user_reset_tokens(self, user_id)
            .await
            .map_err(app_error_to_db)
    }
}

#[async_trait]
impl<T: OAuthDbOps> OAuthClientStateRepository for T {
    async fn store_oauth_client_state(
        &self,
        state: &OAuthClientState,
    ) -> Result<(), DatabaseError> {
        OAuthDbOps::store_oauth_client_state(self, state)
            .await
            .map_err(app_error_to_db)
    }
    async fn consume_oauth_client_state(
        &self,
        state_value: &str,
        provider: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<OAuthClientState>, DatabaseError> {
        OAuthDbOps::consume_oauth_client_state(self, state_value, provider, now)
            .await
            .map_err(app_error_to_db)
    }
}

#[async_trait]
impl<T: TenantDbOps> ToolSelectionRepository for T {
    async fn get_tool_catalog(&self) -> Result<Vec<ToolCatalogEntry>, DatabaseError> {
        TenantDbOps::get_tool_catalog(self)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_tool_catalog_entry(
        &self,
        tool_name: &str,
    ) -> Result<Option<ToolCatalogEntry>, DatabaseError> {
        TenantDbOps::get_tool_catalog_entry(self, tool_name)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_tools_by_category(
        &self,
        category: ToolCategory,
    ) -> Result<Vec<ToolCatalogEntry>, DatabaseError> {
        TenantDbOps::get_tools_by_category(self, category)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_tools_by_min_plan(
        &self,
        plan: TenantPlan,
    ) -> Result<Vec<ToolCatalogEntry>, DatabaseError> {
        TenantDbOps::get_tools_by_min_plan(self, plan)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_overrides(
        &self,
        tenant_id: TenantId,
    ) -> Result<Vec<TenantToolOverride>, DatabaseError> {
        TenantDbOps::get_tenant_tool_overrides(self, tenant_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_override(
        &self,
        tenant_id: TenantId,
        tool_name: &str,
    ) -> Result<Option<TenantToolOverride>, DatabaseError> {
        TenantDbOps::get_tenant_tool_override(self, tenant_id, tool_name)
            .await
            .map_err(app_error_to_db)
    }
    async fn upsert_override(
        &self,
        tenant_id: TenantId,
        tool_name: &str,
        is_enabled: bool,
        enabled_by_user_id: Option<Uuid>,
        reason: Option<String>,
    ) -> Result<TenantToolOverride, DatabaseError> {
        TenantDbOps::upsert_tenant_tool_override(
            self,
            tenant_id,
            tool_name,
            is_enabled,
            enabled_by_user_id,
            reason,
        )
        .await
        .map_err(app_error_to_db)
    }
    async fn delete_override(
        &self,
        tenant_id: TenantId,
        tool_name: &str,
    ) -> Result<bool, DatabaseError> {
        TenantDbOps::delete_tenant_tool_override(self, tenant_id, tool_name)
            .await
            .map_err(app_error_to_db)
    }
    async fn count_enabled_tools(&self, tenant_id: TenantId) -> Result<usize, DatabaseError> {
        TenantDbOps::count_enabled_tools(self, tenant_id)
            .await
            .map_err(app_error_to_db)
    }
}

#[async_trait]
impl<T: UsageDbOps> LlmUsageRepository for T {
    async fn insert_llm_usage(
        &self,
        params: &InsertLlmUsage<'_>,
    ) -> Result<LlmUsageRecord, DatabaseError> {
        UsageDbOps::insert_llm_usage(self, params)
            .await
            .map_err(app_error_to_db)
    }

    async fn get_llm_usage_aggregates(
        &self,
        tenant_id: &str,
        since: &str,
    ) -> Result<Vec<LlmUsageAggregateRow>, DatabaseError> {
        UsageDbOps::get_llm_usage_aggregates(self, tenant_id, since)
            .await
            .map_err(app_error_to_db)
    }

    async fn get_llm_usage_daily_series(
        &self,
        tenant_id: &str,
        since: &str,
    ) -> Result<Vec<LlmUsageDailyRow>, DatabaseError> {
        UsageDbOps::get_llm_usage_daily_series(self, tenant_id, since)
            .await
            .map_err(app_error_to_db)
    }
}

#[async_trait]
impl<T: UsageDbOps> UsageCounterRepository for T {
    async fn increment_counter(
        &self,
        tenant_id: &str,
        user_id: &str,
        counter_key: &str,
        period: &str,
        amount: i64,
    ) -> Result<UsageCounterRecord, DatabaseError> {
        UsageDbOps::increment_usage_counter(self, tenant_id, user_id, counter_key, period, amount)
            .await
            .map_err(app_error_to_db)
    }

    async fn get_counter(
        &self,
        tenant_id: &str,
        user_id: &str,
        counter_key: &str,
        period: &str,
    ) -> Result<UsageCounterRecord, DatabaseError> {
        UsageDbOps::get_usage_counter(self, tenant_id, user_id, counter_key, period)
            .await
            .map_err(app_error_to_db)
    }

    async fn delete_old_counters(&self, period_before: &str) -> Result<u64, DatabaseError> {
        UsageDbOps::delete_old_usage_counters(self, period_before)
            .await
            .map_err(app_error_to_db)
    }
}
