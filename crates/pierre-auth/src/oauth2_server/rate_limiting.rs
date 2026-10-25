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
use pierre_core::constants::oauth_rate_limiting::KNOWN_DEVICE_WINDOW_SECS;
use pierre_core::errors::AppResult;
use pierre_core::models::TenantId;
use sha2::{Digest, Sha256};
use tracing::warn;
use uuid::Uuid;

use super::known_device::KnownDeviceKey;
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

/// The store a sign-in attempt's hit was counted in, so its refund lands in
/// the same one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CountedIn {
    /// The store every replica counts into
    Shared,
    /// This process's own windows
    Local,
}

/// A password sign-in counted in its windows by
/// [`OAuth2RateLimiter::begin_sign_in`], before its password is checked.
#[derive(Debug)]
#[must_use = "a refused attempt must be answered, and an admitted one refunded or kept"]
pub struct SignInAttempt {
    /// Each window the attempt was counted in, with the store that counted it
    counted: Vec<(OAuth2Endpoint, CacheKey, CountedIn)>,
    /// The longest wait a full window asks for, when one is full
    retry_after: Option<u32>,
}

impl SignInAttempt {
    /// `None` when every window had room for the attempt, so its password may
    /// be checked; `Some(seconds)` — the `Retry-After` — when one is full.
    #[must_use]
    pub const fn retry_after(&self) -> Option<u32> {
        self.retry_after
    }
}

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
#[derive(Clone, Debug)]
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
    /// Signs and checks the known-device cookies whose sign-ins skip the
    /// account window (carnet#1088); an `Arc` so every clone of the limiter
    /// handed to a route signs with the one key
    known_devices: Arc<KnownDeviceKey>,
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
            known_devices: Arc::new(KnownDeviceKey::ephemeral()),
        }
    }

    /// This limiter, honouring the known-device cookies `key` signs.
    ///
    /// Without it the limiter signs with a key of its own process, so its
    /// cookies lapse at a restart and are not honoured by another replica;
    /// the composition root hands every limiter the key derived from the
    /// server's JWT secret.
    #[must_use]
    pub fn with_known_device_key(mut self, key: KnownDeviceKey) -> Self {
        self.known_devices = Arc::new(key);
        self
    }

    /// A known-device cookie value for the account `email` names, for the
    /// browser whose password for it just verified: its next sign-ins to that
    /// account skip the account window (see [`Self::begin_sign_in`]).
    #[must_use]
    pub fn issue_known_device(&self, email: &str) -> String {
        self.known_devices.issue(&email_digest(email))
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

    /// Count a password sign-in from `client` naming `email` in the
    /// address's window and the account's, before its password is checked.
    ///
    /// Each window is counted with the store's atomic increment, so a burst of
    /// concurrent attempts cannot all read the same pre-burst count: only as
    /// many as a window admits get past [`SignInAttempt::retry_after`], and
    /// the rest are refused before their password is checked. Past a full
    /// window every attempt is refused alike, so a refusal never says whether
    /// a guess was right. A verified password, or an outcome that was not the
    /// caller's guess, gives its hit back through [`Self::refund_sign_in`],
    /// so athletes signing in successfully from one shared address never fill
    /// its window. `client` is `None` only on a router served without
    /// `ConnectInfo`; the account's window still applies.
    ///
    /// `known_device` is the browser's known-device cookie, when it sent one.
    /// A cookie [`Self::issue_known_device`] issued for this email is counted
    /// in its device's own window instead of the account's (carnet#1088), so
    /// a stranger filling the account window does not lock out a browser the
    /// athlete signed in from before. A device's window admits
    /// `KNOWN_DEVICE_FAILURES` refused passwords a day; past that, or with no
    /// valid cookie, the attempt is metered in the account window like any
    /// other.
    ///
    /// # Errors
    ///
    /// Returns an error when neither store can count the attempt in a window:
    /// the caller refuses it rather than let an unmetered guess through.
    pub async fn begin_sign_in(
        &self,
        client: Option<IpAddr>,
        email: &str,
        known_device: Option<&str>,
    ) -> AppResult<SignInAttempt> {
        let digest = email_digest(email);
        let mut attempt = SignInAttempt {
            counted: Vec::with_capacity(3),
            retry_after: None,
        };
        if let Some(client) = client {
            let endpoint = OAuth2Endpoint::PasswordLogin;
            let key = Self::window_key(endpoint, &metering_key(client).to_string());
            self.count_attempt(&mut attempt, endpoint, key, self.window)
                .await?;
        }
        let device = known_device.and_then(|cookie| self.known_devices.device(cookie, &digest));
        if let Some(device) = device {
            let endpoint = OAuth2Endpoint::PasswordLoginKnownDevice;
            let key = Self::window_key(endpoint, &format!("device:{device}"));
            let (counted, store) = self
                .count_in(
                    endpoint,
                    &key,
                    Duration::from_secs(KNOWN_DEVICE_WINDOW_SECS),
                )
                .await?;
            attempt.counted.push((endpoint, key, store));
            if counted.hits <= u64::from(self.limits.get_limit(endpoint)) {
                return Ok(attempt);
            }
        }
        let endpoint = OAuth2Endpoint::PasswordLoginAccount;
        let key = Self::window_key(endpoint, &format!("email:{digest}"));
        self.count_attempt(&mut attempt, endpoint, key, self.window)
            .await?;
        Ok(attempt)
    }

    /// Count `attempt` in `endpoint`'s window at `key`, lasting `window`, and
    /// raise its wait when that window is full.
    async fn count_attempt(
        &self,
        attempt: &mut SignInAttempt,
        endpoint: OAuth2Endpoint,
        key: CacheKey,
        window: Duration,
    ) -> AppResult<()> {
        let (counted, store) = self.count_in(endpoint, &key, window).await?;
        if counted.hits > u64::from(self.limits.get_limit(endpoint)) {
            let wait = u32::try_from(counted.resets_in.as_secs())
                .unwrap_or(u32::MAX)
                .max(1);
            attempt.retry_after = Some(attempt.retry_after.map_or(wait, |w| w.max(wait)));
        }
        attempt.counted.push((endpoint, key, store));
        Ok(())
    }

    /// Give back the hits [`Self::begin_sign_in`] counted for `attempt`, in
    /// the store that counted each.
    ///
    /// For an attempt whose password verified, or that failed for a reason
    /// that was not the caller's guess (a suspended account, a server fault).
    /// The sign-in is already answered, so a hit that cannot be given back is
    /// logged here rather than returned; it closes with its window.
    pub async fn refund_sign_in(&self, attempt: SignInAttempt) {
        for (endpoint, key, store) in attempt.counted {
            let refunded = match (store, &self.shared) {
                (CountedIn::Shared, Some(shared)) => shared.uncount_in_window(&key).await,
                (CountedIn::Shared | CountedIn::Local, _) => {
                    self.local.uncount_in_window(&key).await
                }
            };
            if let Err(failure) = refunded {
                warn!(
                    endpoint = endpoint.as_str(),
                    error = %failure,
                    "OAuth2 rate limiter could not give back a sign-in attempt's hit"
                );
            }
        }
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
        Ok(self.count_in(endpoint, key, self.window).await?.0)
    }

    /// Count one hit at `key` in a window lasting `window`, naming the store
    /// that counted it: the shared store when there is one and it answers,
    /// else this process's.
    async fn count_in(
        &self,
        endpoint: OAuth2Endpoint,
        key: &CacheKey,
        window: Duration,
    ) -> AppResult<(WindowCount, CountedIn)> {
        if let Some(shared) = &self.shared {
            match shared.count_in_window(key, window).await {
                Ok(counted) => return Ok((counted, CountedIn::Shared)),
                Err(failure) => warn!(
                    endpoint = endpoint.as_str(),
                    error = %failure,
                    "OAuth2 rate limiter's shared store could not count the request; \
                     counting it in this process's window"
                ),
            }
        }
        Ok((
            self.local.count_in_window(key, window).await?,
            CountedIn::Local,
        ))
    }

    /// The cache key of `endpoint`'s window for `subject` (a metered client
    /// address, `user:<id>` for an account's own window, `email:<digest>`
    /// for the account a sign-in names, or `device:<id>` for a known device)
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

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use tokio::task::yield_now;

    use super::*;

    /// A limiter on its own process-local windows, admitting `per_address`
    /// sign-ins a window from one address and `per_account` naming one email.
    fn limiter(per_address: u32, per_account: u32) -> OAuth2RateLimiter {
        let config = RateLimitConfig {
            password_login_rpm: per_address,
            password_login_account_rpm: per_account,
            rate_limit_window_secs: 60,
            ..RateLimitConfig::default()
        };
        OAuth2RateLimiter::new(None, OAuth2RateLimiter::local_window_store(), &config)
    }

    fn address(last: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(203, 0, 113, last))
    }

    /// Thirty wrong passwords for one account fired at once: every attempt is
    /// counted before any password is checked, so exactly the account's
    /// allowance reaches the verifier and the rest are refused with a wait
    /// (carnet#908).
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_concurrent_burst_reaches_the_verifier_only_as_often_as_the_window_admits() {
        let limiter = limiter(1_000, 10);
        // Shared by the spawned attempts and read once they are all done.
        let verified = Arc::new(AtomicUsize::new(0));
        let attempts: Vec<_> = (0..30_u8)
            .map(|n| {
                let limiter = limiter.clone();
                let verified = Arc::clone(&verified);
                tokio::spawn(async move {
                    let attempt = limiter
                        .begin_sign_in(Some(address(n)), "victim@example.com", None)
                        .await
                        .unwrap();
                    let wait = attempt.retry_after();
                    if wait.is_none() {
                        // The password check: slow, and the burst interleaves here.
                        yield_now().await;
                        verified.fetch_add(1, Ordering::SeqCst);
                    }
                    wait
                })
            })
            .collect();
        let mut refused = Vec::new();
        for attempt in attempts {
            if let Some(wait) = attempt.await.unwrap() {
                refused.push(wait);
            }
        }

        assert_eq!(verified.load(Ordering::SeqCst), 10);
        assert_eq!(refused.len(), 20);
        assert!(
            refused.iter().all(|&wait| (1..=60).contains(&wait)),
            "{refused:?}"
        );
    }

    /// Successful sign-ins give their hit back, so one shared address never
    /// fills its window with them; guesses then still meet the full window.
    #[tokio::test]
    async fn refunded_sign_ins_leave_the_address_window_empty() {
        let limiter = limiter(20, 1_000);
        for n in 0..25 {
            let attempt = limiter
                .begin_sign_in(Some(address(1)), &format!("athlete-{n}@example.com"), None)
                .await
                .unwrap();
            assert_eq!(attempt.retry_after(), None, "success {n} is admitted");
            limiter.refund_sign_in(attempt).await;
        }

        for n in 0..20 {
            let guess = limiter
                .begin_sign_in(Some(address(1)), &format!("guess-{n}@example.com"), None)
                .await
                .unwrap();
            assert_eq!(guess.retry_after(), None, "guess {n} fits the empty window");
        }
        let over = limiter
            .begin_sign_in(Some(address(1)), "one-more@example.com", None)
            .await
            .unwrap();
        assert!(over.retry_after().is_some(), "the 21st guess is refused");
    }

    /// Fill the account window of `email` from strangers' addresses, as the
    /// remote lockout does.
    async fn flood_account(limiter: &OAuth2RateLimiter, email: &str) {
        for n in 0..12_u8 {
            let guess = limiter
                .begin_sign_in(Some(address(100 + n)), email, None)
                .await
                .unwrap();
            drop(guess);
        }
        let stranger = limiter
            .begin_sign_in(Some(address(99)), email, None)
            .await
            .unwrap();
        assert!(
            stranger.retry_after().is_some(),
            "the account window is full"
        );
    }

    /// A stranger filling an athlete's account window does not lock out the
    /// browser the athlete signed in from before (carnet#1088).
    #[tokio::test]
    async fn a_known_device_signs_in_while_strangers_fill_the_account_window() {
        let limiter = limiter(1_000, 10);
        let cookie = limiter.issue_known_device("Victim@Example.com ");
        flood_account(&limiter, "victim@example.com").await;

        let athlete = limiter
            .begin_sign_in(Some(address(1)), "victim@example.com", Some(&cookie))
            .await
            .unwrap();
        assert_eq!(athlete.retry_after(), None);
        limiter.refund_sign_in(athlete).await;
    }

    /// A cookie earned on one account does not skip another account's window.
    #[tokio::test]
    async fn a_cookie_for_another_account_is_metered_like_a_stranger() {
        let limiter = limiter(1_000, 10);
        let own = limiter.issue_known_device("attacker@example.com");
        flood_account(&limiter, "victim@example.com").await;

        for cookie in [own.as_str(), "v1.9999999999.forged.sig"] {
            let attempt = limiter
                .begin_sign_in(Some(address(2)), "victim@example.com", Some(cookie))
                .await
                .unwrap();
            assert!(attempt.retry_after().is_some(), "{cookie}");
        }
    }

    /// A known device's refused passwords are capped: past the day's
    /// allowance it is metered in the account window again, so a stolen
    /// cookie buys only that many guesses.
    #[tokio::test]
    async fn a_known_device_that_keeps_guessing_falls_back_to_the_account_window() {
        let limiter = limiter(1_000, 10);
        let cookie = limiter.issue_known_device("victim@example.com");
        for n in 0..10 {
            let guess = limiter
                .begin_sign_in(Some(address(3)), "victim@example.com", Some(&cookie))
                .await
                .unwrap();
            assert_eq!(
                guess.retry_after(),
                None,
                "guess {n} on the device's own window"
            );
        }
        flood_account(&limiter, "victim@example.com").await;

        let over = limiter
            .begin_sign_in(Some(address(3)), "victim@example.com", Some(&cookie))
            .await
            .unwrap();
        assert!(
            over.retry_after().is_some(),
            "the 11th guess meets the full account window"
        );
    }

    /// A successful sign-in on a known device gives its hit back, so typing
    /// one's password right never spends the device's allowance.
    #[tokio::test]
    async fn successful_known_device_sign_ins_leave_its_window_empty() {
        let limiter = limiter(1_000, 10);
        let cookie = limiter.issue_known_device("victim@example.com");
        flood_account(&limiter, "victim@example.com").await;
        for n in 0..25 {
            let attempt = limiter
                .begin_sign_in(Some(address(4)), "victim@example.com", Some(&cookie))
                .await
                .unwrap();
            assert_eq!(attempt.retry_after(), None, "sign-in {n} is admitted");
            limiter.refund_sign_in(attempt).await;
        }
    }
}
