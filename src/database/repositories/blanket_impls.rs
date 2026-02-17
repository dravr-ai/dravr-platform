// ABOUTME: Blanket implementations of repository traits delegating to DatabaseProvider
// ABOUTME: Each impl maps the focused repository trait methods to DatabaseProvider god-trait methods
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2025 Pierre Fitness Intelligence

use super::{
    A2ARepository, AdminRepository, ApiKeyRepository, ChatRepository, FitnessConfigRepository,
    ImpersonationRepository, InsightRepository, LlmCredentialRepository, NotificationRepository,
    OAuth2ServerRepository, OAuthClientStateRepository, OAuthTokenRepository,
    PasswordResetRepository, ProfileRepository, ProviderConnectionRepository, SecurityRepository,
    TenantRepository, ToolSelectionRepository, UsageRepository, UserMcpTokenRepository,
    UserRepository,
};
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
use crate::database::{
    A2AUsage, A2AUsageStats, ConversationRecord, ConversationSummary, CreateUserMcpTokenRequest,
    DatabaseError, MessageRecord, UserMcpToken, UserMcpTokenCreated, UserMcpTokenInfo,
};
use crate::database_plugins::DatabaseProvider;
use crate::errors::{AppError, ErrorCode};

/// Convert `AppError` to `DatabaseError` preserving not-found semantics.
/// `DatabaseProvider` methods return `AppResult<T>` (using `AppError`), but repository
/// traits return `Result<T, DatabaseError>`. This helper maps `AppError::not_found`
/// to `DatabaseError::NotFound` so 404 status codes propagate correctly through
/// the `From<DatabaseError> for AppError` conversion in route handlers.
fn app_error_to_db(e: AppError) -> DatabaseError {
    if e.code == ErrorCode::ResourceNotFound {
        DatabaseError::NotFound {
            entity_type: "resource",
            entity_id: e.message.clone(),
        }
    } else {
        DatabaseError::QueryError {
            context: e.to_string(),
        }
    }
}
use crate::models::{
    AuthorizationCode, ConnectionType, OAuthApp, OAuthNotification, ProviderConnection, Tenant,
    TenantPlan, TenantToolOverride, ToolCatalogEntry, ToolCategory, User, UserOAuthApp,
    UserOAuthToken, UserStatus,
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

#[async_trait]
impl<T: DatabaseProvider> UserRepository for T {
    async fn create(&self, user: &User) -> Result<Uuid, DatabaseError> {
        DatabaseProvider::create_user(self, user)
            .await
            .map_err(app_error_to_db)
    }
    async fn get(&self, user_id: Uuid, tenant_id: TenantId) -> Result<Option<User>, DatabaseError> {
        DatabaseProvider::get_user(self, user_id, tenant_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_global(&self, user_id: Uuid) -> Result<Option<User>, DatabaseError> {
        DatabaseProvider::get_user_global(self, user_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_by_email(&self, email: &str) -> Result<Option<User>, DatabaseError> {
        DatabaseProvider::get_user_by_email(self, email)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_by_email_required(&self, email: &str) -> Result<User, DatabaseError> {
        DatabaseProvider::get_user_by_email_required(self, email)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_by_firebase_uid(&self, firebase_uid: &str) -> Result<Option<User>, DatabaseError> {
        DatabaseProvider::get_user_by_firebase_uid(self, firebase_uid)
            .await
            .map_err(app_error_to_db)
    }
    async fn update_last_active(&self, user_id: Uuid) -> Result<(), DatabaseError> {
        DatabaseProvider::update_last_active(self, user_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn count(&self) -> Result<i64, DatabaseError> {
        DatabaseProvider::get_user_count(self)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_by_status(
        &self,
        status: &str,
        tenant_id: Option<TenantId>,
    ) -> Result<Vec<User>, DatabaseError> {
        DatabaseProvider::get_users_by_status(self, status, tenant_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_by_status_cursor(
        &self,
        status: &str,
        params: &PaginationParams,
    ) -> Result<CursorPage<User>, DatabaseError> {
        DatabaseProvider::get_users_by_status_cursor(self, status, params)
            .await
            .map_err(app_error_to_db)
    }
    async fn update_status(
        &self,
        user_id: Uuid,
        new_status: UserStatus,
        approved_by: Option<Uuid>,
    ) -> Result<User, DatabaseError> {
        DatabaseProvider::update_user_status(self, user_id, new_status, approved_by)
            .await
            .map_err(app_error_to_db)
    }
    async fn update_tenant_id(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> Result<(), DatabaseError> {
        DatabaseProvider::update_user_tenant_id(self, user_id, tenant_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn update_password(
        &self,
        user_id: Uuid,
        password_hash: &str,
    ) -> Result<(), DatabaseError> {
        DatabaseProvider::update_user_password(self, user_id, password_hash)
            .await
            .map_err(app_error_to_db)
    }
    async fn update_display_name(
        &self,
        user_id: Uuid,
        display_name: &str,
    ) -> Result<User, DatabaseError> {
        DatabaseProvider::update_user_display_name(self, user_id, display_name)
            .await
            .map_err(app_error_to_db)
    }
    async fn delete(&self, user_id: Uuid) -> Result<(), DatabaseError> {
        DatabaseProvider::delete_user(self, user_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_first_admin_user(&self) -> Result<Option<User>, DatabaseError> {
        DatabaseProvider::get_first_admin_user(self)
            .await
            .map_err(app_error_to_db)
    }
    async fn has_synthetic_activities(&self, user_id: Uuid) -> Result<bool, DatabaseError> {
        DatabaseProvider::user_has_synthetic_activities(self, user_id)
            .await
            .map_err(app_error_to_db)
    }
}

#[async_trait]
impl<T: DatabaseProvider> OAuthTokenRepository for T {
    async fn upsert_token(&self, token: &UserOAuthToken) -> Result<(), DatabaseError> {
        DatabaseProvider::upsert_user_oauth_token(self, token)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_token(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) -> Result<Option<UserOAuthToken>, DatabaseError> {
        DatabaseProvider::get_user_oauth_token(self, user_id, tenant_id, provider)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_tokens(
        &self,
        user_id: Uuid,
        tenant_id: Option<TenantId>,
    ) -> Result<Vec<UserOAuthToken>, DatabaseError> {
        DatabaseProvider::get_user_oauth_tokens(self, user_id, tenant_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_tenant_provider_tokens(
        &self,
        tenant_id: TenantId,
        provider: &str,
    ) -> Result<Vec<UserOAuthToken>, DatabaseError> {
        DatabaseProvider::get_tenant_provider_tokens(self, tenant_id, provider)
            .await
            .map_err(app_error_to_db)
    }
    async fn delete_token(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) -> Result<(), DatabaseError> {
        DatabaseProvider::delete_user_oauth_token(self, user_id, tenant_id, provider)
            .await
            .map_err(app_error_to_db)
    }
    async fn delete_tokens(&self, user_id: Uuid, tenant_id: TenantId) -> Result<(), DatabaseError> {
        DatabaseProvider::delete_user_oauth_tokens(self, user_id, tenant_id)
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
        DatabaseProvider::refresh_user_oauth_token(
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
        DatabaseProvider::store_user_oauth_app(
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
        DatabaseProvider::get_user_oauth_app(self, user_id, provider)
            .await
            .map_err(app_error_to_db)
    }
    async fn list_user_oauth_apps(
        &self,
        user_id: Uuid,
    ) -> Result<Vec<UserOAuthApp>, DatabaseError> {
        DatabaseProvider::list_user_oauth_apps(self, user_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn remove_user_oauth_app(
        &self,
        user_id: Uuid,
        provider: &str,
    ) -> Result<(), DatabaseError> {
        DatabaseProvider::remove_user_oauth_app(self, user_id, provider)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_provider_last_sync(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) -> Result<Option<DateTime<Utc>>, DatabaseError> {
        DatabaseProvider::get_provider_last_sync(self, user_id, tenant_id, provider)
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
        DatabaseProvider::update_provider_last_sync(self, user_id, tenant_id, provider, sync_time)
            .await
            .map_err(app_error_to_db)
    }
}

#[async_trait]
impl<T: DatabaseProvider> ApiKeyRepository for T {
    async fn create(&self, api_key: &ApiKey) -> Result<(), DatabaseError> {
        DatabaseProvider::create_api_key(self, api_key)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_by_prefix(
        &self,
        prefix: &str,
        hash: &str,
    ) -> Result<Option<ApiKey>, DatabaseError> {
        DatabaseProvider::get_api_key_by_prefix(self, prefix, hash)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_for_user(&self, user_id: Uuid) -> Result<Vec<ApiKey>, DatabaseError> {
        DatabaseProvider::get_user_api_keys(self, user_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn update_last_used(&self, api_key_id: &str) -> Result<(), DatabaseError> {
        DatabaseProvider::update_api_key_last_used(self, api_key_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn deactivate(&self, api_key_id: &str, user_id: Uuid) -> Result<(), DatabaseError> {
        DatabaseProvider::deactivate_api_key(self, api_key_id, user_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_by_id(
        &self,
        api_key_id: &str,
        user_id: Option<Uuid>,
    ) -> Result<Option<ApiKey>, DatabaseError> {
        DatabaseProvider::get_api_key_by_id(self, api_key_id, user_id)
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
        DatabaseProvider::get_api_keys_filtered(self, user_email, active_only, limit, offset)
            .await
            .map_err(app_error_to_db)
    }
    async fn cleanup_expired(&self) -> Result<u64, DatabaseError> {
        DatabaseProvider::cleanup_expired_api_keys(self)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_expired(&self) -> Result<Vec<ApiKey>, DatabaseError> {
        DatabaseProvider::get_expired_api_keys(self)
            .await
            .map_err(app_error_to_db)
    }
}

#[async_trait]
impl<T: DatabaseProvider> UsageRepository for T {
    async fn record_api_key(&self, usage: &ApiKeyUsage) -> Result<(), DatabaseError> {
        DatabaseProvider::record_api_key_usage(self, usage)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_api_key_current(&self, api_key_id: &str) -> Result<u32, DatabaseError> {
        DatabaseProvider::get_api_key_current_usage(self, api_key_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_api_key_stats(
        &self,
        api_key_id: &str,
        start_date: DateTime<Utc>,
        end_date: DateTime<Utc>,
    ) -> Result<ApiKeyUsageStats, DatabaseError> {
        DatabaseProvider::get_api_key_usage_stats(self, api_key_id, start_date, end_date)
            .await
            .map_err(app_error_to_db)
    }
    async fn record_jwt_usage(&self, usage: &JwtUsage) -> Result<(), DatabaseError> {
        DatabaseProvider::record_jwt_usage(self, usage)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_jwt_current_usage(&self, user_id: Uuid) -> Result<u32, DatabaseError> {
        DatabaseProvider::get_jwt_current_usage(self, user_id)
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
        DatabaseProvider::get_request_logs(
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
        DatabaseProvider::get_system_stats(self, tenant_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_top_tools_analysis(
        &self,
        user_id: Uuid,
        start_time: DateTime<Utc>,
        end_time: DateTime<Utc>,
    ) -> Result<Vec<ToolUsage>, DatabaseError> {
        DatabaseProvider::get_top_tools_analysis(self, user_id, start_time, end_time)
            .await
            .map_err(app_error_to_db)
    }
}

#[async_trait]
impl<T: DatabaseProvider> A2ARepository for T {
    async fn create_client(
        &self,
        client: &A2AClient,
        client_secret: &str,
        api_key_id: &str,
    ) -> Result<String, DatabaseError> {
        DatabaseProvider::create_a2a_client(self, client, client_secret, api_key_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_client(&self, client_id: &str) -> Result<Option<A2AClient>, DatabaseError> {
        DatabaseProvider::get_a2a_client(self, client_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_client_by_api_key_id(
        &self,
        api_key_id: &str,
    ) -> Result<Option<A2AClient>, DatabaseError> {
        DatabaseProvider::get_a2a_client_by_api_key_id(self, api_key_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_client_by_name(&self, name: &str) -> Result<Option<A2AClient>, DatabaseError> {
        DatabaseProvider::get_a2a_client_by_name(self, name)
            .await
            .map_err(app_error_to_db)
    }
    async fn list_clients(&self, user_id: &Uuid) -> Result<Vec<A2AClient>, DatabaseError> {
        DatabaseProvider::list_a2a_clients(self, user_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn deactivate_client(&self, client_id: &str) -> Result<(), DatabaseError> {
        DatabaseProvider::deactivate_a2a_client(self, client_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_client_credentials(
        &self,
        client_id: &str,
    ) -> Result<Option<(String, String)>, DatabaseError> {
        DatabaseProvider::get_a2a_client_credentials(self, client_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn invalidate_client_sessions(&self, client_id: &str) -> Result<(), DatabaseError> {
        DatabaseProvider::invalidate_a2a_client_sessions(self, client_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn deactivate_client_api_keys(&self, client_id: &str) -> Result<(), DatabaseError> {
        DatabaseProvider::deactivate_client_api_keys(self, client_id)
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
        DatabaseProvider::create_a2a_session(
            self,
            client_id,
            user_id,
            granted_scopes,
            expires_in_hours,
        )
        .await
        .map_err(app_error_to_db)
    }
    async fn get_session(&self, session_token: &str) -> Result<Option<A2ASession>, DatabaseError> {
        DatabaseProvider::get_a2a_session(self, session_token)
            .await
            .map_err(app_error_to_db)
    }
    async fn update_session_activity(&self, session_token: &str) -> Result<(), DatabaseError> {
        DatabaseProvider::update_a2a_session_activity(self, session_token)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_active_sessions(&self, client_id: &str) -> Result<Vec<A2ASession>, DatabaseError> {
        DatabaseProvider::get_active_a2a_sessions(self, client_id)
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
        DatabaseProvider::create_a2a_task(self, client_id, session_id, task_type, input_data)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_task(&self, task_id: &str) -> Result<Option<A2ATask>, DatabaseError> {
        DatabaseProvider::get_a2a_task(self, task_id)
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
        DatabaseProvider::list_a2a_tasks(self, client_id, status_filter, limit, offset)
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
        DatabaseProvider::update_a2a_task_status(self, task_id, status, result, error)
            .await
            .map_err(app_error_to_db)
    }
    async fn record_usage(&self, usage: &A2AUsage) -> Result<(), DatabaseError> {
        DatabaseProvider::record_a2a_usage(self, usage)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_client_current_usage(&self, client_id: &str) -> Result<u32, DatabaseError> {
        DatabaseProvider::get_a2a_client_current_usage(self, client_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_usage_stats(
        &self,
        client_id: &str,
        start_date: DateTime<Utc>,
        end_date: DateTime<Utc>,
    ) -> Result<A2AUsageStats, DatabaseError> {
        DatabaseProvider::get_a2a_usage_stats(self, client_id, start_date, end_date)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_client_usage_history(
        &self,
        client_id: &str,
        days: u32,
    ) -> Result<Vec<(DateTime<Utc>, u32, u32)>, DatabaseError> {
        DatabaseProvider::get_a2a_client_usage_history(self, client_id, days)
            .await
            .map_err(app_error_to_db)
    }
}

#[async_trait]
impl<T: DatabaseProvider> ProfileRepository for T {
    async fn upsert_profile(
        &self,
        user_id: Uuid,
        profile_data: Value,
    ) -> Result<(), DatabaseError> {
        DatabaseProvider::upsert_user_profile(self, user_id, profile_data)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_profile(&self, user_id: Uuid) -> Result<Option<Value>, DatabaseError> {
        DatabaseProvider::get_user_profile(self, user_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn create_goal(&self, user_id: Uuid, goal_data: Value) -> Result<String, DatabaseError> {
        DatabaseProvider::create_goal(self, user_id, goal_data)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_goals(&self, user_id: Uuid) -> Result<Vec<Value>, DatabaseError> {
        DatabaseProvider::get_user_goals(self, user_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn update_goal_progress(
        &self,
        goal_id: &str,
        user_id: Uuid,
        current_value: f64,
    ) -> Result<(), DatabaseError> {
        DatabaseProvider::update_goal_progress(self, goal_id, user_id, current_value)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_configuration(&self, user_id: &str) -> Result<Option<String>, DatabaseError> {
        DatabaseProvider::get_user_configuration(self, user_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn save_configuration(
        &self,
        user_id: &str,
        config_json: &str,
    ) -> Result<(), DatabaseError> {
        DatabaseProvider::save_user_configuration(self, user_id, config_json)
            .await
            .map_err(app_error_to_db)
    }
}

#[async_trait]
impl<T: DatabaseProvider> InsightRepository for T {
    async fn store(&self, user_id: Uuid, insight_data: Value) -> Result<String, DatabaseError> {
        DatabaseProvider::store_insight(self, user_id, insight_data)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_for_user(
        &self,
        user_id: Uuid,
        insight_type: Option<&str>,
        limit: Option<u32>,
    ) -> Result<Vec<Value>, DatabaseError> {
        DatabaseProvider::get_user_insights(self, user_id, insight_type, limit)
            .await
            .map_err(app_error_to_db)
    }
}

#[async_trait]
impl<T: DatabaseProvider> AdminRepository for T {
    async fn create_token(
        &self,
        request: &CreateAdminTokenRequest,
        admin_jwt_secret: &str,
        jwks_manager: &JwksManager,
    ) -> Result<GeneratedAdminToken, DatabaseError> {
        DatabaseProvider::create_admin_token(self, request, admin_jwt_secret, jwks_manager)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_token_by_id(&self, token_id: &str) -> Result<Option<AdminToken>, DatabaseError> {
        DatabaseProvider::get_admin_token_by_id(self, token_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_token_by_prefix(
        &self,
        token_prefix: &str,
    ) -> Result<Option<AdminToken>, DatabaseError> {
        DatabaseProvider::get_admin_token_by_prefix(self, token_prefix)
            .await
            .map_err(app_error_to_db)
    }
    async fn list_tokens(&self, include_inactive: bool) -> Result<Vec<AdminToken>, DatabaseError> {
        DatabaseProvider::list_admin_tokens(self, include_inactive)
            .await
            .map_err(app_error_to_db)
    }
    async fn deactivate_token(&self, token_id: &str) -> Result<(), DatabaseError> {
        DatabaseProvider::deactivate_admin_token(self, token_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn update_token_last_used(
        &self,
        token_id: &str,
        ip_address: Option<&str>,
    ) -> Result<(), DatabaseError> {
        DatabaseProvider::update_admin_token_last_used(self, token_id, ip_address)
            .await
            .map_err(app_error_to_db)
    }
    async fn record_token_usage(&self, usage: &AdminTokenUsage) -> Result<(), DatabaseError> {
        DatabaseProvider::record_admin_token_usage(self, usage)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_token_usage_history(
        &self,
        token_id: &str,
        start_date: DateTime<Utc>,
        end_date: DateTime<Utc>,
    ) -> Result<Vec<AdminTokenUsage>, DatabaseError> {
        DatabaseProvider::get_admin_token_usage_history(self, token_id, start_date, end_date)
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
        DatabaseProvider::record_admin_provisioned_key(
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
        DatabaseProvider::get_admin_provisioned_keys(self, admin_token_id, start_date, end_date)
            .await
            .map_err(app_error_to_db)
    }
}

#[async_trait]
impl<T: DatabaseProvider> TenantRepository for T {
    async fn create(&self, tenant: &Tenant) -> Result<(), DatabaseError> {
        DatabaseProvider::create_tenant(self, tenant)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_by_id(&self, tenant_id: TenantId) -> Result<Tenant, DatabaseError> {
        DatabaseProvider::get_tenant_by_id(self, tenant_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_by_slug(&self, slug: &str) -> Result<Tenant, DatabaseError> {
        DatabaseProvider::get_tenant_by_slug(self, slug)
            .await
            .map_err(app_error_to_db)
    }
    async fn list_for_user(&self, user_id: Uuid) -> Result<Vec<Tenant>, DatabaseError> {
        DatabaseProvider::list_tenants_for_user(self, user_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn store_oauth_credentials(
        &self,
        credentials: &TenantOAuthCredentials,
    ) -> Result<(), DatabaseError> {
        DatabaseProvider::store_tenant_oauth_credentials(self, credentials)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_oauth_providers(
        &self,
        tenant_id: TenantId,
    ) -> Result<Vec<TenantOAuthCredentials>, DatabaseError> {
        DatabaseProvider::get_tenant_oauth_providers(self, tenant_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_oauth_credentials(
        &self,
        tenant_id: TenantId,
        provider: &str,
    ) -> Result<Option<TenantOAuthCredentials>, DatabaseError> {
        DatabaseProvider::get_tenant_oauth_credentials(self, tenant_id, provider)
            .await
            .map_err(app_error_to_db)
    }
    async fn create_oauth_app(&self, app: &OAuthApp) -> Result<(), DatabaseError> {
        DatabaseProvider::create_oauth_app(self, app)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_oauth_app_by_client_id(&self, client_id: &str) -> Result<OAuthApp, DatabaseError> {
        DatabaseProvider::get_oauth_app_by_client_id(self, client_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn list_oauth_apps_for_user(
        &self,
        user_id: Uuid,
    ) -> Result<Vec<OAuthApp>, DatabaseError> {
        DatabaseProvider::list_oauth_apps_for_user(self, user_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_all(&self) -> Result<Vec<Tenant>, DatabaseError> {
        DatabaseProvider::get_all_tenants(self)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_user_role(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> Result<Option<String>, DatabaseError> {
        DatabaseProvider::get_user_tenant_role(self, user_id, tenant_id)
            .await
            .map_err(app_error_to_db)
    }
}

#[async_trait]
impl<T: DatabaseProvider> OAuth2ServerRepository for T {
    async fn store_client(&self, client: &OAuth2Client) -> Result<(), DatabaseError> {
        DatabaseProvider::store_oauth2_client(self, client)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_client(&self, client_id: &str) -> Result<Option<OAuth2Client>, DatabaseError> {
        DatabaseProvider::get_oauth2_client(self, client_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn store_auth_code(&self, auth_code: &OAuth2AuthCode) -> Result<(), DatabaseError> {
        DatabaseProvider::store_oauth2_auth_code(self, auth_code)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_auth_code(&self, code: &str) -> Result<Option<OAuth2AuthCode>, DatabaseError> {
        DatabaseProvider::get_oauth2_auth_code(self, code)
            .await
            .map_err(app_error_to_db)
    }
    async fn update_auth_code(&self, auth_code: &OAuth2AuthCode) -> Result<(), DatabaseError> {
        DatabaseProvider::update_oauth2_auth_code(self, auth_code)
            .await
            .map_err(app_error_to_db)
    }
    async fn store_refresh_token(
        &self,
        refresh_token: &OAuth2RefreshToken,
    ) -> Result<(), DatabaseError> {
        DatabaseProvider::store_oauth2_refresh_token(self, refresh_token)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_refresh_token(
        &self,
        token: &str,
    ) -> Result<Option<OAuth2RefreshToken>, DatabaseError> {
        DatabaseProvider::get_oauth2_refresh_token(self, token)
            .await
            .map_err(app_error_to_db)
    }
    async fn revoke_refresh_token(&self, token: &str) -> Result<(), DatabaseError> {
        DatabaseProvider::revoke_oauth2_refresh_token(self, token)
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
        DatabaseProvider::consume_auth_code(self, code, client_id, redirect_uri, now)
            .await
            .map_err(app_error_to_db)
    }
    async fn consume_refresh_token(
        &self,
        token: &str,
        client_id: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<OAuth2RefreshToken>, DatabaseError> {
        DatabaseProvider::consume_refresh_token(self, token, client_id, now)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_refresh_token_by_value(
        &self,
        token: &str,
    ) -> Result<Option<OAuth2RefreshToken>, DatabaseError> {
        DatabaseProvider::get_refresh_token_by_value(self, token)
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
        DatabaseProvider::store_authorization_code(
            self,
            code,
            client_id,
            redirect_uri,
            scope,
            user_id,
        )
        .await
        .map_err(app_error_to_db)
    }
    async fn get_authorization_code(&self, code: &str) -> Result<AuthorizationCode, DatabaseError> {
        DatabaseProvider::get_authorization_code(self, code)
            .await
            .map_err(app_error_to_db)
    }
    async fn delete_authorization_code(&self, code: &str) -> Result<(), DatabaseError> {
        DatabaseProvider::delete_authorization_code(self, code)
            .await
            .map_err(app_error_to_db)
    }
    async fn store_state(&self, state: &OAuth2State) -> Result<(), DatabaseError> {
        DatabaseProvider::store_oauth2_state(self, state)
            .await
            .map_err(app_error_to_db)
    }
    async fn consume_state(
        &self,
        state_value: &str,
        client_id: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<OAuth2State>, DatabaseError> {
        DatabaseProvider::consume_oauth2_state(self, state_value, client_id, now)
            .await
            .map_err(app_error_to_db)
    }
}

#[async_trait]
impl<T: DatabaseProvider> SecurityRepository for T {
    async fn save_rsa_keypair(
        &self,
        kid: &str,
        private_key_pem: &str,
        public_key_pem: &str,
        created_at: DateTime<Utc>,
        is_active: bool,
        key_size_bits: i32,
    ) -> Result<(), DatabaseError> {
        DatabaseProvider::save_rsa_keypair(
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
        DatabaseProvider::load_rsa_keypairs(self)
            .await
            .map_err(app_error_to_db)
    }
    async fn update_rsa_keypair_active_status(
        &self,
        kid: &str,
        is_active: bool,
    ) -> Result<(), DatabaseError> {
        DatabaseProvider::update_rsa_keypair_active_status(self, kid, is_active)
            .await
            .map_err(app_error_to_db)
    }
    async fn store_key_version(&self, version: &KeyVersion) -> Result<(), DatabaseError> {
        DatabaseProvider::store_key_version(self, version)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_key_versions(
        &self,
        tenant_id: Option<TenantId>,
    ) -> Result<Vec<KeyVersion>, DatabaseError> {
        DatabaseProvider::get_key_versions(self, tenant_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_current_key_version(
        &self,
        tenant_id: Option<TenantId>,
    ) -> Result<Option<KeyVersion>, DatabaseError> {
        DatabaseProvider::get_current_key_version(self, tenant_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn update_key_version_status(
        &self,
        tenant_id: Option<TenantId>,
        version: u32,
        is_active: bool,
    ) -> Result<(), DatabaseError> {
        DatabaseProvider::update_key_version_status(self, tenant_id, version, is_active)
            .await
            .map_err(app_error_to_db)
    }
    async fn delete_old_key_versions(
        &self,
        tenant_id: Option<TenantId>,
        keep_count: u32,
    ) -> Result<u64, DatabaseError> {
        DatabaseProvider::delete_old_key_versions(self, tenant_id, keep_count)
            .await
            .map_err(app_error_to_db)
    }
    async fn store_audit_event(&self, event: &AuditEvent) -> Result<(), DatabaseError> {
        DatabaseProvider::store_audit_event(self, event)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_audit_events(
        &self,
        tenant_id: Option<TenantId>,
        event_type: Option<&str>,
        limit: Option<u32>,
    ) -> Result<Vec<AuditEvent>, DatabaseError> {
        DatabaseProvider::get_audit_events(self, tenant_id, event_type, limit)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_or_create_system_secret(
        &self,
        secret_type: &str,
    ) -> Result<String, DatabaseError> {
        DatabaseProvider::get_or_create_system_secret(self, secret_type)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_system_secret(&self, secret_type: &str) -> Result<String, DatabaseError> {
        DatabaseProvider::get_system_secret(self, secret_type)
            .await
            .map_err(app_error_to_db)
    }
    async fn update_system_secret(
        &self,
        secret_type: &str,
        new_value: &str,
    ) -> Result<(), DatabaseError> {
        DatabaseProvider::update_system_secret(self, secret_type, new_value)
            .await
            .map_err(app_error_to_db)
    }
    fn encrypt_data_with_aad(&self, data: &str, aad: &str) -> Result<String, DatabaseError> {
        DatabaseProvider::encrypt_data_with_aad(self, data, aad).map_err(app_error_to_db)
    }
    fn decrypt_data_with_aad(&self, encrypted: &str, aad: &str) -> Result<String, DatabaseError> {
        DatabaseProvider::decrypt_data_with_aad(self, encrypted, aad).map_err(app_error_to_db)
    }
}

#[async_trait]
impl<T: DatabaseProvider> NotificationRepository for T {
    async fn store(
        &self,
        user_id: Uuid,
        provider: &str,
        success: bool,
        message: &str,
        expires_at: Option<&str>,
    ) -> Result<String, DatabaseError> {
        DatabaseProvider::store_oauth_notification(
            self, user_id, provider, success, message, expires_at,
        )
        .await
        .map_err(app_error_to_db)
    }
    async fn get_unread(&self, user_id: Uuid) -> Result<Vec<OAuthNotification>, DatabaseError> {
        DatabaseProvider::get_unread_oauth_notifications(self, user_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn mark_read(&self, notification_id: &str, user_id: Uuid) -> Result<bool, DatabaseError> {
        DatabaseProvider::mark_oauth_notification_read(self, notification_id, user_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn mark_all_read(&self, user_id: Uuid) -> Result<u64, DatabaseError> {
        DatabaseProvider::mark_all_oauth_notifications_read(self, user_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_all(
        &self,
        user_id: Uuid,
        limit: Option<i64>,
    ) -> Result<Vec<OAuthNotification>, DatabaseError> {
        DatabaseProvider::get_all_oauth_notifications(self, user_id, limit)
            .await
            .map_err(app_error_to_db)
    }
}

#[async_trait]
impl<T: DatabaseProvider> FitnessConfigRepository for T {
    async fn save_tenant_config(
        &self,
        tenant_id: TenantId,
        configuration_name: &str,
        config: &FitnessConfig,
    ) -> Result<String, DatabaseError> {
        DatabaseProvider::save_tenant_fitness_config(self, tenant_id, configuration_name, config)
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
        DatabaseProvider::save_user_fitness_config(
            self,
            tenant_id,
            user_id,
            configuration_name,
            config,
        )
        .await
        .map_err(app_error_to_db)
    }
    async fn get_tenant_config(
        &self,
        tenant_id: TenantId,
        configuration_name: &str,
    ) -> Result<Option<FitnessConfig>, DatabaseError> {
        DatabaseProvider::get_tenant_fitness_config(self, tenant_id, configuration_name)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_user_config(
        &self,
        tenant_id: TenantId,
        user_id: &str,
        configuration_name: &str,
    ) -> Result<Option<FitnessConfig>, DatabaseError> {
        DatabaseProvider::get_user_fitness_config(self, tenant_id, user_id, configuration_name)
            .await
            .map_err(app_error_to_db)
    }
    async fn list_tenant_configurations(
        &self,
        tenant_id: TenantId,
    ) -> Result<Vec<String>, DatabaseError> {
        DatabaseProvider::list_tenant_fitness_configurations(self, tenant_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn list_user_configurations(
        &self,
        tenant_id: TenantId,
        user_id: &str,
    ) -> Result<Vec<String>, DatabaseError> {
        DatabaseProvider::list_user_fitness_configurations(self, tenant_id, user_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn delete_config(
        &self,
        tenant_id: TenantId,
        user_id: Option<&str>,
        configuration_name: &str,
    ) -> Result<bool, DatabaseError> {
        DatabaseProvider::delete_fitness_config(self, tenant_id, user_id, configuration_name)
            .await
            .map_err(app_error_to_db)
    }
}

#[async_trait]
impl<T: DatabaseProvider> ChatRepository for T {
    async fn create_conversation(
        &self,
        user_id: &str,
        tenant_id: TenantId,
        title: &str,
        model: &str,
        system_prompt: Option<&str>,
    ) -> Result<ConversationRecord, DatabaseError> {
        DatabaseProvider::chat_create_conversation(
            self,
            user_id,
            tenant_id,
            title,
            model,
            system_prompt,
        )
        .await
        .map_err(app_error_to_db)
    }
    async fn get_conversation(
        &self,
        conversation_id: &str,
        user_id: &str,
        tenant_id: TenantId,
    ) -> Result<Option<ConversationRecord>, DatabaseError> {
        DatabaseProvider::chat_get_conversation(self, conversation_id, user_id, tenant_id)
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
        DatabaseProvider::chat_list_conversations(self, user_id, tenant_id, limit, offset)
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
        DatabaseProvider::chat_update_conversation_title(
            self,
            conversation_id,
            user_id,
            tenant_id,
            title,
        )
        .await
        .map_err(app_error_to_db)
    }
    async fn delete_conversation(
        &self,
        conversation_id: &str,
        user_id: &str,
        tenant_id: TenantId,
    ) -> Result<bool, DatabaseError> {
        DatabaseProvider::chat_delete_conversation(self, conversation_id, user_id, tenant_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn add_message(
        &self,
        conversation_id: &str,
        user_id: &str,
        role: &str,
        content: &str,
        token_count: Option<u32>,
        finish_reason: Option<&str>,
    ) -> Result<MessageRecord, DatabaseError> {
        DatabaseProvider::chat_add_message(
            self,
            conversation_id,
            user_id,
            role,
            content,
            token_count,
            finish_reason,
        )
        .await
        .map_err(app_error_to_db)
    }
    async fn get_messages(
        &self,
        conversation_id: &str,
        user_id: &str,
    ) -> Result<Vec<MessageRecord>, DatabaseError> {
        DatabaseProvider::chat_get_messages(self, conversation_id, user_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_recent_messages(
        &self,
        conversation_id: &str,
        user_id: &str,
        limit: i64,
    ) -> Result<Vec<MessageRecord>, DatabaseError> {
        DatabaseProvider::chat_get_recent_messages(self, conversation_id, user_id, limit)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_message_count(
        &self,
        conversation_id: &str,
        user_id: &str,
    ) -> Result<i64, DatabaseError> {
        DatabaseProvider::chat_get_message_count(self, conversation_id, user_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn delete_all_user_conversations(
        &self,
        user_id: &str,
        tenant_id: TenantId,
    ) -> Result<i64, DatabaseError> {
        DatabaseProvider::chat_delete_all_user_conversations(self, user_id, tenant_id)
            .await
            .map_err(app_error_to_db)
    }
}

#[async_trait]
impl<T: DatabaseProvider> UserMcpTokenRepository for T {
    async fn create_token(
        &self,
        user_id: Uuid,
        request: &CreateUserMcpTokenRequest,
    ) -> Result<UserMcpTokenCreated, DatabaseError> {
        DatabaseProvider::create_user_mcp_token(self, user_id, request)
            .await
            .map_err(app_error_to_db)
    }
    async fn validate_token(&self, token_value: &str) -> Result<Uuid, DatabaseError> {
        DatabaseProvider::validate_user_mcp_token(self, token_value)
            .await
            .map_err(app_error_to_db)
    }
    async fn list_tokens(&self, user_id: Uuid) -> Result<Vec<UserMcpTokenInfo>, DatabaseError> {
        DatabaseProvider::list_user_mcp_tokens(self, user_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn revoke_token(&self, token_id: &str, user_id: Uuid) -> Result<(), DatabaseError> {
        DatabaseProvider::revoke_user_mcp_token(self, token_id, user_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_token(
        &self,
        token_id: &str,
        user_id: Uuid,
    ) -> Result<Option<UserMcpToken>, DatabaseError> {
        DatabaseProvider::get_user_mcp_token(self, token_id, user_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn cleanup_expired_tokens(&self) -> Result<u64, DatabaseError> {
        DatabaseProvider::cleanup_expired_user_mcp_tokens(self)
            .await
            .map_err(app_error_to_db)
    }
}

#[async_trait]
impl<T: DatabaseProvider> ImpersonationRepository for T {
    async fn create_session(&self, session: &ImpersonationSession) -> Result<(), DatabaseError> {
        DatabaseProvider::create_impersonation_session(self, session)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_session(
        &self,
        session_id: &str,
    ) -> Result<Option<ImpersonationSession>, DatabaseError> {
        DatabaseProvider::get_impersonation_session(self, session_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_active_session(
        &self,
        user_id: Uuid,
    ) -> Result<Option<ImpersonationSession>, DatabaseError> {
        DatabaseProvider::get_active_impersonation_session(self, user_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn end_session(&self, session_id: &str) -> Result<(), DatabaseError> {
        DatabaseProvider::end_impersonation_session(self, session_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn end_all_sessions(&self, impersonator_id: Uuid) -> Result<u64, DatabaseError> {
        DatabaseProvider::end_all_impersonation_sessions(self, impersonator_id)
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
        DatabaseProvider::list_impersonation_sessions(
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
impl<T: DatabaseProvider> LlmCredentialRepository for T {
    async fn store_credentials(&self, record: &LlmCredentialRecord) -> Result<(), DatabaseError> {
        DatabaseProvider::store_llm_credentials(self, record)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_credentials(
        &self,
        tenant_id: TenantId,
        user_id: Option<Uuid>,
        provider: &str,
    ) -> Result<Option<LlmCredentialRecord>, DatabaseError> {
        DatabaseProvider::get_llm_credentials(self, tenant_id, user_id, provider)
            .await
            .map_err(app_error_to_db)
    }
    async fn list_credentials(
        &self,
        tenant_id: TenantId,
    ) -> Result<Vec<LlmCredentialSummary>, DatabaseError> {
        DatabaseProvider::list_llm_credentials(self, tenant_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn delete_credentials(
        &self,
        tenant_id: TenantId,
        user_id: Option<Uuid>,
        provider: &str,
    ) -> Result<bool, DatabaseError> {
        DatabaseProvider::delete_llm_credentials(self, tenant_id, user_id, provider)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_admin_config_override(
        &self,
        config_key: &str,
        tenant_id: Option<TenantId>,
    ) -> Result<Option<String>, DatabaseError> {
        DatabaseProvider::get_admin_config_override(self, config_key, tenant_id)
            .await
            .map_err(app_error_to_db)
    }
}

#[async_trait]
impl<T: DatabaseProvider> ProviderConnectionRepository for T {
    async fn register_connection(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        connection_type: &ConnectionType,
        metadata: Option<&str>,
    ) -> Result<(), DatabaseError> {
        DatabaseProvider::register_provider_connection(
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
        DatabaseProvider::remove_provider_connection(self, user_id, tenant_id, provider)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_for_user(
        &self,
        user_id: Uuid,
        tenant_id: Option<TenantId>,
    ) -> Result<Vec<ProviderConnection>, DatabaseError> {
        DatabaseProvider::get_user_provider_connections(self, user_id, tenant_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn is_connected(&self, user_id: Uuid, provider: &str) -> Result<bool, DatabaseError> {
        DatabaseProvider::is_provider_connected(self, user_id, provider)
            .await
            .map_err(app_error_to_db)
    }
}

#[async_trait]
impl<T: DatabaseProvider> PasswordResetRepository for T {
    async fn store_token(
        &self,
        user_id: Uuid,
        token_hash: &str,
        created_by: &str,
    ) -> Result<Uuid, DatabaseError> {
        DatabaseProvider::store_password_reset_token(self, user_id, token_hash, created_by)
            .await
            .map_err(app_error_to_db)
    }
    async fn consume_token(&self, token_hash: &str) -> Result<Uuid, DatabaseError> {
        DatabaseProvider::consume_password_reset_token(self, token_hash)
            .await
            .map_err(app_error_to_db)
    }
    async fn invalidate_tokens(&self, user_id: Uuid) -> Result<(), DatabaseError> {
        DatabaseProvider::invalidate_user_reset_tokens(self, user_id)
            .await
            .map_err(app_error_to_db)
    }
}

#[async_trait]
impl<T: DatabaseProvider> OAuthClientStateRepository for T {
    async fn store_oauth_client_state(
        &self,
        state: &OAuthClientState,
    ) -> Result<(), DatabaseError> {
        DatabaseProvider::store_oauth_client_state(self, state)
            .await
            .map_err(app_error_to_db)
    }
    async fn consume_oauth_client_state(
        &self,
        state_value: &str,
        provider: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<OAuthClientState>, DatabaseError> {
        DatabaseProvider::consume_oauth_client_state(self, state_value, provider, now)
            .await
            .map_err(app_error_to_db)
    }
}

#[async_trait]
impl<T: DatabaseProvider> ToolSelectionRepository for T {
    async fn get_tool_catalog(&self) -> Result<Vec<ToolCatalogEntry>, DatabaseError> {
        DatabaseProvider::get_tool_catalog(self)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_tool_catalog_entry(
        &self,
        tool_name: &str,
    ) -> Result<Option<ToolCatalogEntry>, DatabaseError> {
        DatabaseProvider::get_tool_catalog_entry(self, tool_name)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_tools_by_category(
        &self,
        category: ToolCategory,
    ) -> Result<Vec<ToolCatalogEntry>, DatabaseError> {
        DatabaseProvider::get_tools_by_category(self, category)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_tools_by_min_plan(
        &self,
        plan: TenantPlan,
    ) -> Result<Vec<ToolCatalogEntry>, DatabaseError> {
        DatabaseProvider::get_tools_by_min_plan(self, plan)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_overrides(
        &self,
        tenant_id: TenantId,
    ) -> Result<Vec<TenantToolOverride>, DatabaseError> {
        DatabaseProvider::get_tenant_tool_overrides(self, tenant_id)
            .await
            .map_err(app_error_to_db)
    }
    async fn get_override(
        &self,
        tenant_id: TenantId,
        tool_name: &str,
    ) -> Result<Option<TenantToolOverride>, DatabaseError> {
        DatabaseProvider::get_tenant_tool_override(self, tenant_id, tool_name)
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
        DatabaseProvider::upsert_tenant_tool_override(
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
        DatabaseProvider::delete_tenant_tool_override(self, tenant_id, tool_name)
            .await
            .map_err(app_error_to_db)
    }
    async fn count_enabled_tools(&self, tenant_id: TenantId) -> Result<usize, DatabaseError> {
        DatabaseProvider::count_enabled_tools(self, tenant_id)
            .await
            .map_err(app_error_to_db)
    }
}
