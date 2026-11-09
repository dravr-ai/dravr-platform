// ABOUTME: Read-through cache of where a user's consent to an OAuth client stands, for resource servers
// ABOUTME: Bounds the grant read a delegated access token costs to one per client per window, and forgets on a change
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Whether the consent behind a delegated token still stands.
//!
//! An OAuth access token is a self-contained JWT good for an hour, while the
//! consent it rides on can be withdrawn at any moment. `/mcp` and A2A ask here
//! on every delegated call, so a withdrawal stops the token within
//! [`GRANT_STANDING_TTL`] instead of at its expiry.
//!
//! The window is what bounds the database: a client calling every second costs
//! one read per window, however many replicas serve it when the cache is shared.
//! A change forgets the entry, so on a cache shared across replicas (Redis, in
//! production) it takes effect on the next call everywhere; on a process-local
//! cache another replica catches up when its entry expires.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use pierre_cache::{Cache, CacheKey, CacheResource};
use pierre_core::errors::AppResult;
use pierre_core::models::{ClientGrantStanding, TenantId};
use pierre_database::backends::OAuth2ServerRepository;
use tracing::warn;
use uuid::Uuid;

/// How long a replica may answer from a read grant before reading it again.
pub const GRANT_STANDING_TTL: Duration = Duration::from_secs(30);

/// The `provider` segment of the entries' keys, which keeps them apart from
/// the provider data sharing the cache.
const KEY_NAMESPACE: &str = "_oauth_client_grant";

/// Reads and forgets where consents stand.
#[derive(Clone)]
pub struct GrantStandings {
    /// Where grants are read; the server's one repository, shared with every
    /// route and task that holds it
    repository: Arc<dyn OAuth2ServerRepository>,
    /// The server's one cache, shared so a `forget` from the consent and
    /// revocation routes reaches the entry the resource servers read
    cache: Arc<Cache>,
    /// How long an entry answers before the grant is read again
    ttl: Duration,
}

impl fmt::Debug for GrantStandings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GrantStandings")
            .field("ttl", &self.ttl)
            .finish_non_exhaustive()
    }
}

impl GrantStandings {
    /// Standings read from `repository` and held in `cache` for
    /// [`GRANT_STANDING_TTL`].
    #[must_use]
    pub fn new(repository: Arc<dyn OAuth2ServerRepository>, cache: Arc<Cache>) -> Self {
        Self {
            repository,
            cache,
            ttl: GRANT_STANDING_TTL,
        }
    }

    /// Hold each entry for `ttl` instead.
    #[must_use]
    pub const fn with_ttl(mut self, ttl: Duration) -> Self {
        self.ttl = ttl;
        self
    }

    /// Where `user_id`'s consent to `client_id` stands in `tenant_id`.
    ///
    /// A cache that cannot answer or store costs a database read and nothing
    /// else.
    ///
    /// # Errors
    /// A database error when the grant cannot be read: the caller refuses the
    /// request, as it does when the user's quota cannot be read.
    pub async fn read(
        &self,
        user_id: Uuid,
        tenant_id: &str,
        client_id: &str,
    ) -> AppResult<ClientGrantStanding> {
        let key = standing_key(user_id, tenant_id, client_id);
        match self.cache.get::<ClientGrantStanding>(&key).await {
            Ok(Some(standing)) => return Ok(standing),
            Ok(None) => {}
            Err(e) => warn!(%user_id, error = %e, "Grant standing cache read failed"),
        }
        let standing = self
            .repository
            .client_grant_standing(&user_id.to_string(), tenant_id, client_id)
            .await?;
        if let Err(e) = self.cache.set(&key, &standing, self.ttl).await {
            warn!(%user_id, error = %e, "Grant standing cache write failed");
        }
        Ok(standing)
    }

    /// Drop the entry a change to this consent has made stale. A cache that
    /// cannot drop it leaves the entry to expire, which is the window's bound.
    pub async fn forget(&self, user_id: Uuid, tenant_id: &str, client_id: &str) {
        let key = standing_key(user_id, tenant_id, client_id);
        if let Err(e) = self.cache.invalidate(&key).await {
            warn!(%user_id, error = %e, "Grant standing cache invalidation failed");
        }
    }
}

/// The entry of one `(user, tenant, client)`. The tenant rides in the resource
/// because a grant stores it as text, whatever form it holds.
fn standing_key(user_id: Uuid, tenant_id: &str, client_id: &str) -> CacheKey {
    CacheKey::new(
        TenantId::nil(),
        user_id,
        KEY_NAMESPACE.to_owned(),
        CacheResource::Custom(format!("{tenant_id}/{client_id}")),
    )
}
