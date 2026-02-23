// ABOUTME: Security repository trait implementation for RSA keypairs, key rotation, and audit events
// ABOUTME: Delegates to inherent Database methods for cryptographic key management and system secrets
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::Database;
use crate::errors::AppResult;
use crate::security::audit::AuditEvent;
use crate::security::key_rotation::KeyVersion;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::models::TenantId;
use pierre_database::repositories::SecurityRepository;

#[async_trait]
impl SecurityRepository for Database {
    async fn save_rsa_keypair(
        &self,
        kid: &str,
        private_key_pem: &str,
        public_key_pem: &str,
        created_at: DateTime<Utc>,
        is_active: bool,
        key_size_bits: i32,
    ) -> AppResult<()> {
        Self::save_rsa_keypair(
            self,
            kid,
            private_key_pem,
            public_key_pem,
            created_at,
            is_active,
            key_size_bits.try_into().unwrap_or(2048),
        )
        .await
    }
    async fn load_rsa_keypairs(
        &self,
    ) -> AppResult<Vec<(String, String, String, DateTime<Utc>, bool)>> {
        Self::load_rsa_keypairs(self).await
    }
    async fn update_rsa_keypair_active_status(&self, kid: &str, is_active: bool) -> AppResult<()> {
        Self::update_rsa_keypair_active_status_impl(self, kid, is_active).await
    }
    async fn store_key_version(&self, version: &KeyVersion) -> AppResult<()> {
        Self::store_key_version(self, version).await
    }
    async fn get_key_versions(&self, tenant_id: Option<TenantId>) -> AppResult<Vec<KeyVersion>> {
        Self::get_key_versions(self, tenant_id).await
    }
    async fn get_current_key_version(
        &self,
        tenant_id: Option<TenantId>,
    ) -> AppResult<Option<KeyVersion>> {
        Self::get_current_key_version(self, tenant_id).await
    }
    async fn update_key_version_status(
        &self,
        tenant_id: Option<TenantId>,
        version: u32,
        is_active: bool,
    ) -> AppResult<()> {
        Self::update_key_version_status(self, tenant_id, version, is_active).await
    }
    async fn delete_old_key_versions(
        &self,
        tenant_id: Option<TenantId>,
        keep_count: u32,
    ) -> AppResult<u64> {
        Self::delete_old_key_versions(self, tenant_id, keep_count).await
    }
    async fn store_audit_event(&self, event: &AuditEvent) -> AppResult<()> {
        Self::store_audit_event_impl(self, event).await
    }
    async fn get_audit_events(
        &self,
        tenant_id: Option<TenantId>,
        event_type: Option<&str>,
        limit: Option<u32>,
    ) -> AppResult<Vec<AuditEvent>> {
        Self::get_audit_events(self, tenant_id, event_type, limit).await
    }
    async fn get_or_create_system_secret(&self, secret_type: &str) -> AppResult<String> {
        Self::get_or_create_system_secret_impl(self, secret_type).await
    }
    async fn get_system_secret(&self, secret_type: &str) -> AppResult<String> {
        Self::get_system_secret_impl(self, secret_type).await
    }
    async fn update_system_secret(&self, secret_type: &str, new_value: &str) -> AppResult<()> {
        Self::update_system_secret_impl(self, secret_type, new_value).await
    }
    fn encrypt_data_with_aad(&self, data: &str, aad: &str) -> AppResult<String> {
        Self::encrypt_data_with_aad_impl(self, data, aad)
    }
    fn decrypt_data_with_aad(&self, encrypted: &str, aad: &str) -> AppResult<String> {
        Self::decrypt_data_with_aad_impl(self, encrypted, aad)
    }
}
