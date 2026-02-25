// ABOUTME: Cache abstraction layer re-export and environment-based factory
// ABOUTME: Delegates to pierre-cache crate; keeps server-config-aware factory here
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// Re-export everything from pierre-cache
pub use pierre_cache::memory;
pub use pierre_cache::redaction;
pub use pierre_cache::redis_backend as redis;
pub use pierre_cache::redis_config;
pub use pierre_cache::*;

use crate::config::admin::service::AdminConfigService;
use crate::errors::AppResult;

/// Cache factory for creating cache providers
pub mod factory;

// Bridge: implement pierre-cache's CacheTtlConfigProvider for AdminConfigService
#[async_trait::async_trait]
impl CacheTtlConfigProvider for AdminConfigService {
    async fn get_value(
        &self,
        key: &str,
        tenant_id: Option<&str>,
    ) -> AppResult<Option<serde_json::Value>> {
        Self::get_value(self, key, tenant_id).await
    }
}

/// Create TTL config from admin configuration service
///
/// Loads TTL values from the admin config service, falling back to defaults
/// if values are not configured or retrieval fails.
pub async fn cache_ttl_from_admin_config(
    admin_config: &AdminConfigService,
    tenant_id: Option<&str>,
) -> CacheTtlConfig {
    CacheTtlConfig::from_config_provider(admin_config, tenant_id).await
}
