// ABOUTME: OAuth2 endpoint rate limiting: one window per endpoint per client address, shared across replicas on Redis
// ABOUTME: Reports each window's limit, remaining allowance and reset for the 429 refusal and its Retry-After
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use http::HeaderMap;
use pierre_cache::memory::InMemoryCache;
use pierre_cache::{Cache, CacheConfig, CacheKey, CacheProvider, CacheResource, WindowCount};
use pierre_core::errors::AppResult;
use pierre_core::models::TenantId;
use tracing::warn;
use uuid::Uuid;

use crate::client_address::{metering_key, TrustedProxies};
use crate::config::rate_limit::RateLimitConfig;
use crate::rate_limiting::{OAuth2Endpoint, OAuth2RateLimitConfig, OAuth2RateLimitStatus};

/// The `provider` segment of every window's cache key, which keeps the windows
/// apart from the provider data and link-token state sharing a Redis cache.
const WINDOW_KEY_NAMESPACE: &str = "_oauth2_rate_limit";

/// Windows the process-local store holds before it evicts the least recently
/// used one. An expired window reads as absent, so the store needs no sweep;
/// the bound is what caps its memory.
const LOCAL_WINDOW_CAPACITY: usize = 10_000;

/// Per-address rate limiter for the `OAuth2` endpoints.
///
/// Every endpoint has its own fixed window per client address, so requests to
/// `/oauth2/token` never spend `/oauth2/register`'s or `/oauth2/authorize`'s
/// allowance. A window opens at its first request and lasts
/// `rate_limit_window_secs`.
///
/// With a shared store (the server's Redis cache) every replica counts into
/// the same window. When that store cannot count a request, the request is
/// counted in this process's own store instead, so an outage of the shared
/// store meters per replica rather than refusing sign-in. Without a shared
/// store the windows live only in the process's own store, a bounded LRU of
/// their own: never in the server's in-memory cache, whose entries include the
/// link-token burn markers that unauthenticated traffic must not be able to
/// evict.
#[derive(Clone)]
pub struct OAuth2RateLimiter {
    /// The store every replica counts into; an `Arc` because it is the
    /// server's one Redis-backed cache, shared with every handler that caches
    shared: Option<Arc<Cache>>,
    /// This process's windows: all of them without a shared store, and the
    /// ones the shared store could not count
    local: InMemoryCache,
    /// The proxies whose `X-Forwarded-For` entries are read past to the client
    trusted_proxies: TrustedProxies,
    /// Requests each endpoint admits per window
    limits: OAuth2RateLimitConfig,
    /// How long a window lasts from its first request
    window: Duration,
}

impl OAuth2RateLimiter {
    /// A process-local window store sized for the limiter.
    #[must_use]
    pub fn local_window_store() -> InMemoryCache {
        InMemoryCache::new_with_config(&CacheConfig {
            max_entries: LOCAL_WINDOW_CAPACITY,
            enable_background_cleanup: false,
            ..CacheConfig::default()
        })
    }

    /// A limiter counting in `shared` when there is one and in `local`
    /// otherwise, with the limits, window and trusted proxies of `config`.
    #[must_use]
    pub fn new(shared: Option<Arc<Cache>>, local: InMemoryCache, config: &RateLimitConfig) -> Self {
        Self {
            shared,
            local,
            trusted_proxies: config.trusted_proxies.clone(),
            limits: OAuth2RateLimitConfig::from_rate_limit_config(config),
            window: Duration::from_secs(config.rate_limit_window_secs),
        }
    }

    /// The client a request from TCP peer `peer` carrying `headers` came from
    /// (see [`TrustedProxies::client_address`]).
    #[must_use]
    pub fn client_address(&self, peer: IpAddr, headers: &HeaderMap) -> IpAddr {
        self.trusted_proxies.client_address(peer, headers)
    }

    /// Count one request to `endpoint` from `client_ip` and report its window.
    ///
    /// The request is counted whether or not it is admitted; `remaining` is
    /// the allowance the window had left before it. IPv6 clients in one /64
    /// share a window.
    ///
    /// # Errors
    ///
    /// Returns an error when neither store can count the request: the window's
    /// key holds something other than a count.
    pub async fn check_rate_limit(
        &self,
        endpoint: OAuth2Endpoint,
        client_ip: IpAddr,
    ) -> AppResult<OAuth2RateLimitStatus> {
        let limit = self.limits.get_limit(endpoint);
        let counted = self
            .count(endpoint, &Self::window_key(endpoint, client_ip))
            .await?;

        // Requests the window counted before this one.
        let before = u32::try_from(counted.hits.saturating_sub(1)).unwrap_or(u32::MAX);
        let reset_at = (SystemTime::now() + counted.resets_in)
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since_epoch| {
                i64::try_from(since_epoch.as_secs()).unwrap_or(i64::MAX)
            });

        Ok(OAuth2RateLimitStatus {
            is_limited: before >= limit,
            limit,
            remaining: limit.saturating_sub(before),
            reset_at,
            retry_after_seconds: None,
        }
        .with_retry_after())
    }

    /// Count one hit at `key`: in the shared store when there is one and it
    /// answers, else in this process's.
    async fn count(&self, endpoint: OAuth2Endpoint, key: &CacheKey) -> AppResult<WindowCount> {
        if let Some(shared) = &self.shared {
            match shared.count_in_window(key, self.window).await {
                Ok(counted) => return Ok(counted),
                Err(failure) => warn!(
                    endpoint = endpoint.as_str(),
                    error = %failure,
                    "OAuth2 rate limiter's shared store could not count the request; \
                     counting it in this process's window"
                ),
            }
        }
        self.local.count_in_window(key, self.window).await
    }

    /// The cache key of `endpoint`'s window for `client_ip`
    fn window_key(endpoint: OAuth2Endpoint, client_ip: IpAddr) -> CacheKey {
        CacheKey::new(
            TenantId::nil(),
            Uuid::nil(),
            WINDOW_KEY_NAMESPACE.to_owned(),
            CacheResource::Custom(format!("{}:{}", endpoint.as_str(), metering_key(client_ip))),
        )
    }
}
