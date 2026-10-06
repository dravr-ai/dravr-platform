// ABOUTME: OAuth2 endpoint rate limiting: one window per endpoint per client address (or per account), shared on Redis
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
use sha2::{Digest, Sha256};
use tracing::{error, warn};
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
        let key = Self::window_key(endpoint, &metering_key(client_ip).to_string());
        self.status(endpoint, &key).await
    }

    /// Count one attempt at `endpoint` by the signed-in account `user_id` and
    /// report its window: a window per account, wherever the requests come
    /// from, for the checks a session makes on its own account's password.
    ///
    /// # Errors
    ///
    /// Returns an error when neither store can count the attempt.
    pub async fn check_account_rate_limit(
        &self,
        endpoint: OAuth2Endpoint,
        user_id: Uuid,
    ) -> AppResult<OAuth2RateLimitStatus> {
        let key = Self::window_key(endpoint, &format!("user:{user_id}"));
        self.status(endpoint, &key).await
    }

    /// Whether a password sign-in from `client` naming `email` may be tried:
    /// `None` when the address's window and the account's both have room,
    /// `Some(seconds)` — the `Retry-After` — when either is full.
    ///
    /// Reads the windows without counting: only a refused password counts
    /// ([`Self::count_failed_sign_in`]), so athletes signing in successfully
    /// from one shared address never fill its window. Checked before the
    /// password, so past a full window every attempt is refused alike and a
    /// refusal never says whether a guess was right. `client` is `None` only
    /// on a router served without `ConnectInfo`; the account's window still
    /// applies.
    ///
    /// # Errors
    ///
    /// Returns an error when the process's own store holds something other
    /// than a count at a window's key: the caller refuses the attempt rather
    /// than let an unmetered guess through.
    pub async fn sign_in_wait(
        &self,
        client: Option<IpAddr>,
        email: &str,
    ) -> AppResult<Option<u32>> {
        let mut wait = None;
        for (endpoint, key) in Self::sign_in_windows(client, email) {
            let status = self.peek(endpoint, &key).await?;
            if status.is_limited {
                let retry_after = status.retry_after_seconds.unwrap_or(1);
                wait = Some(wait.map_or(retry_after, |longest: u32| longest.max(retry_after)));
            }
        }
        Ok(wait)
    }

    /// Count one refused password sign-in from `client` naming `email` in the
    /// address's window and the account's.
    ///
    /// Every window is counted even when another could not be. The sign-in is
    /// already answered, so a window neither store can count is logged here
    /// rather than returned.
    pub async fn count_failed_sign_in(&self, client: Option<IpAddr>, email: &str) {
        for (endpoint, key) in Self::sign_in_windows(client, email) {
            if let Err(failure) = self.count(endpoint, &key).await {
                error!(
                    endpoint = endpoint.as_str(),
                    error = %failure,
                    "OAuth2 rate limiter could not count a refused password sign-in"
                );
            }
        }
    }

    /// The windows a password sign-in is metered in: the client address's,
    /// when there is one, and the account's, keyed by a digest of the
    /// normalised email so an address with no account is metered exactly like
    /// one with an account and no email lands in the store.
    fn sign_in_windows(client: Option<IpAddr>, email: &str) -> Vec<(OAuth2Endpoint, CacheKey)> {
        let by_account = OAuth2Endpoint::PasswordLoginAccount;
        let account_key = Self::window_key(by_account, &format!("email:{}", email_digest(email)));
        let by_address = client.map(|client| {
            let endpoint = OAuth2Endpoint::PasswordLogin;
            (
                endpoint,
                Self::window_key(endpoint, &metering_key(client).to_string()),
            )
        });
        by_address
            .into_iter()
            .chain([(by_account, account_key)])
            .collect()
    }

    /// Report `endpoint`'s window at `key` without counting a hit in it.
    ///
    /// Reads the shared store and this process's, and reports the fuller: a
    /// hit the shared store could not count was counted here instead.
    async fn peek(
        &self,
        endpoint: OAuth2Endpoint,
        key: &CacheKey,
    ) -> AppResult<OAuth2RateLimitStatus> {
        let mut hits: u64 = 0;
        let mut resets_in = None;
        if let Some(shared) = &self.shared {
            match shared.get::<u64>(key).await {
                Ok(Some(counted)) => {
                    hits = counted;
                    resets_in = shared.ttl(key).await.ok().flatten();
                }
                Ok(None) => {}
                Err(failure) => warn!(
                    endpoint = endpoint.as_str(),
                    error = %failure,
                    "OAuth2 rate limiter's shared store could not read a window; \
                     reading this process's"
                ),
            }
        }
        if let Some(counted) = self.local.get::<u64>(key).await? {
            if counted > hits {
                hits = counted;
                resets_in = self.local.ttl(key).await?;
            }
        }

        let limit = self.limits.get_limit(endpoint);
        let counted = u32::try_from(hits).unwrap_or(u32::MAX);
        let reset_at = (SystemTime::now() + resets_in.unwrap_or_default())
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since_epoch| {
                i64::try_from(since_epoch.as_secs()).unwrap_or(i64::MAX)
            });
        Ok(OAuth2RateLimitStatus {
            is_limited: counted >= limit,
            limit,
            remaining: limit.saturating_sub(counted),
            reset_at,
            retry_after_seconds: None,
        }
        .with_retry_after())
    }

    /// Count one hit in `endpoint`'s window at `key` and report the window.
    async fn status(
        &self,
        endpoint: OAuth2Endpoint,
        key: &CacheKey,
    ) -> AppResult<OAuth2RateLimitStatus> {
        let limit = self.limits.get_limit(endpoint);
        let counted = self.count(endpoint, key).await?;

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

    /// The cache key of `endpoint`'s window for `subject` (a metered client
    /// address, `user:<id>` for an account's own window, or `email:<digest>`
    /// for the account a sign-in names)
    fn window_key(endpoint: OAuth2Endpoint, subject: &str) -> CacheKey {
        CacheKey::new(
            TenantId::nil(),
            Uuid::nil(),
            WINDOW_KEY_NAMESPACE.to_owned(),
            CacheResource::Custom(format!("{}:{subject}", endpoint.as_str())),
        )
    }
}

/// The SHA-256 of `email` trimmed and lowercased, in hex: the subject of the
/// window a password sign-in naming it counts in. Spelling variants of one
/// address share a window, and the address itself never reaches the store.
fn email_digest(email: &str) -> String {
    format!(
        "{:x}",
        Sha256::digest(email.trim().to_lowercase().as_bytes())
    )
}
