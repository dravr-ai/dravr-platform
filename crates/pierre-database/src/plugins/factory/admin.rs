// ABOUTME: Admin repository dispatch for the database factory
// ABOUTME: Delegates AdminRepository, ImpersonationRepository, and UserMcpTokenRepository calls
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::Database;
use crate::database::{
    CreateUserMcpTokenRequest, UserMcpToken, UserMcpTokenCreated, UserMcpTokenInfo,
};
use crate::plugins::{AdminRepository, ImpersonationRepository, UserMcpTokenRepository};
use async_trait::async_trait;
use pierre_core::admin::jwt::JwtSigner;
use pierre_core::admin::models::{
    AdminToken, AdminTokenUsage, CreateAdminTokenRequest, GeneratedAdminToken,
};
use pierre_core::errors::AppResult;
use pierre_core::permissions::impersonation::ImpersonationSession;
use uuid::Uuid;

#[async_trait]
impl AdminRepository for Database {
    async fn create_token(
        &self,
        request: &CreateAdminTokenRequest,
        admin_jwt_secret: &str,
        jwks_manager: &dyn JwtSigner,
    ) -> AppResult<GeneratedAdminToken> {
        match self {
            Self::SQLite(db) => {
                AdminRepository::create_token(db, request, admin_jwt_secret, jwks_manager).await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                AdminRepository::create_token(db, request, admin_jwt_secret, jwks_manager).await
            }
        }
    }
    async fn get_token_by_id(&self, token_id: &str) -> AppResult<Option<AdminToken>> {
        match self {
            Self::SQLite(db) => db.get_token_by_id(token_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_token_by_id(token_id).await,
        }
    }
    async fn get_token_by_prefix(&self, token_prefix: &str) -> AppResult<Option<AdminToken>> {
        match self {
            Self::SQLite(db) => db.get_token_by_prefix(token_prefix).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_token_by_prefix(token_prefix).await,
        }
    }
    async fn list_tokens(&self, include_inactive: bool) -> AppResult<Vec<AdminToken>> {
        match self {
            Self::SQLite(db) => AdminRepository::list_tokens(db, include_inactive).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => AdminRepository::list_tokens(db, include_inactive).await,
        }
    }
    async fn deactivate_token(&self, token_id: &str) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.deactivate_token(token_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.deactivate_token(token_id).await,
        }
    }
    async fn update_token_last_used(
        &self,
        token_id: &str,
        ip_address: Option<&str>,
    ) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.update_token_last_used(token_id, ip_address).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.update_token_last_used(token_id, ip_address).await,
        }
    }
    async fn record_token_usage(&self, usage: &AdminTokenUsage) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.record_token_usage(usage).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.record_token_usage(usage).await,
        }
    }
    async fn get_token_usage_history(
        &self,
        token_id: &str,
        start_date: chrono::DateTime<chrono::Utc>,
        end_date: chrono::DateTime<chrono::Utc>,
    ) -> AppResult<Vec<AdminTokenUsage>> {
        match self {
            Self::SQLite(db) => {
                db.get_token_usage_history(token_id, start_date, end_date)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                db.get_token_usage_history(token_id, start_date, end_date)
                    .await
            }
        }
    }
    async fn record_provisioned_key(
        &self,
        admin_token_id: &str,
        api_key_id: &str,
        user_email: &str,
        tier: &str,
        rate_limit_requests: u32,
        rate_limit_period: &str,
    ) -> AppResult<()> {
        match self {
            Self::SQLite(db) => {
                db.record_provisioned_key(
                    admin_token_id,
                    api_key_id,
                    user_email,
                    tier,
                    rate_limit_requests,
                    rate_limit_period,
                )
                .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                db.record_provisioned_key(
                    admin_token_id,
                    api_key_id,
                    user_email,
                    tier,
                    rate_limit_requests,
                    rate_limit_period,
                )
                .await
            }
        }
    }
    async fn get_provisioned_keys(
        &self,
        admin_token_id: Option<&str>,
        start_date: chrono::DateTime<chrono::Utc>,
        end_date: chrono::DateTime<chrono::Utc>,
    ) -> AppResult<Vec<serde_json::Value>> {
        match self {
            Self::SQLite(db) => {
                db.get_provisioned_keys(admin_token_id, start_date, end_date)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                db.get_provisioned_keys(admin_token_id, start_date, end_date)
                    .await
            }
        }
    }
}

#[async_trait]
impl ImpersonationRepository for Database {
    async fn create_session(&self, session: &ImpersonationSession) -> AppResult<()> {
        match self {
            Self::SQLite(db) => ImpersonationRepository::create_session(db, session).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => ImpersonationRepository::create_session(db, session).await,
        }
    }

    async fn get_session(&self, session_id: &str) -> AppResult<Option<ImpersonationSession>> {
        match self {
            Self::SQLite(db) => ImpersonationRepository::get_session(db, session_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => ImpersonationRepository::get_session(db, session_id).await,
        }
    }

    async fn get_active_session(&self, user_id: Uuid) -> AppResult<Option<ImpersonationSession>> {
        match self {
            Self::SQLite(db) => ImpersonationRepository::get_active_session(db, user_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => ImpersonationRepository::get_active_session(db, user_id).await,
        }
    }

    async fn end_session(&self, session_id: &str) -> AppResult<()> {
        match self {
            Self::SQLite(db) => ImpersonationRepository::end_session(db, session_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => ImpersonationRepository::end_session(db, session_id).await,
        }
    }

    async fn end_all_sessions(&self, impersonator_id: Uuid) -> AppResult<u64> {
        match self {
            Self::SQLite(db) => db.end_all_sessions(impersonator_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.end_all_sessions(impersonator_id).await,
        }
    }

    async fn list_sessions(
        &self,
        impersonator_id: Option<Uuid>,
        target_user_id: Option<Uuid>,
        active_only: bool,
        limit: u32,
    ) -> AppResult<Vec<ImpersonationSession>> {
        match self {
            Self::SQLite(db) => {
                ImpersonationRepository::list_sessions(
                    db,
                    impersonator_id,
                    target_user_id,
                    active_only,
                    limit,
                )
                .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                ImpersonationRepository::list_sessions(
                    db,
                    impersonator_id,
                    target_user_id,
                    active_only,
                    limit,
                )
                .await
            }
        }
    }
}

#[async_trait]
impl UserMcpTokenRepository for Database {
    async fn create_token(
        &self,
        user_id: Uuid,
        request: &CreateUserMcpTokenRequest,
    ) -> AppResult<UserMcpTokenCreated> {
        match self {
            Self::SQLite(db) => UserMcpTokenRepository::create_token(db, user_id, request).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                UserMcpTokenRepository::create_token(db, user_id, request).await
            }
        }
    }

    async fn validate_token(&self, token_value: &str) -> AppResult<Uuid> {
        match self {
            Self::SQLite(db) => db.validate_token(token_value).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.validate_token(token_value).await,
        }
    }

    async fn list_tokens(&self, user_id: Uuid) -> AppResult<Vec<UserMcpTokenInfo>> {
        match self {
            Self::SQLite(db) => UserMcpTokenRepository::list_tokens(db, user_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => UserMcpTokenRepository::list_tokens(db, user_id).await,
        }
    }

    async fn revoke_token(&self, token_id: &str, user_id: Uuid) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.revoke_token(token_id, user_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.revoke_token(token_id, user_id).await,
        }
    }

    async fn get_token(&self, token_id: &str, user_id: Uuid) -> AppResult<Option<UserMcpToken>> {
        match self {
            Self::SQLite(db) => UserMcpTokenRepository::get_token(db, token_id, user_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => UserMcpTokenRepository::get_token(db, token_id, user_id).await,
        }
    }

    async fn cleanup_expired_tokens(&self) -> AppResult<u64> {
        match self {
            Self::SQLite(db) => db.cleanup_expired_tokens().await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.cleanup_expired_tokens().await,
        }
    }
}
