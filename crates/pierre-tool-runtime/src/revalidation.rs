// ABOUTME: Single-flight registry for background activity-cache revalidations, one per (user, tenant)
// ABOUTME: Every surface that refreshes a stale cache in the background claims its slot here first

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Background revalidation slots.
//!
//! A stale activity cache is served and refreshed behind the reader's back —
//! by a group snapshot, by a chat turn, by the Home page. Each of those can
//! arrive many times while one refresh is still running, and a provider read
//! can be a two-minute headless scrape, so they share one slot per `(user,
//! tenant)` here rather than each keeping its own: a refresh the chat path
//! started is one the Home page does not start again.

use std::collections::HashSet;
use std::sync::{Arc, LazyLock, Mutex as StdMutex, PoisonError};

use pierre_core::models::TenantId;
use uuid::Uuid;

/// Upper bound on a single revalidation.
///
/// A sciotte/Garmin scrape that hangs on
/// a provider-side throttle must not hold the single-flight slot (nor the
/// per-profile Chrome `SingletonLock`) open indefinitely, blocking every later
/// revalidation for the same user. Generous relative to a healthy ~2-minute
/// scrape so it only fires on a genuine stall.
pub const REVALIDATION_TIMEOUT_SECS: u64 = 240;

/// Tracks which `(user, tenant)` background revalidations are in flight so
/// concurrent stale-cache chat turns collapse onto a single refresh.
///
/// Without this, every stale-cache turn spawned its own revalidation. The
/// per-profile Chrome `SingletonLock` (see [`pierre_providers`]) serializes
/// those scrapes rather than crashing, so N rapid turns queue N ~2-minute
/// scrapes behind one lock — the later ones redundant by the time they run.
/// Stale-while-revalidate only needs one in-flight refresh per user; the rest
/// are dropped. Entries are removed when the revalidation task finishes (or
/// times out), so the slot frees for the next genuinely-stale turn.
///
/// Production code shares one registry via [`Self::global`]; tests construct
/// isolated instances with [`Self::new`].
pub struct RevalidationRegistry {
    in_flight: Arc<StdMutex<HashSet<(Uuid, TenantId)>>>,
}

impl RevalidationRegistry {
    /// Create an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            in_flight: Arc::new(StdMutex::new(HashSet::new())),
        }
    }

    /// The process-global registry shared across every chat turn.
    #[must_use]
    pub fn global() -> &'static Self {
        static GLOBAL: LazyLock<RevalidationRegistry> = LazyLock::new(RevalidationRegistry::new);
        &GLOBAL
    }

    /// Claim the revalidation slot for a user. Returns a guard that frees the
    /// slot on drop, or `None` if a revalidation is already in flight (the
    /// caller then skips spawning a duplicate).
    pub fn try_claim(&self, key: (Uuid, TenantId)) -> Option<RevalidationGuard> {
        // Recover from poisoning: the set stays consistent even if a holder
        // panicked, and refusing all future revalidations would be worse.
        let mut set = self
            .in_flight
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if set.insert(key) {
            Some(RevalidationGuard {
                in_flight: Arc::clone(&self.in_flight),
                key,
            })
        } else {
            None
        }
    }
}

impl Default for RevalidationRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Frees a claimed revalidation slot when dropped, including on panic or
/// timeout, so a single failed refresh never wedges a user's slot shut.
pub struct RevalidationGuard {
    in_flight: Arc<StdMutex<HashSet<(Uuid, TenantId)>>>,
    key: (Uuid, TenantId),
}

impl Drop for RevalidationGuard {
    fn drop(&mut self) {
        let mut set = self
            .in_flight
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        set.remove(&self.key);
    }
}
