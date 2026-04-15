// ABOUTME: Platform-side FIFO queue and backpressure limiter for Pierre→Sciotte scraping
// ABOUTME: Caps concurrent Chrome processes and sheds excess load with 503 + Retry-After
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Sciotte backpressure limiter (Pierre side)
//!
//! Pierre holds the authoritative concurrency budget for Chrome-driven Sciotte
//! scraping operations. The limiter is a FIFO-fair semaphore paired with an
//! atomic depth gauge for fast-reject, and it is the single place that
//! controls how many Chrome processes can be alive inside the Pierre pod at
//! any given time.
//!
//! Every scraper-driven HTTP handler (`handle_sciotte_login`,
//! `handle_sciotte_submit_otp`, `handle_sciotte_select_2fa`,
//! `spawn_activity_prefetch`) must acquire a [`ScrapePermit`] before starting
//! work. Multi-step login flows hand the permit off across requests by
//! storing it alongside the scraper in `PENDING_OTP_SCRAPERS`; the watchdog
//! evicts permits whose owning flow has gone silent.
//!
//! ## Configuration
//!
//! The limiter intentionally ships no numeric defaults. Operators are the
//! single source of truth for every knob, supplied at startup via seven
//! `PIERRE_SCIOTTE_*` environment variables read by [`LimiterConfig::from_env`].
//! Production values live in Terraform / .envrc and are reviewable as infra,
//! not code. A missing or malformed variable aborts server startup so bad
//! infra cannot silently paper over itself.

use std::env;
use std::future::Future;
use std::num::ParseIntError;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use thiserror::Error;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, TryAcquireError};
use tokio::task::JoinHandle;
use tokio::time;
use tracing::{debug, info, warn};

// ============================================================================
// Configuration
// ============================================================================

/// Environment variable for the maximum number of concurrent scrapes
pub const ENV_MAX_CONCURRENT: &str = "PIERRE_SCIOTTE_MAX_CONCURRENT";
/// Environment variable for the maximum combined queue depth
pub const ENV_MAX_QUEUE_DEPTH: &str = "PIERRE_SCIOTTE_MAX_QUEUE";
/// Environment variable for the acquire timeout in seconds
pub const ENV_ACQUIRE_TIMEOUT_SECS: &str = "PIERRE_SCIOTTE_ACQUIRE_TIMEOUT_SECS";
/// Environment variable for the parked-permit TTL in seconds
pub const ENV_PARKED_PERMIT_TTL_SECS: &str = "PIERRE_SCIOTTE_PERMIT_MAX_LIFETIME_SECS";
/// Environment variable for the watchdog tick interval in seconds
pub const ENV_WATCHDOG_INTERVAL_SECS: &str = "PIERRE_SCIOTTE_WATCHDOG_INTERVAL_SECS";
/// Environment variable for the `Retry-After` hint on queue-full / timeout rejections
pub const ENV_RETRY_AFTER_HINT_SECS: &str = "PIERRE_SCIOTTE_RETRY_AFTER_HINT_SECS";
/// Environment variable for the `Retry-After` hint when the limiter is closed (shutdown)
pub const ENV_CLOSED_RETRY_AFTER_SECS: &str = "PIERRE_SCIOTTE_CLOSED_RETRY_AFTER_SECS";

/// Runtime configuration for [`SciotteLimiter`]. Every field is mandatory and
/// must be supplied by the caller — the crate ships no numeric defaults.
#[derive(Debug, Clone)]
pub struct LimiterConfig {
    /// Hard cap on concurrently running scrapes. Maps directly to the number
    /// of simultaneous Chrome processes inside the Pierre pod.
    pub max_concurrent: usize,
    /// Combined cap on in-flight + waiting scrapes. Requests beyond this
    /// value are fast-rejected so the server can not accumulate an unbounded
    /// backlog of pending futures.
    pub max_queue_depth: usize,
    /// Upper bound on how long a waiting request will block before being
    /// rejected with [`LimiterError::AcquireTimeout`].
    pub acquire_timeout: Duration,
    /// Upper bound on how long a permit can stay parked across a multi-step
    /// flow. After this the watchdog drops the permit so the slot is reused.
    pub parked_permit_ttl: Duration,
    /// Interval at which the watchdog scans for stale parked permits.
    pub watchdog_interval: Duration,
    /// `Retry-After` hint emitted on [`LimiterError::QueueFull`] and
    /// [`LimiterError::AcquireTimeout`] / [`LimiterError::NoCapacity`] rejections.
    pub retry_after_hint: Duration,
    /// `Retry-After` hint emitted on [`LimiterError::Closed`] rejections.
    pub closed_retry_after: Duration,
}

/// Error returned by [`LimiterConfig::from_env`] when a required variable is
/// missing or cannot be parsed. The binary is expected to surface this to
/// the operator and abort startup.
#[derive(Debug, Error)]
pub enum LimiterConfigError {
    /// A required environment variable is not set.
    #[error("missing required environment variable `{0}`")]
    Missing(&'static str),
    /// A required environment variable failed to parse as an integer.
    #[error("environment variable `{name}` is not a valid integer: {source}")]
    Parse {
        /// Name of the offending variable
        name: &'static str,
        /// Underlying parse error
        #[source]
        source: ParseIntError,
    },
    /// `max_queue_depth` is smaller than `max_concurrent`, which would
    /// starve the running pool.
    #[error(
        "max_queue_depth ({queue}) must be >= max_concurrent ({concurrent}) — \
         otherwise active scrapes count against a queue that cannot hold them"
    )]
    QueueSmallerThanConcurrency {
        /// Configured max concurrent value
        concurrent: usize,
        /// Configured queue depth value
        queue: usize,
    },
    /// A numeric field was set to zero where zero is not a legal value.
    #[error("environment variable `{0}` must be greater than zero")]
    ZeroValue(&'static str),
}

impl LimiterConfig {
    /// Read every required field from environment variables and fail fast on
    /// missing or malformed values. Call this once from your binary's `main`.
    ///
    /// # Errors
    ///
    /// Returns [`LimiterConfigError`] when a required `PIERRE_SCIOTTE_*`
    /// variable is missing, fails integer parsing, is set to zero where
    /// zero is illegal, or when `max_queue_depth < max_concurrent`.
    pub fn from_env() -> Result<Self, LimiterConfigError> {
        let max_concurrent = required_usize(ENV_MAX_CONCURRENT)?;
        if max_concurrent == 0 {
            return Err(LimiterConfigError::ZeroValue(ENV_MAX_CONCURRENT));
        }
        let max_queue_depth = required_usize(ENV_MAX_QUEUE_DEPTH)?;
        if max_queue_depth < max_concurrent {
            return Err(LimiterConfigError::QueueSmallerThanConcurrency {
                concurrent: max_concurrent,
                queue: max_queue_depth,
            });
        }

        let acquire_timeout = required_duration_secs(ENV_ACQUIRE_TIMEOUT_SECS)?;
        let parked_permit_ttl = required_duration_secs(ENV_PARKED_PERMIT_TTL_SECS)?;
        let watchdog_interval = required_duration_secs(ENV_WATCHDOG_INTERVAL_SECS)?;
        if watchdog_interval.is_zero() {
            return Err(LimiterConfigError::ZeroValue(ENV_WATCHDOG_INTERVAL_SECS));
        }
        let retry_after_hint = required_duration_secs(ENV_RETRY_AFTER_HINT_SECS)?;
        let closed_retry_after = required_duration_secs(ENV_CLOSED_RETRY_AFTER_SECS)?;

        Ok(Self {
            max_concurrent,
            max_queue_depth,
            acquire_timeout,
            parked_permit_ttl,
            watchdog_interval,
            retry_after_hint,
            closed_retry_after,
        })
    }
}

fn required_usize(name: &'static str) -> Result<usize, LimiterConfigError> {
    let raw = env::var(name).map_err(|_| LimiterConfigError::Missing(name))?;
    raw.parse::<usize>()
        .map_err(|source| LimiterConfigError::Parse { name, source })
}

fn required_duration_secs(name: &'static str) -> Result<Duration, LimiterConfigError> {
    let raw = env::var(name).map_err(|_| LimiterConfigError::Missing(name))?;
    let secs = raw
        .parse::<u64>()
        .map_err(|source| LimiterConfigError::Parse { name, source })?;
    Ok(Duration::from_secs(secs))
}

// ============================================================================
// Errors
// ============================================================================

/// Errors produced by the limiter when a request cannot be admitted.
///
/// Each variant carries the configured `Retry-After` hint so the HTTP layer
/// can propagate it to clients without reaching back into the limiter.
#[derive(Debug, Error)]
pub enum LimiterError {
    /// The combined running + waiting depth is at the configured cap.
    #[error("sciotte queue is full (depth {depth}, max {max})")]
    QueueFull {
        /// Depth observed when the rejection was issued
        depth: usize,
        /// Configured maximum depth
        max: usize,
        /// Retry-After hint to propagate to clients
        retry_after: Duration,
    },

    /// The request waited the full `acquire_timeout` without getting a permit.
    #[error("timed out waiting for a sciotte slot after {}s", .timeout.as_secs())]
    AcquireTimeout {
        /// Configured acquire timeout
        timeout: Duration,
        /// Retry-After hint to propagate to clients
        retry_after: Duration,
    },

    /// No permit was available at the time of a non-blocking `try_acquire`.
    /// Distinct from [`AcquireTimeout`] because the caller explicitly opted
    /// out of waiting.
    #[error("no sciotte permit available (try_acquire)")]
    NoCapacity {
        /// Retry-After hint to propagate to clients
        retry_after: Duration,
    },

    /// The underlying semaphore has been closed.
    #[error("sciotte limiter has been shut down")]
    Closed {
        /// Retry-After hint to propagate to clients
        retry_after: Duration,
    },
}

impl LimiterError {
    /// `Retry-After` value in seconds for HTTP 503 responses, read from the
    /// hint carried inside the error variant.
    #[must_use]
    pub const fn retry_after_secs(&self) -> u64 {
        match self {
            Self::QueueFull { retry_after, .. }
            | Self::AcquireTimeout { retry_after, .. }
            | Self::NoCapacity { retry_after }
            | Self::Closed { retry_after } => retry_after.as_secs(),
        }
    }
}

// ============================================================================
// Permit RAII
// ============================================================================

/// An acquired scraper slot. Dropping releases the underlying semaphore
/// permit and decrements the queue depth atomically.
#[derive(Debug)]
pub struct ScrapePermit {
    permit: Option<OwnedSemaphorePermit>,
    depth: Arc<AtomicUsize>,
    acquired_at: Instant,
}

impl ScrapePermit {
    /// Timestamp at which this permit was issued. Used by the watchdog for
    /// staleness evaluation when the permit is parked.
    #[must_use]
    pub const fn acquired_at(&self) -> Instant {
        self.acquired_at
    }
}

impl Drop for ScrapePermit {
    fn drop(&mut self) {
        drop(self.permit.take());
        self.depth.fetch_sub(1, Ordering::AcqRel);
    }
}

// ============================================================================
// Limiter
// ============================================================================

/// FIFO-fair limiter that caps concurrent Chrome-backed Sciotte scrapes and
/// sheds excess load. Construct once at server startup via
/// [`LimiterConfig::from_env`] and share via `Arc<SciotteLimiter>`.
pub struct SciotteLimiter {
    semaphore: Arc<Semaphore>,
    config: LimiterConfig,
    depth: Arc<AtomicUsize>,
}

impl SciotteLimiter {
    /// Build a limiter from an explicit configuration. This is the only
    /// constructor — there is no `from_env` fallback. Binaries should read
    /// [`LimiterConfig::from_env`] once at startup and pass the result here.
    #[must_use]
    pub fn new(config: LimiterConfig) -> Arc<Self> {
        Arc::new(Self {
            semaphore: Arc::new(Semaphore::new(config.max_concurrent)),
            depth: Arc::new(AtomicUsize::new(0)),
            config,
        })
    }

    /// Configuration used by this limiter.
    #[must_use]
    pub const fn config(&self) -> &LimiterConfig {
        &self.config
    }

    /// Current observed depth (running + waiting).
    #[must_use]
    pub fn depth(&self) -> usize {
        self.depth.load(Ordering::Acquire)
    }

    /// Number of free semaphore permits right now.
    #[must_use]
    pub fn available_permits(&self) -> usize {
        self.semaphore.available_permits()
    }

    /// Acquire a permit, waiting up to `acquire_timeout` if the semaphore is
    /// saturated. Fast-rejects with [`LimiterError::QueueFull`] when the
    /// combined depth already exceeds `max_queue_depth`.
    ///
    /// # Errors
    ///
    /// Returns [`LimiterError::QueueFull`] on fast-reject, [`LimiterError::AcquireTimeout`]
    /// when the caller waits longer than `acquire_timeout`, or [`LimiterError::Closed`]
    /// when the underlying semaphore has been closed during shutdown.
    pub async fn acquire(&self) -> Result<ScrapePermit, LimiterError> {
        let prior = self.depth.fetch_add(1, Ordering::AcqRel);
        if prior >= self.config.max_queue_depth {
            self.depth.fetch_sub(1, Ordering::AcqRel);
            return Err(LimiterError::QueueFull {
                depth: prior,
                max: self.config.max_queue_depth,
                retry_after: self.config.retry_after_hint,
            });
        }

        let sem = Arc::clone(&self.semaphore);
        let permit = match time::timeout(self.config.acquire_timeout, sem.acquire_owned()).await {
            Ok(Ok(p)) => p,
            Ok(Err(_closed)) => {
                self.depth.fetch_sub(1, Ordering::AcqRel);
                return Err(LimiterError::Closed {
                    retry_after: self.config.closed_retry_after,
                });
            }
            Err(_elapsed) => {
                self.depth.fetch_sub(1, Ordering::AcqRel);
                return Err(LimiterError::AcquireTimeout {
                    timeout: self.config.acquire_timeout,
                    retry_after: self.config.retry_after_hint,
                });
            }
        };

        debug!(
            depth = self.depth(),
            available = self.available_permits(),
            "Sciotte permit acquired"
        );
        Ok(ScrapePermit {
            permit: Some(permit),
            depth: Arc::clone(&self.depth),
            acquired_at: Instant::now(),
        })
    }

    /// Non-blocking permit acquisition for low-priority background work
    /// (e.g. activity prefetch). Fails immediately with
    /// [`LimiterError::NoCapacity`] if no permit is available.
    ///
    /// # Errors
    ///
    /// Returns [`LimiterError::QueueFull`] on fast-reject, [`LimiterError::NoCapacity`]
    /// when no permit is available right now, or [`LimiterError::Closed`] when
    /// the underlying semaphore has been closed during shutdown.
    pub fn try_acquire(&self) -> Result<ScrapePermit, LimiterError> {
        let prior = self.depth.fetch_add(1, Ordering::AcqRel);
        if prior >= self.config.max_queue_depth {
            self.depth.fetch_sub(1, Ordering::AcqRel);
            return Err(LimiterError::QueueFull {
                depth: prior,
                max: self.config.max_queue_depth,
                retry_after: self.config.retry_after_hint,
            });
        }

        let sem = Arc::clone(&self.semaphore);
        match sem.try_acquire_owned() {
            Ok(permit) => Ok(ScrapePermit {
                permit: Some(permit),
                depth: Arc::clone(&self.depth),
                acquired_at: Instant::now(),
            }),
            Err(TryAcquireError::NoPermits) => {
                self.depth.fetch_sub(1, Ordering::AcqRel);
                Err(LimiterError::NoCapacity {
                    retry_after: self.config.retry_after_hint,
                })
            }
            Err(TryAcquireError::Closed) => {
                self.depth.fetch_sub(1, Ordering::AcqRel);
                Err(LimiterError::Closed {
                    retry_after: self.config.closed_retry_after,
                })
            }
        }
    }

    /// Spawn a background watchdog loop that runs `tick_fn` on every
    /// `watchdog_interval`. The callback receives the configured
    /// `parked_permit_ttl` so it can evaluate which parked entries to drop.
    /// Returns the [`JoinHandle`] so callers can abort it during shutdown.
    pub fn spawn_watchdog<F, Fut>(self: &Arc<Self>, mut tick_fn: F) -> JoinHandle<()>
    where
        F: FnMut(Duration) -> Fut + Send + 'static,
        Fut: Future<Output = usize> + Send + 'static,
    {
        let me = Arc::clone(self);
        tokio::spawn(async move {
            let mut ticker = time::interval(me.config.watchdog_interval);
            ticker.set_missed_tick_behavior(time::MissedTickBehavior::Delay);
            info!(
                interval_secs = me.config.watchdog_interval.as_secs(),
                ttl_secs = me.config.parked_permit_ttl.as_secs(),
                "Sciotte limiter watchdog started"
            );
            loop {
                ticker.tick().await;
                let evicted = tick_fn(me.config.parked_permit_ttl).await;
                if evicted > 0 {
                    warn!(evicted, "Sciotte watchdog evicted stale parked flows");
                }
            }
        })
    }
}
