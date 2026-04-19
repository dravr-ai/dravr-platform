// ABOUTME: PostgreSQL admin, impersonation, and MCP token repository implementations
// ABOUTME: Handles admin token management, impersonation sessions, and user MCP tokens
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::super::{AdminRepository, ImpersonationRepository, UserMcpTokenRepository};
use super::PostgresDatabase;
use crate::database::{
    CreateUserMcpTokenRequest, UserMcpToken, UserMcpTokenCreated, UserMcpTokenInfo,
};
use crate::plugins::shared;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::admin::jwt::JwtSigner;
use pierre_core::admin::models::{
    AdminPermissions, AdminToken, AdminTokenUsage, CreateAdminTokenRequest, GeneratedAdminToken,
};
use pierre_core::admin::{AdminJwtManager, TokenScope};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::permissions::impersonation::ImpersonationSession;
use sqlx::Row;
use tracing::debug;
use uuid::Uuid;

#[async_trait]
impl AdminRepository for PostgresDatabase {
    // ================================
    // Admin Token Management (PostgreSQL)
    // ================================

    async fn create_token(
        &self,
        request: &CreateAdminTokenRequest,
        admin_jwt_secret: &str,
        jwks_manager: &dyn JwtSigner,
    ) -> AppResult<GeneratedAdminToken> {
        use uuid::Uuid;

        let uuid = Uuid::new_v4().simple();
        let token_id = format!("admin_{uuid}");

        debug!("Creating admin token with RS256 asymmetric signing");

        let jwt_manager = AdminJwtManager::new();

        let permissions = request.permissions.as_ref().map_or_else(
            || {
                if request.is_super_admin {
                    AdminPermissions::super_admin()
                } else {
                    AdminPermissions::default_admin()
                }
            },
            |perms| AdminPermissions::new(perms.clone()),
        );

        let expires_at = request.expires_in_days.map(|days| {
            chrono::Utc::now() + chrono::Duration::days(i64::try_from(days).unwrap_or(365))
        });

        let jwt_token = jwt_manager.generate_token(
            &token_id,
            &request.service_name,
            &permissions,
            &TokenScope {
                is_super_admin: request.is_super_admin,
                expires_at,
                tenant_id: request.tenant_id.as_deref(),
            },
            jwks_manager,
        )?;

        let token_prefix = AdminJwtManager::generate_token_prefix(&jwt_token);
        let token_hash = AdminJwtManager::hash_token_for_storage(&jwt_token)?;
        let jwt_secret_hash = AdminJwtManager::hash_secret(admin_jwt_secret);

        let query = r"
            INSERT INTO admin_tokens (
                id, service_name, service_description, token_hash, token_prefix,
                jwt_secret_hash, permissions, is_super_admin, is_active,
                tenant_id, created_at, expires_at, usage_count
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
        ";

        let permissions_json = permissions.to_json()?;
        let created_at = chrono::Utc::now();

        sqlx::query(query)
            .bind(&token_id)
            .bind(&request.service_name)
            .bind(&request.service_description)
            .bind(&token_hash)
            .bind(&token_prefix)
            .bind(&jwt_secret_hash)
            .bind(&permissions_json)
            .bind(request.is_super_admin)
            .bind(true)
            .bind(&request.tenant_id)
            .bind(created_at)
            .bind(expires_at)
            .bind(0i64)
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Database operation failed: {e}")))?;

        Ok(GeneratedAdminToken {
            token_id,
            service_name: request.service_name.clone(),
            jwt_token,
            token_prefix,
            permissions,
            is_super_admin: request.is_super_admin,
            expires_at,
            created_at,
        })
    }

    async fn get_token_by_id(&self, token_id: &str) -> AppResult<Option<AdminToken>> {
        let query = r"
            SELECT id, service_name, service_description, token_hash, token_prefix,
                   jwt_secret_hash, permissions, is_super_admin, is_active,
                   tenant_id, created_at, expires_at, last_used_at, last_used_ip, usage_count
            FROM admin_tokens WHERE id = $1
        ";

        let row = sqlx::query(query)
            .bind(token_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to fetch optional record: {e}")))?;

        if let Some(row) = row {
            Ok(Some(shared::mappers::parse_admin_token_from_row(&row)?))
        } else {
            Ok(None)
        }
    }

    async fn get_token_by_prefix(&self, token_prefix: &str) -> AppResult<Option<AdminToken>> {
        let query = r"
            SELECT id, service_name, service_description, token_hash, token_prefix,
                   jwt_secret_hash, permissions, is_super_admin, is_active,
                   tenant_id, created_at, expires_at, last_used_at, last_used_ip, usage_count
            FROM admin_tokens WHERE token_prefix = $1
        ";

        let row = sqlx::query(query)
            .bind(token_prefix)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to fetch optional record: {e}")))?;

        if let Some(row) = row {
            Ok(Some(shared::mappers::parse_admin_token_from_row(&row)?))
        } else {
            Ok(None)
        }
    }

    async fn list_tokens(&self, include_inactive: bool) -> AppResult<Vec<AdminToken>> {
        let query = if include_inactive {
            r"
                SELECT id, service_name, service_description, token_hash, token_prefix,
                       jwt_secret_hash, permissions, is_super_admin, is_active,
                       tenant_id, created_at, expires_at, last_used_at, last_used_ip, usage_count
                FROM admin_tokens ORDER BY created_at DESC
            "
        } else {
            r"
                SELECT id, service_name, service_description, token_hash, token_prefix,
                       jwt_secret_hash, permissions, is_super_admin, is_active,
                       tenant_id, created_at, expires_at, last_used_at, last_used_ip, usage_count
                FROM admin_tokens WHERE is_active = true ORDER BY created_at DESC
            "
        };

        let rows = sqlx::query(query)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to fetch records: {e}")))?;

        let mut tokens = Vec::with_capacity(rows.len());
        for row in rows {
            tokens.push(shared::mappers::parse_admin_token_from_row(&row)?);
        }

        Ok(tokens)
    }

    async fn deactivate_token(&self, token_id: &str) -> AppResult<()> {
        let query = "UPDATE admin_tokens SET is_active = false WHERE id = $1";

        sqlx::query(query)
            .bind(token_id)
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Database operation failed: {e}")))?;

        Ok(())
    }

    async fn update_token_last_used(
        &self,
        token_id: &str,
        ip_address: Option<&str>,
    ) -> AppResult<()> {
        let query = r"
            UPDATE admin_tokens 
            SET last_used_at = CURRENT_TIMESTAMP, last_used_ip = $1, usage_count = usage_count + 1
            WHERE id = $2
        ";

        sqlx::query(query)
            .bind(ip_address)
            .bind(token_id)
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Database operation failed: {e}")))?;

        Ok(())
    }

    async fn record_token_usage(&self, usage: &AdminTokenUsage) -> AppResult<()> {
        let query = r"
            INSERT INTO admin_token_usage (
                admin_token_id, timestamp, action, target_resource,
                ip_address, user_agent, request_size_bytes, success,
                method, response_time_ms
            ) VALUES ($1, $2, $3, $4, $5::inet, $6, $7, $8, $9, $10)
        ";

        sqlx::query(query)
            .bind(&usage.admin_token_id)
            .bind(usage.timestamp)
            .bind(usage.action.to_string())
            .bind(&usage.target_resource)
            .bind(&usage.ip_address)
            .bind(&usage.user_agent)
            .bind(
                usage
                    .request_size_bytes
                    .map(|x| i32::try_from(x).unwrap_or(i32::MAX)),
            )
            .bind(usage.success)
            .bind(None::<String>)
            .bind(
                usage
                    .response_time_ms
                    .map(|x| i32::try_from(x).unwrap_or(i32::MAX)),
            )
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Database operation failed: {e}")))?;

        Ok(())
    }

    async fn get_token_usage_history(
        &self,
        token_id: &str,
        start_date: DateTime<Utc>,
        end_date: DateTime<Utc>,
    ) -> AppResult<Vec<AdminTokenUsage>> {
        let query = r"
            SELECT id, admin_token_id, timestamp, action, target_resource,
                   ip_address, user_agent, request_size_bytes, success,
                   method, response_time_ms
            FROM admin_token_usage 
            WHERE admin_token_id = $1 AND timestamp BETWEEN $2 AND $3
            ORDER BY timestamp DESC
        ";

        let rows = sqlx::query(query)
            .bind(token_id)
            .bind(start_date)
            .bind(end_date)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to fetch records: {e}")))?;

        let mut usage_history = Vec::new();
        for row in rows {
            usage_history.push(shared::mappers::parse_admin_token_usage_from_row(&row)?);
        }

        Ok(usage_history)
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
        let query = r"
            INSERT INTO admin_provisioned_keys (
                admin_token_id, api_key_id, user_email, requested_tier,
                provisioned_at, provisioned_by_service, rate_limit_requests,
                rate_limit_period, key_status
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
        ";

        // Get service name from admin token
        let service_name = if let Some(token) = self.get_token_by_id(admin_token_id).await? {
            token.service_name
        } else {
            "unknown".into()
        };

        sqlx::query(query)
            .bind(admin_token_id)
            .bind(api_key_id)
            .bind(user_email)
            .bind(tier)
            .bind(chrono::Utc::now())
            .bind(service_name)
            .bind(i32::try_from(rate_limit_requests).unwrap_or(i32::MAX))
            .bind(rate_limit_period)
            .bind("active")
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Database operation failed: {e}")))?;

        Ok(())
    }

    async fn get_provisioned_keys(
        &self,
        admin_token_id: Option<&str>,
        start_date: DateTime<Utc>,
        end_date: DateTime<Utc>,
    ) -> AppResult<Vec<serde_json::Value>> {
        // Simplified implementation using direct queries instead of complex dynamic binding
        if let Some(token_id) = admin_token_id {
            let rows = sqlx::query(
                r"
                    SELECT id, admin_token_id, api_key_id, user_email, requested_tier,
                           provisioned_at, provisioned_by_service, rate_limit_requests,
                           rate_limit_period, key_status, revoked_at, revoked_reason
                    FROM admin_provisioned_keys 
                    WHERE admin_token_id = $1 AND provisioned_at BETWEEN $2 AND $3
                    ORDER BY provisioned_at DESC
                ",
            )
            .bind(token_id)
            .bind(start_date)
            .bind(end_date)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to fetch records: {e}")))?;

            let mut results = Vec::new();
            for row in rows {
                let result = serde_json::json!({
                    "id": row.get::<i32, _>("id"),
                    "admin_token_id": row.get::<String, _>("admin_token_id"),
                    "api_key_id": row.get::<String, _>("api_key_id"),
                    "user_email": row.get::<String, _>("user_email"),
                    "requested_tier": row.get::<String, _>("requested_tier"),
                    "provisioned_at": row.get::<DateTime<Utc>, _>("provisioned_at"),
                    "provisioned_by_service": row.get::<String, _>("provisioned_by_service"),
                    "rate_limit_requests": row.get::<i32, _>("rate_limit_requests"),
                    "rate_limit_period": row.get::<String, _>("rate_limit_period"),
                    "key_status": row.get::<String, _>("key_status"),
                    "revoked_at": row.get::<Option<DateTime<Utc>>, _>("revoked_at"),
                    "revoked_reason": row.get::<Option<String>, _>("revoked_reason"),
                });
                results.push(result);
            }
            Ok(results)
        } else {
            let rows = sqlx::query(
                r"
                    SELECT id, admin_token_id, api_key_id, user_email, requested_tier,
                           provisioned_at, provisioned_by_service, rate_limit_requests,
                           rate_limit_period, key_status, revoked_at, revoked_reason
                    FROM admin_provisioned_keys 
                    WHERE provisioned_at BETWEEN $1 AND $2
                    ORDER BY provisioned_at DESC
                ",
            )
            .bind(start_date)
            .bind(end_date)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to fetch records: {e}")))?;

            let mut results = Vec::new();
            for row in rows {
                let result = serde_json::json!({
                    "id": row.get::<i32, _>("id"),
                    "admin_token_id": row.get::<String, _>("admin_token_id"),
                    "api_key_id": row.get::<String, _>("api_key_id"),
                    "user_email": row.get::<String, _>("user_email"),
                    "requested_tier": row.get::<String, _>("requested_tier"),
                    "provisioned_at": row.get::<DateTime<Utc>, _>("provisioned_at"),
                    "provisioned_by_service": row.get::<String, _>("provisioned_by_service"),
                    "rate_limit_requests": row.get::<i32, _>("rate_limit_requests"),
                    "rate_limit_period": row.get::<String, _>("rate_limit_period"),
                    "key_status": row.get::<String, _>("key_status"),
                    "revoked_at": row.get::<Option<DateTime<Utc>>, _>("revoked_at"),
                    "revoked_reason": row.get::<Option<String>, _>("revoked_reason"),
                });
                results.push(result);
            }
            Ok(results)
        }
    }

    // ================================
    // Impersonation Session Management
    // ================================
}

#[async_trait]
impl ImpersonationRepository for PostgresDatabase {
    async fn create_session(&self, session: &ImpersonationSession) -> AppResult<()> {
        let query = r"
            INSERT INTO impersonation_sessions (
                id, impersonator_id, target_user_id, reason,
                started_at, ended_at, is_active, created_at
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
        ";

        sqlx::query(query)
            .bind(&session.id)
            .bind(session.impersonator_id)
            .bind(session.target_user_id)
            .bind(&session.reason)
            .bind(session.started_at)
            .bind(session.ended_at)
            .bind(session.is_active)
            .bind(session.created_at)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                AppError::database(format!("Failed to create impersonation session: {e}"))
            })?;

        Ok(())
    }

    async fn get_session(&self, session_id: &str) -> AppResult<Option<ImpersonationSession>> {
        let query = r"
            SELECT id, impersonator_id, target_user_id, reason,
                   started_at, ended_at, is_active, created_at
            FROM impersonation_sessions WHERE id = $1
        ";

        let row = sqlx::query(query)
            .bind(session_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to get impersonation session: {e}")))?;

        row.map(|r| shared::mappers::parse_impersonation_session_from_row(&r))
            .transpose()
    }

    async fn get_active_session(&self, user_id: Uuid) -> AppResult<Option<ImpersonationSession>> {
        let query = r"
            SELECT id, impersonator_id, target_user_id, reason,
                   started_at, ended_at, is_active, created_at
            FROM impersonation_sessions
            WHERE (impersonator_id = $1 OR target_user_id = $2) AND is_active = true
            ORDER BY started_at DESC LIMIT 1
        ";

        let user_id_str = user_id.to_string();
        let row = sqlx::query(query)
            .bind(&user_id_str)
            .bind(&user_id_str)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| {
                AppError::database(format!("Failed to get active impersonation session: {e}"))
            })?;

        row.map(|r| shared::mappers::parse_impersonation_session_from_row(&r))
            .transpose()
    }

    async fn end_session(&self, session_id: &str) -> AppResult<()> {
        let query = r"
            UPDATE impersonation_sessions
            SET is_active = false, ended_at = $1
            WHERE id = $2
        ";

        let ended_at = chrono::Utc::now();
        sqlx::query(query)
            .bind(ended_at)
            .bind(session_id)
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to end impersonation session: {e}")))?;

        Ok(())
    }

    async fn end_all_sessions(&self, impersonator_id: Uuid) -> AppResult<u64> {
        let query = r"
            UPDATE impersonation_sessions
            SET is_active = false, ended_at = $1
            WHERE impersonator_id = $2 AND is_active = true
        ";

        let ended_at = chrono::Utc::now();
        let result = sqlx::query(query)
            .bind(ended_at)
            .bind(impersonator_id)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                AppError::database(format!("Failed to end impersonation sessions: {e}"))
            })?;

        Ok(result.rows_affected())
    }

    async fn list_sessions(
        &self,
        impersonator_id: Option<Uuid>,
        target_user_id: Option<Uuid>,
        active_only: bool,
        limit: u32,
    ) -> AppResult<Vec<ImpersonationSession>> {
        use std::fmt::Write;

        // Build dynamic query based on filters
        let mut query = String::from(
            r"
            SELECT id, impersonator_id, target_user_id, reason,
                   started_at, ended_at, is_active, created_at
            FROM impersonation_sessions WHERE 1=1
            ",
        );

        let mut param_idx = 1u32;

        if impersonator_id.is_some() {
            let _ = write!(query, " AND impersonator_id = ${param_idx}");
            param_idx += 1;
        }
        if target_user_id.is_some() {
            let _ = write!(query, " AND target_user_id = ${param_idx}");
            param_idx += 1;
        }
        if active_only {
            query.push_str(" AND is_active = true");
        }
        let _ = write!(query, " ORDER BY started_at DESC LIMIT ${param_idx}");

        let mut sql_query = sqlx::query(&query);

        if let Some(id) = impersonator_id {
            sql_query = sql_query.bind(id.to_string());
        }
        if let Some(id) = target_user_id {
            sql_query = sql_query.bind(id.to_string());
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
        let limit_i32 = limit as i32;
        sql_query = sql_query.bind(limit_i32);

        let rows = sql_query.fetch_all(&self.pool).await.map_err(|e| {
            AppError::database(format!("Failed to list impersonation sessions: {e}"))
        })?;

        rows.iter()
            .map(shared::mappers::parse_impersonation_session_from_row)
            .collect()
    }
}

#[async_trait]
impl UserMcpTokenRepository for PostgresDatabase {
    async fn create_token(
        &self,
        user_id: Uuid,
        request: &CreateUserMcpTokenRequest,
    ) -> AppResult<UserMcpTokenCreated> {
        let token_value = Self::generate_mcp_token();
        let token_hash = Self::hash_mcp_token(&token_value);
        let token_prefix = token_value.chars().take(12).collect::<String>();
        let id = Uuid::new_v4().to_string();
        let now = chrono::Utc::now();

        let expires_at = request
            .expires_in_days
            .map(|days| now + chrono::Duration::days(i64::from(days)));

        sqlx::query(
            r"
            INSERT INTO user_mcp_tokens (
                id, user_id, name, token_hash, token_prefix,
                expires_at, last_used_at, usage_count, is_revoked, created_at
            ) VALUES ($1, $2, $3, $4, $5, $6, NULL, 0, false, $7)
            ",
        )
        .bind(&id)
        .bind(user_id)
        .bind(&request.name)
        .bind(&token_hash)
        .bind(&token_prefix)
        .bind(expires_at)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to create user MCP token: {e}")))?;

        let token = UserMcpToken {
            id,
            user_id,
            name: request.name.clone(),
            token_hash,
            token_prefix,
            expires_at,
            last_used_at: None,
            usage_count: 0,
            is_revoked: false,
            created_at: now,
        };

        Ok(UserMcpTokenCreated { token, token_value })
    }

    async fn validate_token(&self, token_value: &str) -> AppResult<Uuid> {
        use sqlx::Row;

        let token_hash = Self::hash_mcp_token(token_value);
        let token_prefix = token_value.chars().take(12).collect::<String>();

        let row = sqlx::query(
            r"
            SELECT id, user_id, expires_at, is_revoked
            FROM user_mcp_tokens
            WHERE token_prefix = $1 AND token_hash = $2
            ",
        )
        .bind(&token_prefix)
        .bind(&token_hash)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to validate user MCP token: {e}")))?;

        let row = row.ok_or_else(|| AppError::auth_invalid("Invalid MCP token"))?;
        let is_revoked: bool = row.get("is_revoked");
        if is_revoked {
            return Err(AppError::auth_invalid("MCP token has been revoked"));
        }

        let expires_at: Option<chrono::DateTime<chrono::Utc>> = row.get("expires_at");
        if let Some(exp) = expires_at {
            if exp < chrono::Utc::now() {
                return Err(AppError::auth_invalid("MCP token has expired"));
            }
        }

        let token_id: String = row.get("id");
        self.update_user_mcp_token_usage(&token_id).await?;

        let user_id_str: String = row.get("user_id");
        Uuid::parse_str(&user_id_str)
            .map_err(|e| AppError::internal(format!("Failed to parse user_id UUID: {e}")))
    }

    async fn revoke_token(&self, token_id: &str, user_id: Uuid) -> AppResult<()> {
        let result = sqlx::query(
            r"
            UPDATE user_mcp_tokens
            SET is_revoked = true
            WHERE id = $1 AND user_id = $2
            ",
        )
        .bind(token_id)
        .bind(user_id)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to revoke user MCP token: {e}")))?;

        if result.rows_affected() == 0 {
            return Err(AppError::not_found("MCP token not found or unauthorized"));
        }

        Ok(())
    }

    async fn get_token(&self, token_id: &str, user_id: Uuid) -> AppResult<Option<UserMcpToken>> {
        let row = sqlx::query(
            r"
            SELECT id, user_id, name, token_hash, token_prefix,
                   expires_at, last_used_at, usage_count, is_revoked, created_at
            FROM user_mcp_tokens
            WHERE id = $1 AND user_id = $2
            ",
        )
        .bind(token_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get user MCP token: {e}")))?;

        row.map(|r| shared::mappers::parse_user_mcp_token_from_row(&r))
            .transpose()
    }

    async fn list_tokens(&self, user_id: Uuid) -> AppResult<Vec<UserMcpTokenInfo>> {
        use sqlx::Row;

        let rows = sqlx::query(
            r"
            SELECT id, name, token_prefix, expires_at, last_used_at,
                   usage_count, is_revoked, created_at
            FROM user_mcp_tokens
            WHERE user_id = $1
            ORDER BY created_at DESC
            ",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to list user MCP tokens: {e}")))?;
        rows.iter()
            .map(|row| {
                Ok(UserMcpTokenInfo {
                    id: row.get("id"),
                    name: row.get("name"),
                    token_prefix: row.get("token_prefix"),
                    expires_at: row.get("expires_at"),
                    last_used_at: row.get("last_used_at"),
                    usage_count: u32::try_from(row.get::<i32, _>("usage_count")).map_err(|e| {
                        AppError::internal(format!(
                            "Integer conversion failed for usage_count: {e}"
                        ))
                    })?,
                    is_revoked: row.get("is_revoked"),
                    created_at: row.get("created_at"),
                })
            })
            .collect()
    }

    async fn cleanup_expired_tokens(&self) -> AppResult<u64> {
        let result = sqlx::query(
            r"
            UPDATE user_mcp_tokens
            SET is_revoked = true
            WHERE expires_at IS NOT NULL
            AND expires_at < $1
            AND is_revoked = false
            ",
        )
        .bind(chrono::Utc::now())
        .execute(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!("Failed to cleanup expired user MCP tokens: {e}"))
        })?;

        Ok(result.rows_affected())
    }
}
