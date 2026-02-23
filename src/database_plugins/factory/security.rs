// ABOUTME: Security repository dispatch for the database factory
// ABOUTME: Delegates SecurityRepository and NotificationRepository calls to SQLite or PostgreSQL backends
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::Database;
use crate::database_plugins::{NotificationRepository, SecurityRepository};
use crate::errors::AppResult;
use crate::models::OAuthNotification;
use crate::security::audit::AuditEvent;
use crate::security::key_rotation::KeyVersion;
use async_trait::async_trait;
use pierre_core::models::TenantId;
use uuid::Uuid;

#[async_trait]
impl SecurityRepository for Database {
    async fn store_key_version(&self, version: &KeyVersion) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.store_key_version(version).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.store_key_version(version).await,
        }
    }
    async fn get_key_versions(&self, tenant_id: Option<TenantId>) -> AppResult<Vec<KeyVersion>> {
        match self {
            Self::SQLite(db) => db.get_key_versions(tenant_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_key_versions(tenant_id).await,
        }
    }
    async fn get_current_key_version(
        &self,
        tenant_id: Option<TenantId>,
    ) -> AppResult<Option<KeyVersion>> {
        match self {
            Self::SQLite(db) => db.get_current_key_version(tenant_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_current_key_version(tenant_id).await,
        }
    }
    async fn update_key_version_status(
        &self,
        tenant_id: Option<TenantId>,
        version: u32,
        is_active: bool,
    ) -> AppResult<()> {
        match self {
            Self::SQLite(db) => {
                db.update_key_version_status(tenant_id, version, is_active)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                db.update_key_version_status(tenant_id, version, is_active)
                    .await
            }
        }
    }
    async fn delete_old_key_versions(
        &self,
        tenant_id: Option<TenantId>,
        keep_count: u32,
    ) -> AppResult<u64> {
        match self {
            Self::SQLite(db) => db.delete_old_key_versions(tenant_id, keep_count).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.delete_old_key_versions(tenant_id, keep_count).await,
        }
    }
    async fn store_audit_event(&self, event: &AuditEvent) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.store_audit_event(event).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.store_audit_event(event).await,
        }
    }
    async fn get_audit_events(
        &self,
        tenant_id: Option<TenantId>,
        event_type: Option<&str>,
        limit: Option<u32>,
    ) -> AppResult<Vec<AuditEvent>> {
        match self {
            Self::SQLite(db) => db.get_audit_events(tenant_id, event_type, limit).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_audit_events(tenant_id, event_type, limit).await,
        }
    }
    async fn get_or_create_system_secret(&self, secret_type: &str) -> AppResult<String> {
        match self {
            Self::SQLite(db) => db.get_or_create_system_secret(secret_type).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_or_create_system_secret(secret_type).await,
        }
    }
    async fn get_system_secret(&self, secret_type: &str) -> AppResult<String> {
        match self {
            Self::SQLite(db) => db.get_system_secret(secret_type).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_system_secret(secret_type).await,
        }
    }
    async fn update_system_secret(&self, secret_type: &str, new_value: &str) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.update_system_secret(secret_type, new_value).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.update_system_secret(secret_type, new_value).await,
        }
    }
    async fn save_rsa_keypair(
        &self,
        kid: &str,
        private_key_pem: &str,
        public_key_pem: &str,
        created_at: chrono::DateTime<chrono::Utc>,
        is_active: bool,
        key_size_bits: i32,
    ) -> AppResult<()> {
        match self {
            Self::SQLite(db) => {
                db.save_rsa_keypair(
                    kid,
                    private_key_pem,
                    public_key_pem,
                    created_at,
                    is_active,
                    usize::try_from(key_size_bits).unwrap_or(2048),
                )
                .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                db.save_rsa_keypair(
                    kid,
                    private_key_pem,
                    public_key_pem,
                    created_at,
                    is_active,
                    key_size_bits,
                )
                .await
            }
        }
    }
    async fn load_rsa_keypairs(
        &self,
    ) -> AppResult<Vec<(String, String, String, chrono::DateTime<chrono::Utc>, bool)>> {
        match self {
            Self::SQLite(db) => db.load_rsa_keypairs().await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.load_rsa_keypairs().await,
        }
    }
    async fn update_rsa_keypair_active_status(&self, kid: &str, is_active: bool) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.update_rsa_keypair_active_status(kid, is_active).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.update_rsa_keypair_active_status(kid, is_active).await,
        }
    }
    fn encrypt_data_with_aad(&self, data: &str, aad: &str) -> AppResult<String> {
        match self {
            Self::SQLite(db) => db.encrypt_data_with_aad(data, aad),
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.encrypt_data_with_aad(data, aad),
        }
    }
    fn decrypt_data_with_aad(&self, encrypted: &str, aad: &str) -> AppResult<String> {
        match self {
            Self::SQLite(db) => db.decrypt_data_with_aad(encrypted, aad),
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.decrypt_data_with_aad(encrypted, aad),
        }
    }
}

#[async_trait]
impl NotificationRepository for Database {
    async fn store(
        &self,
        user_id: Uuid,
        provider: &str,
        success: bool,
        message: &str,
        expires_at: Option<&str>,
    ) -> AppResult<String> {
        match self {
            Self::SQLite(db) => {
                NotificationRepository::store(db, user_id, provider, success, message, expires_at)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                NotificationRepository::store(db, user_id, provider, success, message, expires_at)
                    .await
            }
        }
    }

    async fn get_unread(&self, user_id: Uuid) -> AppResult<Vec<OAuthNotification>> {
        match self {
            Self::SQLite(db) => NotificationRepository::get_unread(db, user_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => NotificationRepository::get_unread(db, user_id).await,
        }
    }

    async fn mark_read(&self, notification_id: &str, user_id: Uuid) -> AppResult<bool> {
        match self {
            Self::SQLite(db) => {
                NotificationRepository::mark_read(db, notification_id, user_id).await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                NotificationRepository::mark_read(db, notification_id, user_id).await
            }
        }
    }

    async fn mark_all_read(&self, user_id: Uuid) -> AppResult<u64> {
        match self {
            Self::SQLite(db) => NotificationRepository::mark_all_read(db, user_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => NotificationRepository::mark_all_read(db, user_id).await,
        }
    }

    async fn get_all(
        &self,
        user_id: Uuid,
        limit: Option<i64>,
    ) -> AppResult<Vec<OAuthNotification>> {
        match self {
            Self::SQLite(db) => NotificationRepository::get_all(db, user_id, limit).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => NotificationRepository::get_all(db, user_id, limit).await,
        }
    }
}
