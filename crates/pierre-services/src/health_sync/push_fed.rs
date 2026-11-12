// ABOUTME: Whether a provider's wellness reaches an athlete by push, so the pull sync stands down
// ABOUTME: Judged from the athlete's read plan: a live push grant stays the wellness source beside a session
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use dravr_enforme::error::{EnformeError, EnformeResult};
use pierre_core::models::TenantId;
use pierre_database::RepositoryRegistry;
use pierre_providers::backend_resolver::{pushed_backend_for, read_plan, sync_backend};
use tracing::{info, warn};
use uuid::Uuid;

use super::token_tenant_id;
use crate::garmin_push;

/// Whether `provider`'s wellness reaches `user` in `tenant` by push, so no
/// pull of it runs.
///
/// It does when the provider's pushed backend ([`pushed_backend_for`]) is a
/// live grant in the athlete's read plan there ([`read_plan`], the one rule
/// routing and every status surface read, carnet#1302) — the fallback behind
/// their session, or the primary of a grant held alone — and its
/// connection's last reported permissions (none reported reads as all
/// granted) include the health export.
///
/// The pushes then stay the wellness source while the session serves the
/// activity reads, so no day is written twice (carnet#1494). A dead grant is
/// in no plan, so the session beside it is pulled. A user who withheld the
/// health export is pulled as before: nothing else serves their wellness.
///
/// # Errors
///
/// When the pushed connection's permissions cannot be read.
pub(super) async fn wellness_fed_by_push(
    registry: &RepositoryRegistry,
    user: Uuid,
    tenant: TenantId,
    provider: &str,
) -> EnformeResult<bool> {
    let Some(pushed) = pushed_backend_for(provider) else {
        return Ok(false);
    };
    if !read_plan(registry, user, tenant, pushed)
        .await
        .serves(pushed)
    {
        return Ok(false);
    }
    let grants = registry
        .provider_connections
        .provider_grants(user, tenant, pushed)
        .await
        .map_err(|e| {
            EnformeError::store(format!("Failed to read the {pushed} permissions: {e}"))
        })?;
    Ok(garmin_push::shares_wellness(grants.as_deref()))
}

/// Whether a pull of `provider`'s wellness for `user_id` stands down because
/// it reaches them by push (`wellness_fed_by_push`).
///
/// It is judged in the tenant of the row the sync would read
/// ([`sync_backend`]), else of the pushed backend's row. A caller asks before
/// it starts a sync: standing down is nothing to pull, never a failure, so it
/// raises no error, no alert and no reconnect. A lookup that fails is logged and reads as `false`: the sync
/// then runs and reports what it finds.
pub async fn wellness_pull_stands_down(
    registry: &RepositoryRegistry,
    user_id: &str,
    provider: &str,
) -> bool {
    match pull_fed_by_push(registry, user_id, provider).await {
        Ok(true) => {
            info!(
                user_id,
                provider, "Wellness arrives by push; the sync has nothing to pull"
            );
            true
        }
        Ok(false) => false,
        Err(e) => {
            warn!(user_id, provider, error = %e, "Could not tell whether the provider pushes this wellness; syncing");
            false
        }
    }
}

/// [`wellness_pull_stands_down`]'s fallible half.
async fn pull_fed_by_push(
    registry: &RepositoryRegistry,
    user_id: &str,
    provider: &str,
) -> EnformeResult<bool> {
    let Some(pushed) = pushed_backend_for(provider) else {
        return Ok(false);
    };
    let user = user_id
        .parse::<Uuid>()
        .map_err(|e| EnformeError::store(format!("Invalid user_id UUID: {e}")))?;
    let tokens = registry
        .oauth_tokens
        .get_tokens(user, None)
        .await
        .map_err(|e| EnformeError::store(format!("Failed to look up user tokens: {e}")))?;
    let backend = sync_backend(provider);
    let row = tokens
        .iter()
        .find(|t| t.provider == backend)
        .or_else(|| tokens.iter().find(|t| t.provider == pushed));
    match row {
        Some(row) => wellness_fed_by_push(registry, user, token_tenant_id(row)?, provider).await,
        None => Ok(false),
    }
}
