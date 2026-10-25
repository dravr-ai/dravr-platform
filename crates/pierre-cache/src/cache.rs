// ABOUTME: Unified Cache wrapper dispatching to in-memory or Redis backends
// ABOUTME: Provides a single concrete type generic callers can hold via CacheProvider
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::time::Duration;

use async_trait::async_trait;
use secrecy::ExposeSecret;
use serde::{Deserialize, Serialize};
use tracing::info;
use uuid::Uuid;

use crate::errors::AppResult;
use crate::memory::InMemoryCache;
use crate::redaction::redact_url;
#[cfg(feature = "redis")]
use crate::redis_backend::RedisCache;
use crate::{CacheConfig, CacheKey, CacheProvider, WindowCount};

/// Cache backend enum for pluggable implementations
#[non_exhaustive]
#[derive(Clone, Debug)]
enum CacheBackend {
    InMemory(InMemoryCache),
    #[cfg(feature = "redis")]
    Redis(Box<RedisCache>),
}

/// Unified cache interface supporting both in-memory and Redis backends
#[derive(Clone, Debug)]
pub struct Cache {
    inner: CacheBackend,
}

impl Cache {
    /// Create cache instance based on configuration
    ///
    /// # Errors
    ///
    /// Returns an error if cache initialization fails
    pub async fn new(config: CacheConfig) -> AppResult<Self> {
        let inner = if let Some(ref redis_url) = config.redis_url {
            info!(
                "Initializing Redis cache (url: {})",
                redact_url(redis_url.expose_secret())
            );
            Self::build_redis_backend(config).await?
        } else {
            info!(
                "Initializing in-memory cache (max entries: {})",
                config.max_entries
            );
            let memory = InMemoryCache::new(config).await?;
            CacheBackend::InMemory(memory)
        };

        Ok(Self { inner })
    }

    #[cfg(feature = "redis")]
    async fn build_redis_backend(config: CacheConfig) -> AppResult<CacheBackend> {
        let redis = RedisCache::new(config).await?;
        Ok(CacheBackend::Redis(Box::new(redis)))
    }

    #[cfg(not(feature = "redis"))]
    async fn build_redis_backend(config: CacheConfig) -> AppResult<CacheBackend> {
        // Redis URL set but redis feature disabled — fall back to in-memory with a warning.
        tracing::warn!(
            "REDIS_URL provided but `redis` feature is disabled; falling back to in-memory cache"
        );
        let memory = InMemoryCache::new(config).await?;
        Ok(CacheBackend::InMemory(memory))
    }

    /// Store value in cache with TTL
    ///
    /// # Errors
    ///
    /// Returns an error if serialization or storage fails
    pub async fn set<T: Serialize + Send + Sync>(
        &self,
        key: &CacheKey,
        value: &T,
        ttl: Duration,
    ) -> AppResult<()> {
        match &self.inner {
            CacheBackend::InMemory(cache) => cache.set(key, value, ttl).await,
            #[cfg(feature = "redis")]
            CacheBackend::Redis(cache) => cache.set(key, value, ttl).await,
        }
    }

    /// Retrieve value from cache
    ///
    /// # Errors
    ///
    /// Returns an error if deserialization fails
    pub async fn get<T: for<'de> Deserialize<'de>>(&self, key: &CacheKey) -> AppResult<Option<T>> {
        match &self.inner {
            CacheBackend::InMemory(cache) => cache.get(key).await,
            #[cfg(feature = "redis")]
            CacheBackend::Redis(cache) => cache.get(key).await,
        }
    }

    /// Remove single cache entry
    ///
    /// # Errors
    ///
    /// Returns an error if invalidation fails
    pub async fn invalidate(&self, key: &CacheKey) -> AppResult<()> {
        match &self.inner {
            CacheBackend::InMemory(cache) => cache.invalidate(key).await,
            #[cfg(feature = "redis")]
            CacheBackend::Redis(cache) => cache.invalidate(key).await,
        }
    }

    /// Remove all cache entries matching pattern
    ///
    /// # Errors
    ///
    /// Returns an error if pattern invalidation fails
    pub async fn invalidate_pattern(&self, pattern: &str) -> AppResult<u64> {
        match &self.inner {
            CacheBackend::InMemory(cache) => cache.invalidate_pattern(pattern).await,
            #[cfg(feature = "redis")]
            CacheBackend::Redis(cache) => cache.invalidate_pattern(pattern).await,
        }
    }

    /// Remove every entry cached for `user_id`, in every tenant and for every
    /// provider ([`CacheKey::every_entry_of_user`]), on whichever backend this
    /// cache runs. Returns how many entries were removed.
    ///
    /// The deleted-account path calls it: an account that no longer exists
    /// must leave no response cached under its id.
    ///
    /// # Errors
    ///
    /// Returns an error if the backend cannot be reached
    pub async fn invalidate_user(&self, user_id: Uuid) -> AppResult<u64> {
        self.invalidate_pattern(&CacheKey::every_entry_of_user(user_id))
            .await
    }

    /// Check if key exists in cache
    ///
    /// # Errors
    ///
    /// Returns an error if existence check fails
    pub async fn exists(&self, key: &CacheKey) -> AppResult<bool> {
        match &self.inner {
            CacheBackend::InMemory(cache) => cache.exists(key).await,
            #[cfg(feature = "redis")]
            CacheBackend::Redis(cache) => cache.exists(key).await,
        }
    }

    /// Get remaining TTL for key
    ///
    /// # Errors
    ///
    /// Returns an error if TTL check fails
    pub async fn ttl(&self, key: &CacheKey) -> AppResult<Option<Duration>> {
        match &self.inner {
            CacheBackend::InMemory(cache) => cache.ttl(key).await,
            #[cfg(feature = "redis")]
            CacheBackend::Redis(cache) => cache.ttl(key).await,
        }
    }

    /// Whether every process connected to this cache's backend sees the same
    /// entries (Redis), rather than this process alone (in-memory)
    #[must_use]
    pub const fn is_shared_across_processes(&self) -> bool {
        match &self.inner {
            CacheBackend::InMemory(_) => false,
            #[cfg(feature = "redis")]
            CacheBackend::Redis(_) => true,
        }
    }

    /// Count one hit against the fixed window counted at `key`
    ///
    /// # Errors
    ///
    /// Returns an error if the backend cannot be reached, or if `key` holds a
    /// value that is not a window counter
    pub async fn count_in_window(
        &self,
        key: &CacheKey,
        window: Duration,
    ) -> AppResult<WindowCount> {
        match &self.inner {
            CacheBackend::InMemory(cache) => cache.count_in_window(key, window).await,
            #[cfg(feature = "redis")]
            CacheBackend::Redis(cache) => cache.count_in_window(key, window).await,
        }
    }

    /// Take back one hit counted at `key`, never below zero
    ///
    /// # Errors
    ///
    /// Returns an error if the backend cannot be reached, or if `key` holds a
    /// value that is not a window counter
    pub async fn uncount_in_window(&self, key: &CacheKey) -> AppResult<()> {
        match &self.inner {
            CacheBackend::InMemory(cache) => cache.uncount_in_window(key).await,
            #[cfg(feature = "redis")]
            CacheBackend::Redis(cache) => cache.uncount_in_window(key).await,
        }
    }

    /// Verify cache backend is healthy
    ///
    /// # Errors
    ///
    /// Returns an error if health check fails
    pub async fn health_check(&self) -> AppResult<()> {
        match &self.inner {
            CacheBackend::InMemory(cache) => cache.health_check().await,
            #[cfg(feature = "redis")]
            CacheBackend::Redis(cache) => cache.health_check().await,
        }
    }

    /// Clear all cache entries
    ///
    /// # Errors
    ///
    /// Returns an error if clear operation fails
    pub async fn clear_all(&self) -> AppResult<()> {
        match &self.inner {
            CacheBackend::InMemory(cache) => cache.clear_all().await,
            #[cfg(feature = "redis")]
            CacheBackend::Redis(cache) => cache.clear_all().await,
        }
    }
}

/// Expose the same dispatch surface as the inherent impl through the canonical
/// [`CacheProvider`] trait. Generic callers (e.g. the middleware layer)
/// constrain on `C: CacheProvider` and accept an `Arc<Cache>` via type
/// inference without depending on a concrete backend type.
#[async_trait]
impl CacheProvider for Cache {
    async fn new(config: CacheConfig) -> AppResult<Self> {
        Self::new(config).await
    }

    async fn set<T: Serialize + Send + Sync>(
        &self,
        key: &CacheKey,
        value: &T,
        ttl: Duration,
    ) -> AppResult<()> {
        Self::set(self, key, value, ttl).await
    }

    async fn get<T: for<'de> Deserialize<'de>>(&self, key: &CacheKey) -> AppResult<Option<T>> {
        Self::get(self, key).await
    }

    async fn invalidate(&self, key: &CacheKey) -> AppResult<()> {
        Self::invalidate(self, key).await
    }

    async fn invalidate_pattern(&self, pattern: &str) -> AppResult<u64> {
        Self::invalidate_pattern(self, pattern).await
    }

    async fn exists(&self, key: &CacheKey) -> AppResult<bool> {
        Self::exists(self, key).await
    }

    async fn ttl(&self, key: &CacheKey) -> AppResult<Option<Duration>> {
        Self::ttl(self, key).await
    }

    async fn count_in_window(&self, key: &CacheKey, window: Duration) -> AppResult<WindowCount> {
        Self::count_in_window(self, key, window).await
    }

    async fn uncount_in_window(&self, key: &CacheKey) -> AppResult<()> {
        Self::uncount_in_window(self, key).await
    }

    async fn health_check(&self) -> AppResult<()> {
        Self::health_check(self).await
    }

    async fn clear_all(&self) -> AppResult<()> {
        Self::clear_all(self).await
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use uuid::Uuid;

    use super::Cache;
    use crate::models::TenantId;
    use crate::{CacheConfig, CacheKey, CacheResource};

    fn key(tenant_id: TenantId, user_id: Uuid, provider: &str) -> CacheKey {
        CacheKey::new(
            tenant_id,
            user_id,
            provider.to_owned(),
            CacheResource::AthleteProfile,
        )
    }

    /// Every entry of the deleted user goes, in every tenant and for every
    /// provider, and nobody else's entry is touched.
    #[tokio::test]
    async fn invalidating_a_user_removes_their_entries_in_every_tenant_only() {
        let cache = Cache::new(CacheConfig {
            enable_background_cleanup: false,
            ..CacheConfig::default()
        })
        .await
        .unwrap();
        let (deleted, kept) = (Uuid::new_v4(), Uuid::new_v4());
        let (tenant_a, tenant_b) = (TenantId::generate(), TenantId::generate());
        let ttl = Duration::from_secs(60);
        for entry in [
            key(tenant_a, deleted, "strava"),
            key(tenant_b, deleted, "whoop"),
            key(tenant_a, kept, "strava"),
        ] {
            cache.set(&entry, &"cached", ttl).await.unwrap();
        }

        assert_eq!(cache.invalidate_user(deleted).await.unwrap(), 2);

        assert!(!cache
            .exists(&key(tenant_a, deleted, "strava"))
            .await
            .unwrap());
        assert!(!cache
            .exists(&key(tenant_b, deleted, "whoop"))
            .await
            .unwrap());
        assert!(cache.exists(&key(tenant_a, kept, "strava")).await.unwrap());
        assert_eq!(cache.invalidate_user(deleted).await.unwrap(), 0);
    }
}
