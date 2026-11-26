// ABOUTME: The lock that serialises clearing one provider for one user in one tenant, so two disconnects never both clear
// ABOUTME: A keyed in-process mutex on every backend, plus a transaction-scoped advisory lock across instances on PostgreSQL
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # The provider clear lock
//!
//! A disconnect reads whether the provider is still held, then clears it.
//! Two disconnects of one pair arriving together both read "held" before
//! either clears, so both revoke, both purge and both count
//! `provider.disconnected` (carnet#1838). The lock is held across the read
//! and the clear: the second request waits, then reads nothing held.
//!
//! Two layers, taken in this order:
//!
//! - **In-process**: a mutex per key, on every backend. It is the whole lock
//!   on `SQLite`, which serves one process. On PostgreSQL it queues the
//!   waiters of one instance in memory, so they do not each pin a pooled
//!   connection while the holder needs connections for the clear.
//! - **Across instances, PostgreSQL only**: `pg_advisory_xact_lock` on a
//!   transaction left open for the lock's life. The clear spans upstream
//!   HTTP calls and many repositories, so it cannot run inside that
//!   transaction; the transaction exists only to scope the lock. A request
//!   cancelled mid-clear drops it, the pool rolls it back on release, and the
//!   lock goes with it — a session-level lock would instead return to the
//!   pool still held.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, LazyLock, Mutex, PoisonError, Weak};

use pierre_core::models::TenantId;
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};
use uuid::Uuid;

/// One async mutex per key a clear is running or waiting on. Entries are
/// weak, so a key nobody holds is dropped on the next acquire.
static LOCAL_LOCKS: LazyLock<Mutex<HashMap<String, Weak<AsyncMutex<()>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// The key of one user's provider in one tenant: the user-facing provider, so
/// the two backends of a coalesced pair share it.
pub(crate) fn clear_key(user_id: Uuid, tenant_id: TenantId, provider: &str) -> String {
    format!("provider-clear:{user_id}:{tenant_id}:{provider}")
}

/// Wait for the in-process mutex of `key` and hold it.
pub(crate) async fn hold_local(key: &str) -> OwnedMutexGuard<()> {
    let mutex = {
        let mut locks = LOCAL_LOCKS.lock().unwrap_or_else(PoisonError::into_inner);
        locks.retain(|_, held| held.strong_count() > 0);
        let live = locks.get(key).and_then(Weak::upgrade);
        live.unwrap_or_else(|| {
            let fresh = Arc::new(AsyncMutex::new(()));
            locks.insert(key.to_owned(), Arc::downgrade(&fresh));
            fresh
        })
    };
    mutex.lock_owned().await
}

/// Held while one provider of one user is cleared; dropping it releases it.
///
/// Opaque: nothing reads through it, it only marks the span. A drop ends it
/// on every path, a cancelled request included: the pool rolls the advisory
/// lock's transaction back when it takes the connection back (it wrote
/// nothing), and the in-process mutex is freed.
pub struct ProviderClearLock {
    /// The transaction scoping the PostgreSQL advisory lock, `None` on
    /// `SQLite`. Declared first so it drops first: this instance's next
    /// waiter is let through only once the rollback is queued.
    #[cfg(feature = "postgresql")]
    _remote: Option<sqlx::Transaction<'static, sqlx::Postgres>>,
    /// The in-process layer.
    _local: OwnedMutexGuard<()>,
}

impl fmt::Debug for ProviderClearLock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProviderClearLock").finish_non_exhaustive()
    }
}

impl ProviderClearLock {
    /// A lock held by the in-process layer alone (`SQLite`).
    pub(crate) const fn local(local: OwnedMutexGuard<()>) -> Self {
        Self {
            #[cfg(feature = "postgresql")]
            _remote: None,
            _local: local,
        }
    }

    /// A lock held in process and by the advisory lock `remote` scopes.
    #[cfg(feature = "postgresql")]
    pub(crate) const fn with_remote(
        local: OwnedMutexGuard<()>,
        remote: sqlx::Transaction<'static, sqlx::Postgres>,
    ) -> Self {
        Self {
            _remote: Some(remote),
            _local: local,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::time;

    use super::*;

    #[tokio::test]
    async fn one_key_is_held_by_one_holder_and_other_keys_are_free() {
        let tenant = TenantId::from_uuid(Uuid::new_v4());
        let key = clear_key(Uuid::new_v4(), tenant, "strava");
        let other = clear_key(Uuid::new_v4(), tenant, "strava");

        let first = hold_local(&key).await;
        assert!(
            time::timeout(Duration::from_millis(50), hold_local(&key))
                .await
                .is_err(),
            "a second holder of a held key waits"
        );
        drop(
            time::timeout(Duration::from_millis(50), hold_local(&other))
                .await
                .expect("another key is not held"),
        );

        drop(first);
        drop(
            time::timeout(Duration::from_millis(50), hold_local(&key))
                .await
                .expect("a released key is free again"),
        );
        let locks = LOCAL_LOCKS.lock().unwrap_or_else(PoisonError::into_inner);
        assert!(
            locks.get(&key).is_none_or(|held| held.strong_count() == 0),
            "a key nobody holds keeps no live mutex"
        );
    }
}
