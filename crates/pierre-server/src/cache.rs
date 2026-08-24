// ABOUTME: Server-only cache helpers: env-driven factory + AdminConfigService TTL bridge
// ABOUTME: Cache types and CacheProvider trait now live in pierre_cache::*
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::time::Duration;

use pierre_cache::redis_config::RedisConnectionConfig;
use pierre_cache::{Cache, CacheConfig, CacheTtlConfig, CacheTtlConfigProvider};
use pierre_core::errors::AppResult;
use pierre_runtime_context::ConfigLookupScope;

use crate::config::admin::service::AdminConfigService;
use crate::constants::get_server_config;

/// Build a [`Cache`] from environment-driven server configuration.
///
/// Supports both in-memory and Redis backends based on the `REDIS_URL` server
/// config field. Uses sensible defaults if server configuration is not yet
/// initialized (e.g. in tests that bypass `init_server_config`).
///
/// # Errors
///
/// Returns an error if cache initialization fails.
pub async fn cache_from_env() -> AppResult<Cache> {
    let config = get_server_config().map_or_else(
        || CacheConfig {
            max_entries: 1000,
            redis_url: None,
            cleanup_interval: Duration::from_mins(5),
            enable_background_cleanup: true,
            redis_connection: RedisConnectionConfig::default(),
            ttl: CacheTtlConfig::default(),
        },
        |server_config| CacheConfig {
            max_entries: server_config.cache.max_entries,
            redis_url: server_config.cache.redis_url.clone(),
            cleanup_interval: Duration::from_secs(server_config.cache.cleanup_interval_secs),
            enable_background_cleanup: true,
            redis_connection: server_config.cache.redis_connection.clone(),
            ttl: CacheTtlConfig {
                profile_secs: server_config.cache.ttl.profile_secs,
                activity_list_secs: server_config.cache.ttl.activity_list_secs,
                activity_secs: server_config.cache.ttl.activity_secs,
                stats_secs: server_config.cache.ttl.stats_secs,
            },
        },
    );

    Cache::new(config).await
}

// Bridge: implement pierre-cache's CacheTtlConfigProvider for AdminConfigService.
// AdminConfigService is server-internal, so this impl must live here.
#[async_trait::async_trait]
impl CacheTtlConfigProvider for AdminConfigService {
    async fn get_value(
        &self,
        key: &str,
        tenant_id: Option<&str>,
    ) -> AppResult<Option<serde_json::Value>> {
        Self::get_value(
            self,
            key,
            ConfigLookupScope {
                user_id: None,
                tenant_id,
            },
        )
        .await
    }
}

/// Create TTL config from admin configuration service.
///
/// Loads TTL values from the admin config service, falling back to defaults
/// if values are not configured or retrieval fails.
pub async fn cache_ttl_from_admin_config(
    admin_config: &AdminConfigService,
    tenant_id: Option<&str>,
) -> CacheTtlConfig {
    CacheTtlConfig::from_config_provider(admin_config, tenant_id).await
}
