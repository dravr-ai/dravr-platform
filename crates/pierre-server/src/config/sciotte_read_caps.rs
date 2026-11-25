// ABOUTME: Resolves an athlete's session-read caps: their tier, then each sciotte_reads.* parameter
// ABOUTME: The SessionReadLimits the provider rate limiter is bound to at startup (carnet#1661)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The athlete's session-read caps, as the admin config holds them.
//!
//! The provider rate limiter counts every read of an athlete's signed-in
//! session against caps by tier
//! ([`pierre_providers::request_budget::SessionReadLimits`]). This resolver
//! reads the athlete's tier from their `users` row (Starter when the row
//! cannot be read, as the quota ladder does), then each
//! `sciotte_reads.<cap>.<tier>` parameter for that athlete: their own
//! override, else the system-wide one, else the catalog default, which is
//! the compiled cap. A tenant row is not read: the cap belongs to the
//! athlete's session, whatever tenant the read is made in.
//!
//! Overrides are read uncached on each resolution, and the limiter keeps an
//! answer for one minute, so a `pierre-cli config set` or a tier change
//! reaches every instance within that minute, with no restart.

use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use pierre_core::models::{SessionReadCap, UserTier};
use pierre_database::backends::UserRepository;
use pierre_providers::request_budget::{AthleteReadCaps, SessionReadLimits};
use pierre_runtime_context::{AdminConfigLookup, ConfigLookupScope};
use serde_json::Value;
use tracing::warn;
use uuid::Uuid;

/// The session-read caps of each athlete, read from the admin config.
pub struct SciotteReadCaps {
    /// The admin config the caps are parameters of; `None` when it could
    /// not start, which leaves every athlete on their tier's compiled caps.
    /// Shared with every other reader of the admin config.
    config: Option<Arc<dyn AdminConfigLookup>>,
    /// Where an athlete's tier is read. Shared with the server's registry.
    users: Arc<dyn UserRepository>,
}

impl fmt::Debug for SciotteReadCaps {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SciotteReadCaps")
            .field("config", &self.config.is_some())
            .finish_non_exhaustive()
    }
}

impl SciotteReadCaps {
    /// Caps read from `config`, for tiers read from `users`.
    #[must_use]
    pub fn new(config: Option<Arc<dyn AdminConfigLookup>>, users: Arc<dyn UserRepository>) -> Self {
        Self { config, users }
    }

    /// `user_id`'s tier; Starter when their row cannot be read.
    async fn tier(&self, user_id: Uuid) -> UserTier {
        match self.users.get_global(user_id).await {
            Ok(Some(user)) => user.tier,
            Ok(None) => UserTier::Starter,
            Err(e) => {
                warn!(%user_id, error = %e, "the athlete's tier could not be read; their session reads are held to the Starter caps");
                UserTier::Starter
            }
        }
    }
}

/// The cap a stored `value` of `key` names; `None`, logged, when it is not
/// a whole number a cap can hold.
fn cap_value(key: &str, value: &Value) -> Option<u32> {
    let cap = value.as_u64().and_then(|n| u32::try_from(n).ok());
    if cap.is_none() {
        warn!(key, %value, "not a session-read cap; using the compiled one");
    }
    cap
}

#[async_trait]
impl SessionReadLimits for SciotteReadCaps {
    async fn caps(&self, user_id: Uuid) -> AthleteReadCaps {
        let tier = self.tier(user_id).await;
        let mut resolved = AthleteReadCaps::compiled(tier);
        let Some(config) = &self.config else {
            return resolved;
        };
        let user = user_id.to_string();
        let scope = ConfigLookupScope {
            user_id: Some(&user),
            tenant_id: None,
        };
        for cap in SessionReadCap::ALL {
            let key = cap.config_key(&resolved.tier);
            match config.get_value(&key, scope).await {
                Ok(Some(value)) => {
                    if let Some(value) = cap_value(&key, &value) {
                        cap.set(&mut resolved.caps, value);
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    warn!(%user_id, key, error = %e, "a session-read cap could not be read; using the compiled one");
                }
            }
        }
        resolved
    }
}
