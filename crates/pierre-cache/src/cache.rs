// ABOUTME: Unified Cache wrapper dispatching to in-memory or Redis backends
// ABOUTME: Provides a single concrete type generic callers can hold via CacheProvider
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::errors::AppResult;
use crate::memory::InMemoryCache;
use crate::redaction::redact_url;
#[cfg(feature = "redis")]
use crate::redis_backend::RedisCache;
use crate::{CacheConfig, CacheKey, CacheProvider};

/// Cache backend enum for pluggable implementations
#[non_exhaustive]
#[derive(Clone)]
enum CacheBackend {
    InMemory(InMemoryCache),
    #[cfg(feature = "redis")]
    Redis(Box<RedisCache>),
}

/// Unified cache interface supporting both in-memory and Redis backends
#[derive(Clone)]
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
            info!("Initializing Redis cache (url: {})", redact_url(redis_url));
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

    async fn health_check(&self) -> AppResult<()> {
        Self::health_check(self).await
    }

    async fn clear_all(&self) -> AppResult<()> {
        Self::clear_all(self).await
    }
}
