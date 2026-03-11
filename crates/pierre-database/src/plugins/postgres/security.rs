// ABOUTME: PostgreSQL security and notification repository implementations
// ABOUTME: Manages audit events, RSA keypairs, key rotation, and OAuth notifications
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::super::{NotificationRepository, SecurityRepository};
use super::PostgresDatabase;
use crate::plugins::shared;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::admin::AdminJwtManager;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::KeyVersion;
use pierre_core::models::OAuthNotification;
use pierre_core::models::TenantId;
use pierre_core::models::{AuditEvent, AuditEventType, AuditSeverity};
use pierre_core::uuid_utils::parse_uuid;
use sqlx::Row;
use std::fmt::Write;
use tracing::{debug, warn};
use uuid::Uuid;

#[async_trait]
impl SecurityRepository for PostgresDatabase {
    // ================================
    // RSA Key Persistence for JWT Signing
    // ================================

    /// Save RSA keypair to database for persistence across restarts
    async fn save_rsa_keypair(
        &self,
        kid: &str,
        private_key_pem: &str,
        public_key_pem: &str,
        created_at: DateTime<Utc>,
        is_active: bool,
        key_size_bits: i32,
    ) -> AppResult<()> {
        sqlx::query(
            r"
            INSERT INTO rsa_keypairs (kid, private_key_pem, public_key_pem, created_at, is_active, key_size_bits)
            VALUES ($1, $2, $3, $4, $5, $6)
            ON CONFLICT(kid) DO UPDATE SET
                private_key_pem = EXCLUDED.private_key_pem,
                public_key_pem = EXCLUDED.public_key_pem,
                is_active = EXCLUDED.is_active
            ",
        )
        .bind(kid)
        .bind(private_key_pem)
        .bind(public_key_pem)
        .bind(created_at)
        .bind(is_active)
        .bind(key_size_bits)
        .execute(&self.pool).await.map_err(|e| AppError::database(format!("Database operation failed: {e}")))?;

        Ok(())
    }

    /// Load all RSA keypairs from database
    async fn load_rsa_keypairs(
        &self,
    ) -> AppResult<Vec<(String, String, String, DateTime<Utc>, bool)>> {
        let rows = sqlx::query(
            "SELECT kid, private_key_pem, public_key_pem, created_at, is_active FROM rsa_keypairs ORDER BY created_at DESC",
        )
        .fetch_all(&self.pool).await.map_err(|e| AppError::database(format!("Failed to fetch records: {e}")))?;

        let mut keypairs = Vec::new();
        for row in rows {
            let kid: String = row.get("kid");
            let private_key_pem: String = row.get("private_key_pem");
            let public_key_pem: String = row.get("public_key_pem");
            let created_at: DateTime<Utc> = row.get("created_at");
            let is_active: bool = row.get("is_active");

            keypairs.push((kid, private_key_pem, public_key_pem, created_at, is_active));
        }

        Ok(keypairs)
    }

    /// Update active status of RSA keypair
    async fn update_rsa_keypair_active_status(&self, kid: &str, is_active: bool) -> AppResult<()> {
        sqlx::query("UPDATE rsa_keypairs SET is_active = $1 WHERE kid = $2")
            .bind(is_active)
            .bind(kid)
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Database operation failed: {e}")))?;

        Ok(())
    }

    // ================================
    // Key Rotation & Security - PostgreSQL implementations
    // ================================

    async fn store_key_version(&self, version: &KeyVersion) -> AppResult<()> {
        let query = r"
            INSERT INTO key_versions (tenant_id, version, created_at, expires_at, is_active, algorithm)
            VALUES ($1, $2, $3, $4, $5, $6)
            ON CONFLICT (tenant_id, version) DO UPDATE SET
                expires_at = EXCLUDED.expires_at,
                is_active = EXCLUDED.is_active,
                algorithm = EXCLUDED.algorithm
        ";

        sqlx::query(query)
            .bind(version.tenant_id.map(|id| id.to_string()))
            .bind(i32::try_from(version.version).unwrap_or(0)) // Safe: version ranges are controlled by application
            .bind(version.created_at)
            .bind(version.expires_at)
            .bind(version.is_active)
            .bind(&version.algorithm)
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to store key version: {e}")))?;

        debug!(
            "Stored key version {} for tenant {:?}",
            version.version, version.tenant_id
        );
        Ok(())
    }

    async fn get_key_versions(&self, tenant_id: Option<TenantId>) -> AppResult<Vec<KeyVersion>> {
        let query = match tenant_id {
            Some(_) => {
                r"
                SELECT tenant_id, version, created_at, expires_at, is_active, algorithm
                FROM key_versions 
                WHERE tenant_id = $1
                ORDER BY version DESC
            "
            }
            None => {
                r"
                SELECT tenant_id, version, created_at, expires_at, is_active, algorithm
                FROM key_versions 
                WHERE tenant_id IS NULL
                ORDER BY version DESC
            "
            }
        };

        let rows = if let Some(tid) = tenant_id {
            sqlx::query(query)
                .bind(tid.to_string())
                .fetch_all(&self.pool)
                .await
        } else {
            sqlx::query(query).fetch_all(&self.pool).await
        }
        .map_err(|e| AppError::database(format!("Failed to fetch key versions: {e}")))?;

        let mut versions = Vec::new();
        for row in rows {
            let tenant_id_str: Option<String> = row.get("tenant_id");
            let tenant_id = if let Some(tid) = tenant_id_str {
                Some(TenantId::from_uuid(parse_uuid(&tid)?))
            } else {
                None
            };

            let version = KeyVersion {
                tenant_id,
                version: u32::try_from(row.get::<i32, _>("version")).unwrap_or(0), // Safe: stored versions are always positive
                created_at: row.get("created_at"),
                expires_at: row.get("expires_at"),
                is_active: row.get("is_active"),
                algorithm: row.get("algorithm"),
            };
            versions.push(version);
        }

        Ok(versions)
    }

    async fn get_current_key_version(
        &self,
        tenant_id: Option<TenantId>,
    ) -> AppResult<Option<KeyVersion>> {
        let query = match tenant_id {
            Some(_) => {
                r"
                SELECT tenant_id, version, created_at, expires_at, is_active, algorithm
                FROM key_versions 
                WHERE tenant_id = $1 AND is_active = true
                ORDER BY version DESC
                LIMIT 1
            "
            }
            None => {
                r"
                SELECT tenant_id, version, created_at, expires_at, is_active, algorithm
                FROM key_versions 
                WHERE tenant_id IS NULL AND is_active = true
                ORDER BY version DESC
                LIMIT 1
            "
            }
        };

        let row = if let Some(tid) = tenant_id {
            sqlx::query(query)
                .bind(tid.to_string())
                .fetch_optional(&self.pool)
                .await
        } else {
            sqlx::query(query).fetch_optional(&self.pool).await
        }
        .map_err(|e| AppError::database(format!("Failed to fetch current key version: {e}")))?;

        if let Some(row) = row {
            let tenant_id_str: Option<String> = row.get("tenant_id");
            let tenant_id = if let Some(tid) = tenant_id_str {
                Some(TenantId::from_uuid(parse_uuid(&tid)?))
            } else {
                None
            };

            let version = KeyVersion {
                tenant_id,
                version: u32::try_from(row.get::<i32, _>("version")).unwrap_or(0), // Safe: stored versions are always positive
                created_at: row.get("created_at"),
                expires_at: row.get("expires_at"),
                is_active: row.get("is_active"),
                algorithm: row.get("algorithm"),
            };
            Ok(Some(version))
        } else {
            Ok(None)
        }
    }

    async fn update_key_version_status(
        &self,
        tenant_id: Option<TenantId>,
        version: u32,
        is_active: bool,
    ) -> AppResult<()> {
        let query = match tenant_id {
            Some(_) => {
                r"
                UPDATE key_versions 
                SET is_active = $3
                WHERE tenant_id = $1 AND version = $2
            "
            }
            None => {
                r"
                UPDATE key_versions 
                SET is_active = $2
                WHERE tenant_id IS NULL AND version = $1
            "
            }
        };

        let result = if let Some(tid) = tenant_id {
            sqlx::query(query)
                .bind(tid.to_string())
                .bind(i32::try_from(version).unwrap_or(0)) // Safe: version ranges are controlled by application
                .bind(is_active)
                .execute(&self.pool)
                .await
        } else {
            sqlx::query(query)
                .bind(i32::try_from(version).unwrap_or(0)) // Safe: version ranges are controlled by application
                .bind(is_active)
                .execute(&self.pool)
                .await
        }
        .map_err(|e| AppError::database(format!("Failed to update key version status: {e}")))?;

        if result.rows_affected() == 0 {
            warn!(
                "No key version found to update: tenant={:?}, version={}",
                tenant_id, version
            );
        } else {
            debug!(
                "Updated key version {} status to {} for tenant {:?}",
                version, is_active, tenant_id
            );
        }

        Ok(())
    }

    async fn delete_old_key_versions(
        &self,
        tenant_id: Option<TenantId>,
        keep_count: u32,
    ) -> AppResult<u64> {
        let query = match tenant_id {
            Some(_) => {
                r"
                DELETE FROM key_versions 
                WHERE tenant_id = $1 
                AND version NOT IN (
                    SELECT version FROM key_versions 
                    WHERE tenant_id = $1
                    ORDER BY version DESC 
                    LIMIT $2
                )
            "
            }
            None => {
                r"
                DELETE FROM key_versions 
                WHERE tenant_id IS NULL 
                AND version NOT IN (
                    SELECT version FROM key_versions 
                    WHERE tenant_id IS NULL
                    ORDER BY version DESC 
                    LIMIT $1
                )
            "
            }
        };

        let result = if let Some(tid) = tenant_id {
            sqlx::query(query)
                .bind(tid.to_string())
                .bind(i32::try_from(keep_count).unwrap_or(0)) // Safe: keep_count ranges are controlled by application
                .execute(&self.pool)
                .await
        } else {
            sqlx::query(query)
                .bind(i32::try_from(keep_count).unwrap_or(0)) // Safe: keep_count ranges are controlled by application
                .execute(&self.pool)
                .await
        }
        .map_err(|e| AppError::database(format!("Failed to delete old key versions: {e}")))?;

        let deleted_count = result.rows_affected();
        debug!(
            "Deleted {} old key versions for tenant {:?}, kept {} most recent",
            deleted_count, tenant_id, keep_count
        );

        Ok(deleted_count)
    }

    async fn store_audit_event(&self, event: &AuditEvent) -> AppResult<()> {
        let query = r"
            INSERT INTO audit_events (
                id, event_type, severity, message, source, result, 
                tenant_id, user_id, ip_address, user_agent, metadata, timestamp
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9::inet, $10, $11, $12)
        ";

        let event_type_str = format!("{:?}", event.event_type);
        let severity_str = format!("{:?}", event.severity);
        let metadata_json = serde_json::to_string(&event.metadata)?;

        sqlx::query(query)
            .bind(event.event_id.to_string())
            .bind(&event_type_str)
            .bind(&severity_str)
            .bind(&event.description)
            .bind("security") // source - using generic security source
            .bind(&event.result)
            .bind(event.tenant_id.map(|id| id.to_string()))
            .bind(event.user_id.map(|id| id.to_string()))
            .bind(&event.source_ip)
            .bind(&event.user_agent)
            .bind(&metadata_json)
            .bind(event.timestamp)
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Database operation failed: {e}")))?;

        Ok(())
    }

    /// Complex audit query with dynamic filtering, pagination, and exhaustive enum mapping
    ///
    /// JUSTIFICATION for `#[allow(clippy::too_many_lines)]`:
    /// - Dynamic SQL query building with optional filters (`tenant_id`, `event_type`, `limit`)
    /// - Exhaustive match for 25+ `AuditEventType` variants (cannot be extracted without loss of context)
    /// - Exhaustive match for `AuditSeverity` variants
    /// - Row-to-struct mapping with UUID parsing and JSON deserialization
    /// - Refactoring would fragment audit event construction logic across multiple functions
    #[allow(clippy::too_many_lines)]
    async fn get_audit_events(
        &self,
        tenant_id: Option<TenantId>,
        event_type: Option<&str>,
        limit: Option<u32>,
    ) -> AppResult<Vec<AuditEvent>> {
        use std::fmt::Write;

        let mut query = r"
            SELECT id, event_type, severity, message, source, result,
                   tenant_id, user_id, ip_address, user_agent, metadata, timestamp
            FROM audit_events
            WHERE true
        "
        .to_owned();

        let mut bind_count = 0;
        if tenant_id.is_some() {
            bind_count += 1;
            if write!(query, " AND tenant_id = ${bind_count}").is_err() {
                return Err(AppError::database(
                    "Failed to write tenant_id clause to query".to_owned(),
                ));
            }
        }
        if event_type.is_some() {
            bind_count += 1;
            if write!(query, " AND event_type = ${bind_count}").is_err() {
                return Err(AppError::database(
                    "Failed to write event_type clause to query".to_owned(),
                ));
            }
        }

        query.push_str(" ORDER BY timestamp DESC");

        if limit.is_some() {
            bind_count += 1;
            if write!(query, " LIMIT ${bind_count}").is_err() {
                return Err(AppError::database(
                    "Failed to write LIMIT clause to query".to_owned(),
                ));
            }
        }

        let mut sql_query = sqlx::query(&query);

        if let Some(tid) = tenant_id {
            sql_query = sql_query.bind(tid.to_string());
        }
        if let Some(et) = event_type {
            sql_query = sql_query.bind(et);
        }
        if let Some(l) = limit {
            sql_query = sql_query.bind(i32::try_from(l).unwrap_or(0)); // Safe: limit ranges are controlled by application
        }

        let rows = sql_query
            .fetch_all(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to get audit events: {e}")))?;

        let mut events = Vec::new();
        for row in rows {
            let event_id_str: String = row.get("id");
            let event_id = uuid::Uuid::parse_str(&event_id_str)
                .map_err(|e| AppError::database(format!("Invalid audit event UUID: {e}")))?;

            let event_type_str: String = row.get("event_type");
            let event_type = match event_type_str.as_str() {
                "UserLogin" => AuditEventType::UserLogin,
                "UserLogout" => AuditEventType::UserLogout,
                "AuthenticationFailed" => AuditEventType::AuthenticationFailed,
                "ApiKeyUsed" => AuditEventType::ApiKeyUsed,
                "OAuthCredentialsAccessed" => AuditEventType::OAuthCredentialsAccessed,
                "OAuthCredentialsModified" => AuditEventType::OAuthCredentialsModified,
                "OAuthCredentialsCreated" => AuditEventType::OAuthCredentialsCreated,
                "OAuthCredentialsDeleted" => AuditEventType::OAuthCredentialsDeleted,
                "TokenRefreshed" => AuditEventType::TokenRefreshed,
                "TenantCreated" => AuditEventType::TenantCreated,
                "TenantModified" => AuditEventType::TenantModified,
                "TenantDeleted" => AuditEventType::TenantDeleted,
                "TenantUserAdded" => AuditEventType::TenantUserAdded,
                "TenantUserRemoved" => AuditEventType::TenantUserRemoved,
                "TenantUserRoleChanged" => AuditEventType::TenantUserRoleChanged,
                "DataEncrypted" => AuditEventType::DataEncrypted,
                "DataDecrypted" => AuditEventType::DataDecrypted,
                "KeyRotated" => AuditEventType::KeyRotated,
                "EncryptionFailed" => AuditEventType::EncryptionFailed,
                "ToolExecutionFailed" => AuditEventType::ToolExecutionFailed,
                "ProviderApiCalled" => AuditEventType::ProviderApiCalled,
                "ConfigurationChanged" => AuditEventType::ConfigurationChanged,
                "SystemMaintenance" => AuditEventType::SystemMaintenance,
                "SecurityPolicyViolation" => AuditEventType::SecurityPolicyViolation,
                _ => AuditEventType::ToolExecuted, // Default fallback
            };

            let severity_str: String = row.get("severity");
            let severity = match severity_str.as_str() {
                "Warning" => AuditSeverity::Warning,
                "Error" => AuditSeverity::Error,
                "Critical" => AuditSeverity::Critical,
                _ => AuditSeverity::Info, // Default fallback
            };

            let tenant_id_str: Option<String> = row.get("tenant_id");
            let tenant_id = if let Some(tid) = tenant_id_str {
                Some(TenantId::from_uuid(parse_uuid(&tid)?))
            } else {
                None
            };

            let user_id_str: Option<String> = row.get("user_id");
            let user_id = if let Some(uid) = user_id_str {
                Some(parse_uuid(&uid)?)
            } else {
                None
            };

            let metadata_json: String = row.get("metadata");
            let metadata: serde_json::Value = serde_json::from_str(&metadata_json)
                .unwrap_or_else(|_| serde_json::Value::Object(serde_json::Map::new()));

            let event = AuditEvent {
                event_id,
                event_type,
                severity,
                timestamp: row.get("timestamp"),
                user_id,
                tenant_id,
                source_ip: row.get("ip_address"),
                user_agent: row.get("user_agent"),
                session_id: None, // Not stored in current schema
                description: row.get("message"),
                metadata,
                resource: None,             // Not stored in current schema
                action: "audit".to_owned(), // Default action
                result: row.get("result"),
            };
            events.push(event);
        }

        Ok(events)
    }

    // ================================
    // System Secret Management Implementation
    // ================================

    /// Get or create system secret (generates if not exists)
    async fn get_or_create_system_secret(&self, secret_type: &str) -> AppResult<String> {
        // Try to get existing secret
        if let Ok(secret) = self.get_system_secret(secret_type).await {
            return Ok(secret);
        }

        // Generate new secret
        let secret_value = match secret_type {
            "admin_jwt_secret" => AdminJwtManager::generate_jwt_secret(),
            _ => {
                return Err(AppError::invalid_input(format!(
                    "Unknown secret type: {secret_type}"
                )))
            }
        };

        // Store in database
        let now = chrono::Utc::now();
        sqlx::query("INSERT INTO system_secrets (secret_type, secret_value, created_at, updated_at) VALUES ($1, $2, $3, $4)")
            .bind(secret_type)
            .bind(&secret_value)
            .bind(now)
            .bind(now)
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Database operation failed: {e}")))?;

        Ok(secret_value)
    }

    /// Get existing system secret
    async fn get_system_secret(&self, secret_type: &str) -> AppResult<String> {
        let row = sqlx::query("SELECT secret_value FROM system_secrets WHERE secret_type = $1")
            .bind(secret_type)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to fetch record: {e}")))?;

        Ok(row
            .try_get("secret_value")
            .map_err(|e| AppError::database(format!("Failed to parse secret_value column: {e}")))?)
    }

    /// Update or insert system secret (supports both initial storage and rotation)
    async fn update_system_secret(&self, secret_type: &str, new_value: &str) -> AppResult<()> {
        let now = chrono::Utc::now();
        sqlx::query(
            "INSERT INTO system_secrets (secret_type, secret_value, created_at, updated_at) \
             VALUES ($1, $2, $3, $4) \
             ON CONFLICT(secret_type) DO UPDATE SET secret_value = EXCLUDED.secret_value, updated_at = EXCLUDED.updated_at",
        )
        .bind(secret_type)
        .bind(new_value)
        .bind(now)
        .bind(now)
        .execute(&self.pool).await.map_err(|e| AppError::database(format!("Database operation failed: {e}")))?;

        Ok(())
    }

    // ================================
    // OAuth Notifications
    // ================================

    // ================================
    // Encryption Interface (delegates to HasEncryption trait)
    // ================================

    fn encrypt_data_with_aad(&self, data: &str, aad: &str) -> AppResult<String> {
        shared::encryption::HasEncryption::encrypt_data_with_aad(self, data, aad)
    }

    fn decrypt_data_with_aad(&self, encrypted: &str, aad: &str) -> AppResult<String> {
        shared::encryption::HasEncryption::decrypt_data_with_aad(self, encrypted, aad)
    }
}

#[async_trait]
impl NotificationRepository for PostgresDatabase {
    async fn store(
        &self,
        user_id: Uuid,
        provider: &str,
        success: bool,
        message: &str,
        expires_at: Option<&str>,
    ) -> AppResult<String> {
        let notification_id = Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();

        sqlx::query(
            r"
            INSERT INTO oauth_notifications (id, user_id, provider, success, message, expires_at, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            ",
        )
        .bind(&notification_id)
        .bind(user_id.to_string())
        .bind(provider)
        .bind(success)
        .bind(message)
        .bind(expires_at)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Database operation failed: {e}")))?;

        Ok(notification_id)
    }

    async fn get_unread(&self, user_id: Uuid) -> AppResult<Vec<OAuthNotification>> {
        let rows = sqlx::query(
            r"
            SELECT id, user_id, provider, success, message, expires_at, created_at, read_at
            FROM oauth_notifications
            WHERE user_id = $1 AND read_at IS NULL
            ORDER BY created_at DESC
            ",
        )
        .bind(user_id.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to fetch records: {e}")))?;

        let mut notifications = Vec::new();
        for row in rows {
            notifications.push(OAuthNotification {
                id: row.get("id"),
                user_id: row.get("user_id"),
                provider: row.get("provider"),
                success: row.get("success"),
                message: row.get("message"),
                expires_at: row.get("expires_at"),
                created_at: row.get("created_at"),
                read_at: row.get("read_at"),
            });
        }

        Ok(notifications)
    }

    async fn mark_read(&self, notification_id: &str, user_id: Uuid) -> AppResult<bool> {
        let result = sqlx::query(
            r"
            UPDATE oauth_notifications 
            SET read_at = CURRENT_TIMESTAMP
            WHERE id = $1 AND user_id = $2 AND read_at IS NULL
            ",
        )
        .bind(notification_id)
        .bind(user_id.to_string())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Database operation failed: {e}")))?;

        Ok(result.rows_affected() > 0)
    }

    async fn mark_all_read(&self, user_id: Uuid) -> AppResult<u64> {
        let result = sqlx::query(
            r"
            UPDATE oauth_notifications 
            SET read_at = CURRENT_TIMESTAMP
            WHERE user_id = $1 AND read_at IS NULL
            ",
        )
        .bind(user_id.to_string())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Database operation failed: {e}")))?;

        Ok(result.rows_affected())
    }

    async fn get_all(
        &self,
        user_id: Uuid,
        limit: Option<i64>,
    ) -> AppResult<Vec<OAuthNotification>> {
        let mut query_str = String::from(
            r"
            SELECT id, user_id, provider, success, message, expires_at, created_at, read_at
            FROM oauth_notifications
            WHERE user_id = $1
            ORDER BY created_at DESC
            ",
        );

        if let Some(l) = limit {
            write!(query_str, " LIMIT {l}")
                .map_err(|e| AppError::internal(format!("Format error: {e}")))?;
        }

        let rows = sqlx::query(&query_str)
            .bind(user_id.to_string())
            .fetch_all(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to fetch records: {e}")))?;

        let mut notifications = Vec::new();
        for row in rows {
            notifications.push(OAuthNotification {
                id: row.get("id"),
                user_id: row.get("user_id"),
                provider: row.get("provider"),
                success: row.get("success"),
                message: row.get("message"),
                expires_at: row.get("expires_at"),
                created_at: row.get("created_at"),
                read_at: row.get("read_at"),
            });
        }

        Ok(notifications)
    }
}
