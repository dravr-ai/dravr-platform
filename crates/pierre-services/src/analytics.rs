// ABOUTME: Analytics consent cache + event tiers for the notify->PostHog sink
// ABOUTME: OnceLock singleton with noop fallback; SHA-256 hashed user IDs for anonymization
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::env;
use std::sync::{LazyLock, OnceLock};

use dashmap::DashMap;
use sha2::{Digest, Sha256};
use tracing::{info, warn};

/// Global singleton instance, initialized once at server startup
static ANALYTICS: OnceLock<Box<dyn AnalyticsTracker>> = OnceLock::new();

/// SHA-256 hash a value for anonymized analytics identifiers
///
/// `PostHog` never sees real user UUIDs, tenant IDs, or channel user IDs.
/// Only the hash (hex-encoded) is transmitted.
#[must_use]
pub fn hash_id(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    hex::encode(hasher.finalize())
}

/// Consent tier of a catalogued notify event, controlling whether per-user
/// opt-in consent gates its capture to the `PostHog` analytics sink.
///
/// The tier is declared per event in dravr-contremaitre's `notify-events.yaml`
/// and asserted by `notify_catalogue_test`. It is the single switch the sink
/// consults to choose a `distinct_id` strategy and a consent requirement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EventTier {
    /// Per-user behavioural event (signup, provider connect, chat turn,
    /// checkout). The `PostHog` `distinct_id` is the hashed user id and
    /// capture requires the user's `analytics_consent` (opt-in).
    Product,
    /// System / health / cost event carrying no personal dimension (sync
    /// outcomes, LLM latency and COGS, fallback and circuit-breaker signals).
    /// The `PostHog` `distinct_id` is the hashed tenant id (or the literal
    /// `service`); the user dimension is never forwarded. Captured
    /// unconditionally as legitimate-interest telemetry.
    Operational,
}

impl EventTier {
    /// Whether an event of this tier may be captured given the user's analytics
    /// consent.
    ///
    /// Operational events are consent-exempt (they carry no personal
    /// dimension); product events require opt-in consent.
    ///
    /// ```
    /// use pierre_services::analytics::EventTier;
    /// assert!(EventTier::Operational.should_capture(false));
    /// assert!(EventTier::Product.should_capture(true));
    /// assert!(!EventTier::Product.should_capture(false));
    /// ```
    #[must_use]
    pub const fn should_capture(self, consent: bool) -> bool {
        match self {
            Self::Operational => true,
            Self::Product => consent,
        }
    }
}

/// Initialize the global analytics tracker from environment variables
///
/// When `POSTHOG_ENABLED` is `true` (default) and `POSTHOG_API_KEY` is set,
/// a live `PostHog` tracker is created. Otherwise a noop tracker is installed
/// so callers never need null checks.
pub fn init_analytics() {
    let enabled = env::var("POSTHOG_ENABLED").map_or(true, |v| v != "false" && v != "0");

    let tracker: Box<dyn AnalyticsTracker> = create_posthog_tracker().unwrap_or_else(|| {
        if enabled {
            warn!("Analytics enabled but POSTHOG_API_KEY not set -- using noop tracker");
        } else {
            info!("Analytics disabled (POSTHOG_ENABLED=false)");
        }
        Box::new(NoopAnalyticsTracker)
    });

    let _ = ANALYTICS.set(tracker);
}

/// Fallback noop instance for when `init_analytics()` has not been called
static NOOP_FALLBACK: NoopAnalyticsTracker = NoopAnalyticsTracker;

/// Get a reference to the global analytics tracker
///
/// Always returns a valid tracker, either the live `PostHog` client or a noop.
/// If called before `init_analytics()`, returns a noop silently.
pub fn analytics() -> &'static dyn AnalyticsTracker {
    ANALYTICS
        .get()
        .map_or(&NOOP_FALLBACK as &dyn AnalyticsTracker, AsRef::as_ref)
}

/// Whether the user behind `hashed_user_id` has affirmative analytics consent.
///
/// Convenience over [`analytics()`] for the `PostHog` notify sink's product-tier
/// consent gate. Returns `false` before `init_analytics()` (noop tracker).
#[must_use]
pub fn is_consented(hashed_user_id: &str) -> bool {
    analytics().is_consented(hashed_user_id)
}

// =============================================================================
// Identity cache (notify-event enrichment)
// =============================================================================

/// Process-wide cache mapping a raw `user_id` to the user's email.
///
/// Populated at authentication time from the durable user record and read by
/// the notify-event enricher synchronously on the tracing thread, so every
/// Slack/`PostHog` message can carry the user's email without a database
/// round-trip. Keyed by the raw `user_id` string carried on notify events —
/// not the hashed id the consent cache uses — because the enricher resolves
/// straight from an event's `user_id` field.
///
/// Distinct from the consent cache: email is internal operator/identity
/// context for the private monitoring channel, not consent-gated `PostHog`
/// state. A cold pod that has not yet seen a user simply renders no email for
/// that event until the next authentication warms the entry.
static USER_EMAILS: LazyLock<DashMap<String, String>> = LazyLock::new(DashMap::new);

/// Cache a user's email for notify-event enrichment.
///
/// Call wherever the durable user record is loaded during authentication —
/// the same call sites that warm the consent cache — so identity coverage
/// tracks consent coverage. No-op for empty inputs.
pub fn cache_user_email(user_id: &str, email: &str) {
    if user_id.is_empty() || email.is_empty() {
        return;
    }
    USER_EMAILS.insert(user_id.to_owned(), email.to_owned());
}

/// Resolve a cached email for `user_id`, or `None` when the user has not been
/// seen on this pod since its last cold start.
///
/// Returns an owned clone so the caller holds no map guard across the tracing
/// hot path.
///
/// ```
/// use pierre_services::analytics::{cache_user_email, resolve_user_email};
/// assert_eq!(resolve_user_email("u-doc"), None);
/// cache_user_email("u-doc", "jane@acme.com");
/// assert_eq!(resolve_user_email("u-doc").as_deref(), Some("jane@acme.com"));
/// cache_user_email("u-doc", ""); // empty email is a no-op, prior value kept
/// assert_eq!(resolve_user_email("u-doc").as_deref(), Some("jane@acme.com"));
/// ```
#[must_use]
pub fn resolve_user_email(user_id: &str) -> Option<String> {
    USER_EMAILS.get(user_id).map(|e| e.value().clone())
}

// =============================================================================
// Trait
// =============================================================================

/// Per-user analytics consent cache for the `PostHog` notify sink.
///
/// Implementations are either a live `PostHog`-backed cache or a noop. The
/// messaging-funnel, tool, and command events are emitted via the unified
/// `info!(target: "notify", ...)` path and captured by tronc's `NotifyLayer`
/// through `PierreAnalyticsProvider`; this trait only owns the consent state
/// that the provider's product-tier gate consults.
pub trait AnalyticsTracker: Send + Sync {
    // -- Consent --

    /// Update the consent cache for a user (called on consent change and session start)
    fn set_consent(&self, user_id: &str, enabled: bool);

    /// Returns `true` if the cache already has a known consent value for this user.
    ///
    /// Used by hydration callsites to skip a database fetch when the pod has already
    /// seen this user since the last cold start.
    fn has_consent_cached(&self, user_id: &str) -> bool;

    /// Returns `true` only when this user has a cached, affirmative analytics
    /// consent. The `PostHog` notify sink calls this to gate product-tier events;
    /// operational events are consent-exempt and never reach it.
    fn is_consented(&self, hashed_user_id: &str) -> bool;

    /// Populate the consent cache from a durable source (typically the users table)
    /// only if the user is not already cached.
    ///
    /// Unlike `set_consent`, this never overwrites an existing entry — so an in-memory
    /// consent flip made via `/privacy on|off` cannot be clobbered by a later hydration
    /// call that read a stale row from the database.
    fn hydrate_consent(&self, user_id: &str, enabled: bool);
}

// =============================================================================
// Noop implementation
// =============================================================================

/// No-op tracker used when `PostHog` is disabled or unconfigured
struct NoopAnalyticsTracker;

impl AnalyticsTracker for NoopAnalyticsTracker {
    fn set_consent(&self, _: &str, _: bool) {}
    fn has_consent_cached(&self, _: &str) -> bool {
        false
    }
    fn is_consented(&self, _: &str) -> bool {
        false
    }
    fn hydrate_consent(&self, _: &str, _: bool) {}
}

// =============================================================================
// PostHog implementation
// =============================================================================

/// Attempt to create a `PostHog` consent-cache tracker from environment.
///
/// Returns `None` when `POSTHOG_API_KEY` is unset or empty — without a
/// configured `PostHog` project there is nothing for the consent cache to
/// gate, so the noop tracker is installed instead. The actual capture HTTP
/// is owned by tronc's `NotifyLayer` (via `PierreAnalyticsProvider`); this
/// tracker only holds the per-user consent state the provider's product-tier
/// gate consults.
#[cfg(feature = "analytics-posthog")]
fn create_posthog_tracker() -> Option<Box<dyn AnalyticsTracker>> {
    let api_key = env::var("POSTHOG_API_KEY").ok()?;
    if api_key.is_empty() {
        return None;
    }

    let host = env::var("POSTHOG_HOST").unwrap_or_else(|_| "https://us.i.posthog.com".to_owned());

    info!(host = %host, "PostHog analytics consent cache initialized");

    Some(Box::new(PostHogTracker {
        consent_cache: DashMap::new(),
    }))
}

/// Stub when the `analytics-posthog` feature is disabled.
#[cfg(not(feature = "analytics-posthog"))]
fn create_posthog_tracker() -> Option<Box<dyn AnalyticsTracker>> {
    None
}

#[cfg(feature = "analytics-posthog")]
struct PostHogTracker {
    /// Per-user consent cache: `hashed_user_id` to consented
    /// Missing entry = no consent known, skip events
    consent_cache: DashMap<String, bool>,
}

#[cfg(feature = "analytics-posthog")]
impl PostHogTracker {
    /// Check if a user has given analytics consent.
    /// Returns `false` if user is unknown or has not consented.
    fn has_consent(&self, hashed_user_id: &str) -> bool {
        self.consent_cache.get(hashed_user_id).is_some_and(|v| *v)
    }
}

#[cfg(feature = "analytics-posthog")]
impl AnalyticsTracker for PostHogTracker {
    fn set_consent(&self, user_id: &str, enabled: bool) {
        self.consent_cache.insert(user_id.to_owned(), enabled);
    }

    fn has_consent_cached(&self, user_id: &str) -> bool {
        self.consent_cache.contains_key(user_id)
    }

    fn is_consented(&self, hashed_user_id: &str) -> bool {
        self.has_consent(hashed_user_id)
    }

    fn hydrate_consent(&self, user_id: &str, enabled: bool) {
        self.consent_cache
            .entry(user_id.to_owned())
            .or_insert(enabled);
    }
}
