// ABOUTME: Process-wide guard state for ChatProvider::Chain primary protection
// ABOUTME: GitHub rate-limit headroom + circuit breaker for preemptive fallback

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Chain Guard
//!
//! Shared state for two preemptive-fallback signals consulted by
//! [`ChatProvider::Chain`]:
//!
//! 1. **GitHub rate-limit headroom (Strategy A).** A separate periodic
//!    probe (in pierre-server) calls `GET https://api.github.com/rate_limit`
//!    on every tick and pushes the `core.remaining`/`core.reset` figures
//!    in here via [`ChainGuard::record_github_rate_limit`]. Chain reads
//!    [`ChainGuard::is_github_budget_low`] before each request and
//!    routes directly to the secondary when the budget is below
//!    threshold — Copilot's session-token exchange shares this 5000/hr
//!    pool, so a near-exhausted budget is a strong predictor that the
//!    *next* Copilot call will fail with `Authentication required`.
//!
//! 2. **Circuit breaker on primary auth/rate-limit failures (Strategy B).**
//!    [`ChainGuard::record_primary_failure`] tracks consecutive
//!    auth-shaped errors from the primary. After
//!    [`CIRCUIT_FAILURE_THRESHOLD`] consecutive failures the circuit
//!    *opens* — every subsequent request skips primary for
//!    [`CIRCUIT_COOLDOWN`]. The first request after cooldown is the
//!    half-open probe: a primary success on it closes the circuit; a
//!    failure resets the cooldown.
//!
//! State is per-process and lives behind a single [`LazyLock<ChainGuard>`]
//! so both the probe (which writes A) and the chain wrapper (which
//! reads A and writes B) hit the same instance without explicit
//! plumbing through `ServerContext`. The values are `AtomicU64`/`AtomicUsize`
//! so reads on the request hot path don't take a lock.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::LazyLock;
use std::time::{SystemTime, UNIX_EPOCH};

/// Threshold below which the GitHub core API budget is considered "low"
/// and the Chain preemptively routes to the secondary. Pierre's Copilot
/// session token exchange spends 1 call per refresh (~every few
/// minutes), plus the periodic probe spends 1, plus any tool the LLM
/// invokes that hits api.github.com. 200 leaves enough headroom for a
/// handful of refresh cycles before the limit window resets.
pub const GITHUB_BUDGET_THRESHOLD: u64 = 200;

/// Consecutive auth-shaped failures from the primary before opening the
/// circuit. 3 is the standard Hystrix/resilience4j default — high
/// enough to absorb a single transient blip without flipping, low
/// enough that a stuck-broken primary doesn't waste many requests.
pub const CIRCUIT_FAILURE_THRESHOLD: usize = 3;

/// How long the circuit stays open after the failure threshold trips.
/// 60s strikes a balance: long enough to outlast typical Copilot rate
/// windows (which reset every minute), short enough that we re-probe
/// primary often when it's only briefly degraded.
pub const CIRCUIT_COOLDOWN_SECS: u64 = 60;

/// Sentinel for "GitHub rate limit hasn't been probed yet" — chain
/// treats this as fail-open (use primary). `u64::MAX` is unambiguous:
/// no GitHub API quota can legitimately be this high.
const RATE_LIMIT_UNKNOWN: u64 = u64::MAX;

/// Process-wide [`ChainGuard`] shared by the GitHub rate-limit probe
/// (pierre-server) and the `ChatProvider::Chain` request path
/// (pierre-llm). LazyLock for zero-config plumbing — first access on
/// either side initialises the same instance.
pub static CHAIN_GUARD: LazyLock<ChainGuard> = LazyLock::new(ChainGuard::new);

/// Outcome of recording a GitHub rate-limit probe result. Callers
/// (the probe in pierre-server) compare the previous and current
/// budget tiers to decide whether to emit
/// `llm.rate_limit_low` / `llm.rate_limit_recovered` notify events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RateLimitTransition {
    /// Was below threshold and stayed below.
    StillLow,
    /// Was at or above threshold and dropped below.
    EnteredLow,
    /// Was below threshold and rose to or above.
    ExitedLow,
    /// Was at or above and stayed at or above.
    StillOk,
}

/// Outcome of recording a primary outcome on the circuit breaker.
/// Callers emit `llm.circuit_opened` / `llm.circuit_closed` notify
/// events on the transition variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitTransition {
    /// Circuit was closed and stayed closed.
    StillClosed,
    /// Circuit was open and stayed open.
    StillOpen,
    /// Circuit just opened on this event (failure threshold crossed).
    Opened,
    /// Circuit just closed on this event (primary success on half-open).
    Closed,
}

/// Shared guard state. Cheap to read from the hot path: every field
/// is an atomic primitive.
pub struct ChainGuard {
    /// `X-RateLimit-Remaining` from the most recent github.com `/rate_limit`
    /// probe. `RATE_LIMIT_UNKNOWN` until the probe has run once.
    github_remaining: AtomicU64,
    /// Unix-seconds when GitHub's core rate-limit window resets. Stored
    /// for telemetry; not used in the skip decision.
    github_reset_at: AtomicU64,
    /// Consecutive auth/rate-limit failures from the primary since the
    /// last success.
    consecutive_primary_failures: AtomicUsize,
    /// Unix-milliseconds when the circuit opened. `0` means closed.
    circuit_opened_at_ms: AtomicU64,
}

impl Default for ChainGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl ChainGuard {
    /// Build a fresh guard in the fail-open initial state: rate limit
    /// unknown, circuit closed, no recorded failures.
    #[must_use]
    pub fn new() -> Self {
        Self {
            github_remaining: AtomicU64::new(RATE_LIMIT_UNKNOWN),
            github_reset_at: AtomicU64::new(0),
            consecutive_primary_failures: AtomicUsize::new(0),
            circuit_opened_at_ms: AtomicU64::new(0),
        }
    }

    /// Record the latest GitHub rate-limit probe result. Returns the
    /// budget-tier transition for the caller to optionally page on.
    pub fn record_github_rate_limit(&self, remaining: u64, reset_at: u64) -> RateLimitTransition {
        let previous = self.github_remaining.load(Ordering::Relaxed);
        let was_low = previous != RATE_LIMIT_UNKNOWN && previous < GITHUB_BUDGET_THRESHOLD;
        let is_low = remaining < GITHUB_BUDGET_THRESHOLD;
        self.github_remaining.store(remaining, Ordering::Relaxed);
        self.github_reset_at.store(reset_at, Ordering::Relaxed);
        match (was_low, is_low) {
            (true, true) => RateLimitTransition::StillLow,
            (false, true) => RateLimitTransition::EnteredLow,
            (true, false) => RateLimitTransition::ExitedLow,
            (false, false) => RateLimitTransition::StillOk,
        }
    }

    /// Latest known GitHub `core.remaining`. `None` when the probe
    /// hasn't yet recorded a value (fail-open: treat as no budget
    /// pressure rather than blocking the primary path).
    #[must_use]
    pub fn github_remaining(&self) -> Option<u64> {
        let raw = self.github_remaining.load(Ordering::Relaxed);
        if raw == RATE_LIMIT_UNKNOWN {
            None
        } else {
            Some(raw)
        }
    }

    /// Latest known GitHub rate-limit reset epoch (unix seconds).
    #[must_use]
    pub fn github_reset_at(&self) -> u64 {
        self.github_reset_at.load(Ordering::Relaxed)
    }

    /// True when the GitHub budget has been probed and is below
    /// [`GITHUB_BUDGET_THRESHOLD`]. Hot-path check used by Chain to
    /// decide whether to skip the primary preemptively.
    #[must_use]
    pub fn is_github_budget_low(&self) -> bool {
        self.github_remaining()
            .is_some_and(|r| r < GITHUB_BUDGET_THRESHOLD)
    }

    /// Record an auth-shaped primary failure. Returns the circuit
    /// transition for the caller to optionally page on. Non-auth
    /// failures should NOT call this — they shouldn't count against
    /// the circuit (we want to react to systemic rate-limit /
    /// expired-token issues, not to transient request-specific 5xxs).
    pub fn record_primary_failure(&self) -> CircuitTransition {
        let was_open = self.is_circuit_open();
        let count = self
            .consecutive_primary_failures
            .fetch_add(1, Ordering::Relaxed)
            + 1;
        if count >= CIRCUIT_FAILURE_THRESHOLD && !was_open {
            self.circuit_opened_at_ms.store(now_ms(), Ordering::Relaxed);
            CircuitTransition::Opened
        } else if was_open {
            // Half-open probe failed — reset the cooldown clock so the
            // next probe doesn't fire until another full window passes.
            self.circuit_opened_at_ms.store(now_ms(), Ordering::Relaxed);
            CircuitTransition::StillOpen
        } else {
            CircuitTransition::StillClosed
        }
    }

    /// Record a successful primary call. Resets the consecutive-failure
    /// counter and, if the circuit was open, closes it.
    pub fn record_primary_success(&self) -> CircuitTransition {
        let was_open = self.is_circuit_open();
        self.consecutive_primary_failures
            .store(0, Ordering::Relaxed);
        if was_open {
            self.circuit_opened_at_ms.store(0, Ordering::Relaxed);
            CircuitTransition::Closed
        } else {
            CircuitTransition::StillClosed
        }
    }

    /// True when the circuit is currently open (cooldown still
    /// active). `record_primary_success` and the cooldown-elapsed
    /// check are the only ways to flip back to closed.
    #[must_use]
    pub fn is_circuit_open(&self) -> bool {
        let opened_at = self.circuit_opened_at_ms.load(Ordering::Relaxed);
        if opened_at == 0 {
            return false;
        }
        let elapsed_ms = now_ms().saturating_sub(opened_at);
        elapsed_ms < CIRCUIT_COOLDOWN_SECS * 1_000
    }

    /// True when the circuit has been open longer than the cooldown
    /// window and is ready for a half-open probe. Used by Chain to
    /// decide whether to try the primary once "for real" or keep
    /// skipping it.
    #[must_use]
    pub fn is_half_open(&self) -> bool {
        let opened_at = self.circuit_opened_at_ms.load(Ordering::Relaxed);
        if opened_at == 0 {
            return false;
        }
        let elapsed_ms = now_ms().saturating_sub(opened_at);
        elapsed_ms >= CIRCUIT_COOLDOWN_SECS * 1_000
    }

    /// True when Chain should skip the primary for this request:
    /// either GitHub budget is low (Strategy A) or the circuit is
    /// open and not yet half-open (Strategy B).
    #[must_use]
    pub fn should_skip_primary(&self) -> bool {
        if self.is_github_budget_low() {
            return true;
        }
        // Circuit open but not yet ready to half-open probe.
        let opened_at = self.circuit_opened_at_ms.load(Ordering::Relaxed);
        if opened_at == 0 {
            return false;
        }
        let elapsed_ms = now_ms().saturating_sub(opened_at);
        elapsed_ms < CIRCUIT_COOLDOWN_SECS * 1_000
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}
