// ABOUTME: Whether an activity-cache write lands a new training session for the athlete
// ABOUTME: Sends the athlete's per_session persona digest, off the fetch's path, when one lands
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use chrono::{Duration, Utc};
use pierre_core::models::{Activity, TenantId};
use pierre_services::notification_digest_scheduler::session_landed;
use tracing::warn;
use uuid::Uuid;

use super::activity_cache_retention_days;
use crate::runtime::ToolRuntime;

/// Whether writing `activities` through lands a training session: one that
/// starts after every session the cache already holds for this athlete from
/// any provider, or any session at all when it holds none.
///
/// Both writers to the activity cache ask this before their upsert, so every
/// way a session reaches the platform — a provider webhook, a chat turn's
/// fetch, the nightly capture sweep, a group snapshot — counts, and a session
/// already cached never counts twice. The comparison spans every provider:
/// the same ride recorded by two of them lands once, and a provider connected
/// later writing its history lands nothing that is not newer than what the
/// athlete already had. Read only when a notification service
/// is wired, since the landing is only ever used to send the `per_session`
/// persona digest. A failed read is logged and lands nothing.
pub(super) async fn lands_a_session(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant_id: &TenantId,
    provider: &str,
    activities: &[Activity],
) -> bool {
    if runtime.notification_service().is_none() {
        return false;
    }
    let Some(newest) = activities.iter().map(Activity::start_date).max() else {
        return false;
    };
    let now = Utc::now();
    let window_start = now - Duration::days(activity_cache_retention_days());
    match runtime
        .repos()
        .activity_cache
        .get_cached_activities(user_id, tenant_id, None, window_start, newest.max(now), 1)
        .await
    {
        Ok(cached) => cached
            .first()
            .is_none_or(|latest| newest > latest.start_date()),
        Err(e) => {
            warn!(
                user_id = %user_id,
                provider = %provider,
                error = %e,
                "Activity cache: newest-session read failed; no session landing"
            );
            false
        }
    }
}

/// Send the athlete's `per_session` persona digest for a session that just
/// landed, off the fetch's path: the digest may reach a linked chat channel,
/// and a turn's fetch must not wait on that. Best-effort — a failure is
/// logged, and the withheld notifications wait for the next session.
pub(super) fn deliver_session_digest(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant_id: TenantId,
) {
    let Some(service) = runtime.notification_service().map(Arc::clone) else {
        return;
    };
    let repos = Arc::clone(runtime.repos());
    let strings = Arc::clone(runtime.messaging_strings_registry());
    tokio::spawn(async move {
        if let Err(e) = session_landed(&repos, &service, &strings, user_id, tenant_id).await {
            warn!(
                user_id = %user_id,
                error = %e,
                "per-session persona digest failed (best-effort)"
            );
        }
    });
}
