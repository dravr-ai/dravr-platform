// ABOUTME: Security database operations covering key rotation, audit events, and system secrets
// ABOUTME: Enables SecurityRepository and NotificationRepository blanket impls
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::AppResult;
use pierre_core::models::{AuditEvent, KeyVersion, OAuthNotification, TenantId};
use uuid::Uuid;

/// Security, audit, encryption, and notification database operations
#[async_trait]
pub trait SecurityDbOps: Send + Sync + Clone {
    // --- RSA Key Persistence for JWT Signing ---

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

    // --- Encryption Interface ---

    /// Encrypt data with AAD (Additional Authenticated Data).
    ///
    /// # Errors
    /// Returns `AppError` if encryption fails.
    fn encrypt_data_with_aad(&self, data: &str, aad: &str) -> AppResult<String>;

    /// Decrypt data with AAD (Additional Authenticated Data).
    ///
    /// # Errors
    /// Returns `AppError` if decryption fails or AAD mismatch.
    fn decrypt_data_with_aad(&self, encrypted: &str, aad: &str) -> AppResult<String>;

    // --- Key Rotation & Security ---

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

    // --- System Secret Management ---

    /// Get or create system secret (generates if not exists)
    async fn get_or_create_system_secret(&self, secret_type: &str) -> AppResult<String>;

    /// Get existing system secret
    async fn get_system_secret(&self, secret_type: &str) -> AppResult<String>;

    /// Update system secret (for rotation)
    async fn update_system_secret(&self, secret_type: &str, new_value: &str) -> AppResult<()>;

    // --- OAuth Notifications ---

    /// Store OAuth completion notification for MCP resource delivery
    async fn store_oauth_notification(
        &self,
        user_id: Uuid,
        provider: &str,
        success: bool,
        message: &str,
        expires_at: Option<&str>,
    ) -> AppResult<String>;

    /// Get unread OAuth notifications for a user
    async fn get_unread_oauth_notifications(
        &self,
        user_id: Uuid,
    ) -> AppResult<Vec<OAuthNotification>>;

    /// Mark OAuth notification as read
    async fn mark_oauth_notification_read(
        &self,
        notification_id: &str,
        user_id: Uuid,
    ) -> AppResult<bool>;

    /// Mark all OAuth notifications as read for a user
    async fn mark_all_oauth_notifications_read(&self, user_id: Uuid) -> AppResult<u64>;

    /// Get all OAuth notifications for a user (read and unread)
    async fn get_all_oauth_notifications(
        &self,
        user_id: Uuid,
        limit: Option<i64>,
    ) -> AppResult<Vec<OAuthNotification>>;
}
