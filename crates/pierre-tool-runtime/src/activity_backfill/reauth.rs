// ABOUTME: What a background activity job does when it finds its provider session dead
// ABOUTME: Flag the connection, tell the athlete once, answer the conversation that asked — never for a walk-only session
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use chrono::{DateTime, Utc};
use pierre_core::models::{ReauthMark, TenantId};
use pierre_providers::backend_resolver::session_beside_grant;
use pierre_services::training_history_read::walk_only_session;
use tracing::{info, warn};
use uuid::Uuid;

use super::ActivityBackfillJob;
use crate::protocol::reauth_notice::{flag_needs_reauth, ReauthNotice};
use crate::runtime::ToolRuntime;

/// Reason recorded on a connection a backfill flags: the word the capture
/// sweep writes for the same condition, so `last_error` reads one way.
pub(super) const SESSION_EXPIRED: &str = "session_expired";

/// React to a backfill that found `job`'s provider session dead: flag the
/// connection, tell the athlete once, and answer the conversation that asked.
pub(super) async fn notify_backfill_reauth(
    job: &ActivityBackfillJob,
    attempt_started_at: DateTime<Utc>,
) {
    backfill_session_expired(
        &job.resources,
        job.user_id,
        job.tenant_id,
        &job.provider_name,
        job.pierre_conversation_id.as_deref(),
        attempt_started_at,
    )
    .await;
}

/// What a backfill that began at `attempt_started_at` does when it finds the
/// `provider` session of `(user_id, tenant_id)` dead.
///
/// 1. Flag the connection and send the once-per-transition reconnect notice,
///    through [`flag_needs_reauth`] like every other flagging path. A
///    connection the athlete reconnected after the backfill began is left
///    active and sent nothing: its session is the new one, and "your session
///    expired" over it would contradict the connection the app shows.
/// 2. Answer the conversation that asked, when there is one and messaging is
///    wired ([`crate::runtime::BackfillNotifier::push_provider_reauth`]),
///    unless the notice just reached the athlete's chats — they would read the
///    same link twice. That answer is not deduped: an athlete who asks again
///    while still disconnected gets the link again, so a first link that was
///    broken or never clicked does not silence them. A usr/pwd session beside
///    a grant that serves its provider ([`session_beside_grant`]) is never
///    answered: the grant still serves the athlete (carnet#1386).
///
/// `pub` so the integration suite can drive it without a scraper.
pub async fn backfill_session_expired(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant_id: TenantId,
    provider: &str,
    pierre_conversation_id: Option<&str>,
    attempt_started_at: DateTime<Utc>,
) {
    if walk_only_session_expired(runtime, user_id, tenant_id, provider).await {
        return;
    }
    let notice = match flag_needs_reauth(
        runtime,
        user_id,
        tenant_id,
        provider,
        SESSION_EXPIRED,
        attempt_started_at,
    )
    .await
    {
        Ok((ReauthMark::ReconnectedSince, _)) => {
            info!(
                provider = %provider,
                "Activity backfill: reconnected since the backfill began; nothing to send"
            );
            return;
        }
        Ok((_, notice)) => notice,
        Err(e) => {
            warn!(error = %e, provider = %provider, "Activity backfill: could not flag the expired session");
            ReauthNotice::default()
        }
    };
    if notice.chat_channels > 0
        || session_beside_grant(runtime.repos(), user_id, tenant_id, provider).await
    {
        return;
    }
    let (Some(conversation_id), Some(notifier)) =
        (pierre_conversation_id, runtime.backfill_notifier())
    else {
        return;
    };
    notifier
        .push_provider_reauth(user_id, tenant_id, conversation_id, provider)
        .await;
}

/// Whether the dead `provider` session is one only history walks read
/// ([`walk_only_session`]). A walk that finds it dead ends incomplete on its
/// own row: no flag on it, no notice, no answer.
async fn walk_only_session_expired(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant_id: TenantId,
    provider: &str,
) -> bool {
    let walk_only = walk_only_session(runtime.repos(), user_id, tenant_id, provider).await;
    if walk_only {
        info!(
            provider = %provider,
            "Activity backfill: a walk-only session expired; nothing to flag"
        );
    }
    walk_only
}
