// ABOUTME: In-memory cache implementation with LRU eviction and TTL support
// ABOUTME: Includes background cleanup task for expired entries
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::{Duration, Instant};

use lru::LruCache;
use serde::{Deserialize, Serialize};
use tokio::spawn;
use tokio::sync::{mpsc, RwLock};
use tokio::time::interval;
use tracing::debug;

use crate::constants::cache_config::DEFAULT_CAPACITY;
use crate::errors::{AppError, AppResult};
use crate::{CacheConfig, CacheKey, CacheProvider, WindowCount};

/// In-memory cache entry with expiration
#[derive(Debug, Clone)]
struct CacheEntry {
    data: Vec<u8>,
    expires_at: Instant,
}

impl CacheEntry {
    fn new(data: Vec<u8>, ttl: Duration) -> Self {
        Self {
            data,
            expires_at: Instant::now() + ttl,
        }
    }

    fn is_expired(&self) -> bool {
        Instant::now() >= self.expires_at
    }

    fn remaining_ttl(&self) -> Option<Duration> {
        self.expires_at.checked_duration_since(Instant::now())
    }
}

/// In-memory cache with LRU eviction and background cleanup
///
/// Uses `Arc<RwLock<LruCache>>` for shared state between cache operations and background cleanup task.
/// The Arc is required because the cleanup task (spawned in `new_with_config`) needs shared
/// ownership of the store to remove expired entries concurrently.
/// `LruCache` provides O(1) eviction by automatically removing least-recently-used entries.
#[derive(Clone, Debug)]
pub struct InMemoryCache {
    store: Arc<RwLock<LruCache<String, CacheEntry>>>,
    shutdown_tx: Option<Arc<mpsc::Sender<()>>>,
}

impl InMemoryCache {
    /// Default cache capacity when config specifies zero entries
    /// Falls back to minimum capacity (1) if constant is somehow zero
    const DEFAULT_CACHE_CAPACITY: NonZeroUsize = match NonZeroUsize::new(DEFAULT_CAPACITY) {
        Some(n) => n,
        None => NonZeroUsize::MIN,
    };

    /// Create in-memory cache with optional background cleanup task.
    ///
    /// With `enable_background_cleanup` it spawns the cleanup task, so it must
    /// then run inside a Tokio runtime.
    #[must_use]
    pub fn new_with_config(config: &CacheConfig) -> Self {
        // LruCache requires NonZeroUsize for capacity
        let capacity =
            NonZeroUsize::new(config.max_entries).unwrap_or(Self::DEFAULT_CACHE_CAPACITY);

        let store = Arc::new(RwLock::new(LruCache::new(capacity)));

        let shutdown_tx = if config.enable_background_cleanup {
            let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);
            let store_clone = store.clone();
            let cleanup_interval = config.cleanup_interval;

            spawn(async move {
                let mut interval = interval(cleanup_interval);
                loop {
                    tokio::select! {
                        _ = interval.tick() => {
                            Self::cleanup_expired(&store_clone).await;
                        }
                        _ = shutdown_rx.recv() => {
                            debug!("Cache cleanup task received shutdown signal");
                            break;
                        }
                    }
                }
            });

            Some(Arc::new(shutdown_tx))
        } else {
            None
        };

        Self { store, shutdown_tx }
    }

    /// Remove all expired entries from cache
    async fn cleanup_expired(store: &Arc<RwLock<LruCache<String, CacheEntry>>>) {
        let mut store_guard = store.write().await;

        // Collect expired keys first (can't modify while iterating)
        let expired_keys: Vec<String> = store_guard
            .iter()
            .filter_map(|(k, v)| {
                if v.is_expired() {
                    Some(k.clone()) // Safe: collect keys for removal after iteration
                } else {
                    None
                }
            })
            .collect();

        // Remove expired entries
        for key in &expired_keys {
            store_guard.pop(key);
        }

        let removed = expired_keys.len();
        drop(store_guard);
        if removed > 0 {
            debug!("Cleaned up {} expired cache entries", removed);
        }
    }
}

#[async_trait::async_trait]
impl CacheProvider for InMemoryCache {
    async fn new(config: CacheConfig) -> AppResult<Self> {
        Ok(Self::new_with_config(&config))
    }

    async fn set<T: Serialize + Send + Sync>(
        &self,
        key: &CacheKey,
        value: &T,
        ttl: Duration,
    ) -> AppResult<()> {
        let serialized = serde_json::to_vec(value)
            .map_err(|e| AppError::internal(format!("Cache serialization failed: {e}")))?;
        let entry = CacheEntry::new(serialized, ttl);

        // LruCache handles eviction automatically on push
        self.store.write().await.push(key.to_string(), entry);

        Ok(())
    }

    async fn set_if_absent<T: Serialize + Send + Sync>(
        &self,
        key: &CacheKey,
        value: &T,
        ttl: Duration,
    ) -> AppResult<bool> {
        let serialized = serde_json::to_vec(value)
            .map_err(|e| AppError::internal(format!("Cache serialization failed: {e}")))?;
        let key = key.to_string();
        let mut store = self.store.write().await;

        // One write guard covers the test and the write, so of two concurrent
        // callers in this process exactly one stores.
        if store.peek(&key).is_some_and(|entry| !entry.is_expired()) {
            drop(store);
            return Ok(false);
        }
        store.push(key, CacheEntry::new(serialized, ttl));
        drop(store);
        Ok(true)
    }

    async fn get<T: for<'de> Deserialize<'de>>(&self, key: &CacheKey) -> AppResult<Option<T>> {
        let mut store = self.store.write().await;

        // LruCache::get is mutable (updates access order for LRU)
        if let Some(entry) = store.get(&key.to_string()) {
            if entry.is_expired() {
                // Remove expired entry
                store.pop(&key.to_string());
                drop(store);
                return Ok(None);
            }

            let value: T = serde_json::from_slice(&entry.data)
                .map_err(|e| AppError::internal(format!("Cache deserialization failed: {e}")))?;
            drop(store);
            return Ok(Some(value));
        }
        drop(store);

        Ok(None)
    }

    async fn invalidate(&self, key: &CacheKey) -> AppResult<()> {
        self.store.write().await.pop(&key.to_string());
        Ok(())
    }

    async fn invalidate_pattern(&self, pattern: &str) -> AppResult<u64> {
        let mut store = self.store.write().await;

        // Use proper glob matching for cache key patterns
        // Patterns like "tenant:*:provider:strava:*" will correctly match wildcards
        let glob_pattern = glob::Pattern::new(pattern)
            .map_err(|e| AppError::internal(format!("Invalid glob pattern '{pattern}': {e}")))?;

        // Collect keys to remove (can't modify while iterating)
        let keys_to_remove: Vec<String> = store
            .iter()
            .filter_map(|(k, _)| {
                if glob_pattern.matches(k) {
                    Some(k.clone()) // Safe: collect keys for removal after iteration
                } else {
                    None
                }
            })
            .collect();

        // Remove matching keys
        for key in &keys_to_remove {
            store.pop(key);
        }

        let removed = keys_to_remove.len() as u64;
        drop(store);
        Ok(removed)
    }

    async fn exists(&self, key: &CacheKey) -> AppResult<bool> {
        let mut store = self.store.write().await;

        // LruCache::get is mutable, need write lock
        if let Some(entry) = store.get(&key.to_string()) {
            if entry.is_expired() {
                // Remove expired entry
                store.pop(&key.to_string());
                drop(store);
                return Ok(false);
            }
            drop(store);
            return Ok(true);
        }
        drop(store);

        Ok(false)
    }

    async fn ttl(&self, key: &CacheKey) -> AppResult<Option<Duration>> {
        let store = self.store.write().await;

        // Use peek to avoid updating LRU order
        if let Some(entry) = store.peek(&key.to_string()) {
            if entry.is_expired() {
                return Ok(None);
            }
            let ttl = entry.remaining_ttl();
            drop(store);
            return Ok(ttl);
        }

        Ok(None)
    }

    async fn count_in_window(&self, key: &CacheKey, window: Duration) -> AppResult<WindowCount> {
        let key = key.to_string();
        let mut store = self.store.write().await;

        // One write guard covers the read and the write, so concurrent hits in
        // this process each see the count the previous one left.
        let open_window = store.get(&key).filter(|entry| !entry.is_expired());
        let (hits, expires_at) = match open_window {
            Some(entry) => {
                let counted: u64 = serde_json::from_slice(&entry.data).map_err(|e| {
                    AppError::internal(format!("Cache entry is not a window counter: {e}"))
                })?;
                (counted.saturating_add(1), entry.expires_at)
            }
            None => (1, Instant::now() + window),
        };

        // A JSON number, as `set` would write it, so `get::<u64>` reads it back.
        let data = hits.to_string().into_bytes();
        store.push(key, CacheEntry { data, expires_at });
        drop(store);

        Ok(WindowCount {
            hits,
            resets_in: expires_at.saturating_duration_since(Instant::now()),
        })
    }

    async fn uncount_in_window(&self, key: &CacheKey) -> AppResult<()> {
        let key = key.to_string();
        let mut store = self.store.write().await;

        // One write guard covers the read and the write, as in
        // `count_in_window`.
        let Some(entry) = store.peek(&key).filter(|entry| !entry.is_expired()) else {
            return Ok(());
        };
        let counted: u64 = serde_json::from_slice(&entry.data)
            .map_err(|e| AppError::internal(format!("Cache entry is not a window counter: {e}")))?;
        let expires_at = entry.expires_at;
        let data = counted.saturating_sub(1).to_string().into_bytes();
        store.push(key, CacheEntry { data, expires_at });
        drop(store);
        Ok(())
    }

    async fn health_check(&self) -> AppResult<()> {
        // In-memory cache is always healthy
        Ok(())
    }

    async fn clear_all(&self) -> AppResult<()> {
        self.store.write().await.clear();
        Ok(())
    }
}

impl Drop for InMemoryCache {
    fn drop(&mut self) {
        // Signal background cleanup task to shutdown on drop
        // Note: This only works if the Sender is fully dropped (all Arc clones released)
        // The task will exit when all senders are dropped and recv() returns None
        if let Some(tx) = &self.shutdown_tx {
            // Try to send shutdown signal, errors are expected if channel is already closed
            if let Err(e) = tx.try_send(()) {
                debug!(error = ?e, "Cache shutdown signal send failed (channel likely closed)");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use tokio::spawn;
    use tokio::time::sleep;
    use uuid::Uuid;

    use super::InMemoryCache;
    use crate::models::TenantId;
    use crate::{CacheConfig, CacheKey, CacheProvider, CacheResource};

    fn cache() -> InMemoryCache {
        InMemoryCache::new_with_config(&CacheConfig {
            enable_background_cleanup: false,
            ..CacheConfig::default()
        })
    }

    fn key(name: &str) -> CacheKey {
        CacheKey::new(
            TenantId::nil(),
            Uuid::nil(),
            "test".to_owned(),
            CacheResource::Custom(name.to_owned()),
        )
    }

    #[tokio::test]
    async fn set_if_absent_stores_once_and_leaves_the_first_value() {
        let cache = cache();
        let key = key("once");
        assert!(cache
            .set_if_absent(&key, &"first", Duration::from_secs(60))
            .await
            .unwrap());
        assert!(
            !cache
                .set_if_absent(&key, &"second", Duration::from_secs(60))
                .await
                .unwrap(),
            "bug: a live entry must refuse the second write"
        );
        let held: Option<String> = cache.get(&key).await.unwrap();
        assert_eq!(held.as_deref(), Some("first"));
    }

    #[tokio::test]
    async fn set_if_absent_takes_a_key_whose_entry_expired() {
        let cache = cache();
        let key = key("expired");
        assert!(cache
            .set_if_absent(&key, &1_u8, Duration::from_millis(1))
            .await
            .unwrap());
        sleep(Duration::from_millis(5)).await;
        assert!(
            cache
                .set_if_absent(&key, &2_u8, Duration::from_secs(60))
                .await
                .unwrap(),
            "bug: an expired entry no longer holds its key"
        );
    }

    #[tokio::test]
    async fn exactly_one_of_many_concurrent_callers_stores() {
        let cache = Arc::new(cache());
        let mut tasks = Vec::new();
        for _ in 0..16 {
            let cache = Arc::clone(&cache);
            tasks.push(spawn(async move {
                cache
                    .set_if_absent(&key("race"), &true, Duration::from_secs(60))
                    .await
                    .unwrap()
            }));
        }
        let mut stored = 0;
        for task in tasks {
            if task.await.unwrap() {
                stored += 1;
            }
        }
        assert_eq!(stored, 1, "bug: the test and the write must be atomic");
    }
}
