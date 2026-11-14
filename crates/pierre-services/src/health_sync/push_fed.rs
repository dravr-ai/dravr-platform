// ABOUTME: Whether a provider's wellness reaches an athlete by push, so the pull sync stands down
// ABOUTME: Judged from the athlete's read plan; a history walk's scope reads the session for the days before the grant
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::future::Future;

use chrono::{DateTime, NaiveDate, NaiveTime, Utc};

use dravr_enforme::error::{EnformeError, EnformeResult};
use pierre_core::models::TenantId;
use pierre_database::RepositoryRegistry;
use pierre_providers::backend_resolver::{pushed_backend_for, read_plan, sync_backend};
use tracing::{info, warn};
use uuid::Uuid;

use super::token_tenant_id;
use crate::garmin_push;

tokio::task_local! {
    /// The athlete whose history walk runs on this task.
    static HISTORY_WALK: HistoryWalkScope;
}

/// Whose history walk runs on a task, and from which day the push holds
/// their wellness.
#[derive(Debug, Clone, Copy)]
struct HistoryWalkScope {
    user: Uuid,
    pushed_from: NaiveDate,
}

/// Run `work`, the wellness half of `user`'s history walk (carnet#1482).
///
/// A live push grant beside the athlete's session is their wellness source
/// ([`wellness_fed_by_push`]), so no pull reads the session; within this
/// scope it is read once, for the nights the provider pushed before the
/// grant existed. Only those are written: a day from `pushed_from` on is the
/// push's ([`history_walk_writes`]), and a sync writes a whole day, so
/// writing it would blank the readings only the push carries and re-file the
/// row under the session's source. The scope is task-local, so it is set
/// inside the task that runs the sync.
pub async fn history_walk_scope<F: Future>(
    user: Uuid,
    pushed_from: NaiveDate,
    work: F,
) -> F::Output {
    HISTORY_WALK
        .scope(HistoryWalkScope { user, pushed_from }, work)
        .await
}

/// Whether the running task is `user`'s history walk.
fn in_history_walk_of(user: Uuid) -> bool {
    HISTORY_WALK
        .try_with(|walking| walking.user == user)
        .unwrap_or(false)
}

/// Whether a sync on this task writes `user_id`'s wellness for `day`: every
/// day, except within the user's history walk, where a day the push holds
/// is left to it ([`history_walk_scope`]).
pub(super) fn history_walk_writes(user_id: &str, day: NaiveDate) -> bool {
    HISTORY_WALK
        .try_with(|walking| walking.user.to_string() != user_id || day < walking.pushed_from)
        .unwrap_or(true)
}

/// Whether a sync on this task writes `user_id`'s night that `ended` then:
/// every night, except within the user's history walk, where a night
/// ending from the push's first day on is left to it. The cutoff is that
/// day's first instant, the same day the push's first stored night counts
/// toward ([`history_walk_scope`]).
pub(super) fn history_walk_writes_night(user_id: &str, ended: DateTime<Utc>) -> bool {
    HISTORY_WALK
        .try_with(|walking| {
            walking.user.to_string() != user_id
                || ended < walking.pushed_from.and_time(NaiveTime::MIN).and_utc()
        })
        .unwrap_or(true)
}

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
/// Within the user's history walk ([`history_walk_scope`]) the session is
/// read once, for what came before the grant.
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
    if in_history_walk_of(user) {
        return Ok(false);
    }
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
