// ABOUTME: Cache abstraction layer for API response caching with tenant isolation
// ABOUTME: Pluggable backend support (in-memory, Redis) following DatabaseProvider pattern
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Pierre Cache Layer
//!
//! Provides a pluggable cache abstraction with support for in-memory (LRU) and Redis
//! backends.  The crate re-exports `pierre_core` modules so that moved files can keep
//! `use crate::errors::*` unchanged.
//!
//! ## Feature flags
//!
//! * **`redis`** — enables the Redis-backed cache implementation.

#![deny(unsafe_code)]

// Re-export pierre-core modules so `crate::errors`, `crate::models`, `crate::constants`
// resolve inside this crate (same pattern as pierre-providers / pierre-llm).
pub use pierre_core::constants;
pub use pierre_core::errors;
pub use pierre_core::models;

/// In-memory cache implementation
pub mod memory;

/// Redis cache implementation (requires `redis` feature)
#[cfg(feature = "redis")]
pub mod redis_backend;

/// URL redaction utility for safe logging of connection strings
pub mod redaction;

/// Redis connection and retry configuration
pub mod redis_config;

/// Unified `Cache` wrapper dispatching to in-memory or Redis backends
mod cache;

pub use cache::Cache;

use constants::cache::{
    DEFAULT_CACHE_MAX_ENTRIES, DEFAULT_CLEANUP_INTERVAL_SECS, TTL_ACTIVITY_LIST_SECS,
    TTL_ACTIVITY_SECS, TTL_PROFILE_SECS, TTL_PROVIDER_ROSTER_SECS, TTL_STATS_SECS,
    TTL_TRAINING_HISTORY_SECS,
};
use errors::AppResult;
use models::TenantId;
use redis_config::RedisConnectionConfig;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::time::Duration;
use uuid::Uuid;

/// Cache provider trait for pluggable backend implementations
///
/// Provides a storage-agnostic interface for caching API responses with
/// tenant-isolated keys and configurable TTLs. Implementations include
/// an in-memory LRU cache ([`memory::MemoryCache`]) and an optional
/// Redis-backed cache (requires the `redis` feature).
///
/// # Usage
///
/// ```rust,no_run
/// use pierre_cache::{CacheProvider, CacheConfig, CacheKey, CacheResource};
/// use pierre_cache::memory::InMemoryCache;
/// use pierre_core::models::TenantId;
/// use std::time::Duration;
/// use uuid::Uuid;
///
/// # async fn example() -> pierre_core::errors::AppResult<()> {
/// let cache = InMemoryCache::new(CacheConfig::default()).await?;
///
/// let key = CacheKey::new(
///     TenantId::generate(),
///     Uuid::new_v4(),
///     "strava".to_owned(),
///     CacheResource::AthleteProfile,
/// );
///
/// // Store a value with 1-hour TTL
/// cache.set(&key, &"cached_data", Duration::from_hours(1)).await?;
///
/// // Retrieve the value
/// let value: Option<String> = cache.get(&key).await?;
/// # Ok(())
/// # }
/// ```
#[async_trait::async_trait]
pub trait CacheProvider: Send + Sync + Clone {
    /// Create cache instance with configuration
    ///
    /// # Errors
    ///
    /// Returns an error if cache initialization fails
    async fn new(config: CacheConfig) -> AppResult<Self>
    where
        Self: Sized;

    /// Store value in cache with TTL
    ///
    /// # Errors
    ///
    /// Returns an error if serialization or storage fails
    async fn set<T: Serialize + Send + Sync>(
        &self,
        key: &CacheKey,
        value: &T,
        ttl: Duration,
    ) -> AppResult<()>;

    /// Retrieve value from cache
    ///
    /// # Errors
    ///
    /// Returns an error if deserialization fails
    async fn get<T: for<'de> Deserialize<'de>>(&self, key: &CacheKey) -> AppResult<Option<T>>;

    /// Remove single cache entry
    ///
    /// # Errors
    ///
    /// Returns an error if invalidation fails
    async fn invalidate(&self, key: &CacheKey) -> AppResult<()>;

    /// Remove all cache entries matching pattern (e.g., "tenant:*:strava:*")
    ///
    /// # Errors
    ///
    /// Returns an error if pattern invalidation fails
    async fn invalidate_pattern(&self, pattern: &str) -> AppResult<u64>;

    /// Check if key exists in cache
    ///
    /// # Errors
    ///
    /// Returns an error if existence check fails
    async fn exists(&self, key: &CacheKey) -> AppResult<bool>;

    /// Get remaining TTL for key
    ///
    /// # Errors
    ///
    /// Returns an error if TTL check fails
    async fn ttl(&self, key: &CacheKey) -> AppResult<Option<Duration>>;

    /// Count one hit against the fixed window counted at `key`.
    ///
    /// The first hit creates the counter to live for `window`; later hits
    /// increment it without extending that lifetime, so the window closes
    /// `window` after its first hit and the next hit opens a fresh one. The
    /// increment and the expiry are one atomic step in every backend, so on
    /// Redis every process sharing the instance counts into the same window
    /// and no concurrent hit is lost.
    ///
    /// # Errors
    ///
    /// Returns an error if the backend cannot be reached, or if `key` holds a
    /// value that is not a window counter
    async fn count_in_window(&self, key: &CacheKey, window: Duration) -> AppResult<WindowCount>;

    /// Verify cache backend is healthy
    ///
    /// # Errors
    ///
    /// Returns an error if health check fails
    async fn health_check(&self) -> AppResult<()>;

    /// Clear all cache entries (for testing/admin)
    ///
    /// # Errors
    ///
    /// Returns an error if clear operation fails
    async fn clear_all(&self) -> AppResult<()>;
}

/// A fixed window's count once one more hit is counted, as
/// [`CacheProvider::count_in_window`] reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowCount {
    /// Hits counted in the window, this one included
    pub hits: u64,
    /// Time left before the window closes and its count starts over
    pub resets_in: Duration,
}

/// Cache configuration
#[derive(Debug, Clone)]
pub struct CacheConfig {
    /// Maximum number of entries (for in-memory cache)
    pub max_entries: usize,
    /// Redis connection URL (for Redis cache)
    pub redis_url: Option<String>,
    /// Cleanup interval for expired entries
    pub cleanup_interval: Duration,
    /// Enable background cleanup task (should be false in tests to avoid runtime conflicts)
    pub enable_background_cleanup: bool,
    /// Redis connection and retry configuration
    pub redis_connection: RedisConnectionConfig,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            max_entries: DEFAULT_CACHE_MAX_ENTRIES,
            redis_url: None,
            cleanup_interval: Duration::from_secs(DEFAULT_CLEANUP_INTERVAL_SECS),
            // Default to enabled - production code should use background cleanup
            // Tests can explicitly disable by setting to false
            enable_background_cleanup: true,
            redis_connection: RedisConnectionConfig::default(),
        }
    }
}

/// Structured cache key with tenant and user isolation
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CacheKey {
    /// Tenant ID for multi-tenant isolation
    pub tenant_id: TenantId,
    /// User ID for per-user isolation
    pub user_id: Uuid,
    /// OAuth provider name
    pub provider: String,
    /// Specific resource being cached
    pub resource: CacheResource,
}

impl CacheKey {
    /// Create cache key
    #[must_use]
    pub const fn new(
        tenant_id: TenantId,
        user_id: Uuid,
        provider: String,
        resource: CacheResource,
    ) -> Self {
        Self {
            tenant_id,
            user_id,
            provider,
            resource,
        }
    }

    /// Create pattern for invalidating all entries for a user
    #[must_use]
    pub fn user_pattern(tenant_id: TenantId, user_id: Uuid, provider: &str) -> String {
        format!("tenant:{tenant_id}:user:{user_id}:provider:{provider}:*")
    }

    /// Create pattern for invalidating all entries for a tenant
    #[must_use]
    pub fn tenant_pattern(tenant_id: TenantId, provider: &str) -> String {
        format!("tenant:{tenant_id}:*:provider:{provider}:*")
    }
}

impl fmt::Display for CacheKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "tenant:{}:user:{}:provider:{}:{}",
            self.tenant_id, self.user_id, self.provider, self.resource
        )
    }
}

/// Cache resource types with specific parameters
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CacheResource {
    /// Athlete profile (24h TTL)
    AthleteProfile,
    /// Activity list with pagination and optional time/sport filters (1 h TTL)
    ActivityList {
        /// Page number for pagination
        page: u32,
        /// Items per page
        per_page: u32,
        /// Optional Unix timestamp (seconds) - return activities before this time
        before: Option<i64>,
        /// Optional Unix timestamp (seconds) - return activities after this time
        after: Option<i64>,
        /// Optional sport type filter (e.g., "run", "ride") for server-side filtering
        sport_type: Option<String>,
    },
    /// Single activity summary (1h TTL)
    Activity {
        /// Activity ID
        activity_id: u64,
    },
    /// Athlete statistics (6h TTL)
    Stats {
        /// Athlete ID
        athlete_id: u64,
    },
    /// Detailed activity with streams (1h TTL)
    DetailedActivity {
        /// Activity ID
        activity_id: u64,
    },
    /// Endurance daily training-history rollup result (`get_training_history`).
    /// Per-(tenant,user) and per-date-range; invalidated whenever the
    /// underlying `training_history` rows are upserted by
    /// `training_history_compute`.
    TrainingHistory {
        /// Inclusive ISO date (YYYY-MM-DD) of the start of the requested range.
        from: String,
        /// Inclusive ISO date (YYYY-MM-DD) of the end of the requested range.
        to: String,
    },
    /// The athletes a coach account coaches on the provider, as the provider's
    /// roster reports them (10 min TTL). Keyed by the coach's own tenant and
    /// user, so one coach's roster never answers for another's.
    ProviderRoster,
    /// The sciotte login flow an athlete has parked on the scraper service,
    /// awaiting its OTP/2FA continuation. Shared rather than process-local so a
    /// continuation reaches the flow whichever pod it lands on. TTL is
    /// caller-specified because it has to track the scraper service's own
    /// deployed parked-flow lifetime (`DRAVR_SCIOTTE_PARKED_PERMIT_TTL_SECS`),
    /// a knob this crate has no visibility into — so `recommended_ttl` returns
    /// 0 for it, as it does for [`CacheResource::Custom`].
    SciotteLoginFlow,
    /// General-purpose keyed resource for non-domain uses (nonces, rate limits, etc.).
    /// TTL is caller-specified — `recommended_ttl` returns 0 for this variant.
    Custom(String),
}

impl CacheResource {
    /// Get recommended TTL for this resource type
    #[must_use]
    pub const fn recommended_ttl(&self) -> Duration {
        match self {
            Self::AthleteProfile => Duration::from_secs(TTL_PROFILE_SECS),
            Self::ActivityList { .. } => Duration::from_secs(TTL_ACTIVITY_LIST_SECS),
            Self::Activity { .. } | Self::DetailedActivity { .. } => {
                Duration::from_secs(TTL_ACTIVITY_SECS)
            }
            Self::Stats { .. } => Duration::from_secs(TTL_STATS_SECS),
            Self::TrainingHistory { .. } => Duration::from_secs(TTL_TRAINING_HISTORY_SECS),
            Self::ProviderRoster => Duration::from_secs(TTL_PROVIDER_ROSTER_SECS),
            Self::SciotteLoginFlow | Self::Custom(_) => Duration::ZERO,
        }
    }
}

impl fmt::Display for CacheResource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AthleteProfile => write!(f, "athlete_profile"),
            Self::ActivityList {
                page,
                per_page,
                before,
                after,
                sport_type,
            } => {
                let before_str = before.map_or(String::new(), |t| format!(":before:{t}"));
                let after_str = after.map_or(String::new(), |t| format!(":after:{t}"));
                let sport_str = sport_type
                    .as_ref()
                    .map_or(String::new(), |s| format!(":sport:{s}"));
                write!(
                    f,
                    "activity_list:page:{page}:per_page:{per_page}{before_str}{after_str}{sport_str}"
                )
            }
            Self::Activity { activity_id } => write!(f, "activity:{activity_id}"),
            Self::Stats { athlete_id } => write!(f, "stats:{athlete_id}"),
            Self::DetailedActivity { activity_id } => {
                write!(f, "detailed_activity:{activity_id}")
            }
            Self::TrainingHistory { from, to } => {
                write!(f, "training_history:{from}:{to}")
            }
            Self::ProviderRoster => write!(f, "provider_roster"),
            Self::SciotteLoginFlow => write!(f, "sciotte_login_flow"),
            Self::Custom(ref key) => write!(f, "custom:{key}"),
        }
    }
}
