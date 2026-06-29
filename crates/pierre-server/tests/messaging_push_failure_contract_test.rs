// ABOUTME: Contract test — a failed backfill-completion push is queued for retry, not silently dropped
// ABOUTME: Models Meta WhatsApp error 131047 (out-of-24h-window) and pins push/sync-reply path parity
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
#![cfg(feature = "client-messaging")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::must_use_candidate,
    clippy::similar_names,
    clippy::uninlined_format_args
)]

use std::sync::Arc;

use chrono::{Duration, Utc};
use pierre_core::models::messaging::ChannelType;
use pierre_core::models::ConnectionType;
use pierre_database::RepositoryRegistry;
use pierre_mcp_server::services::backfill_notifier::ServerBackfillNotifier;
use pierre_messaging::channel::MessagingChannel;
use pierre_tool_runtime::runtime::BackfillNotifier;

// Shared messaging fixtures + channel fakes, pulled in via `#[path]` so this
// contract test reuses the one copy the notifier tests share.
#[path = "helpers/messaging_fixtures.rs"]
mod messaging_fixtures;
use messaging_fixtures::{
    cached_activity, create_test_db, seed_activity_cache, seed_conversation, seed_session,
    seed_user, strings, CapturingChannel, FailingChannel, FakeResolver,
};

/// A backfill-completion push whose channel send FAILS must persist + enqueue the
/// dropped notice for the background retry worker — exactly as the synchronous
/// reply path does (`dispatch::send_outbound_response` on a send error) — instead
/// of the old warn-and-drop. This models Meta `WhatsApp` error 131047
/// (out-of-24h-window / message-template-required): a real per-channel delivery
/// failure a Telegram-only fix never exercises, where the user previously got
/// NOTHING after a completed backfill. Pins that the push now QUEUES (path parity
/// with the live reply) so the worker retries with backoff + dead-letters.
#[tokio::test]
async fn failed_push_is_enqueued_for_retry_not_dropped() {
    let db = create_test_db().await;
    let repos: Arc<RepositoryRegistry> = Arc::new(db.repositories());
    let (user_uuid, tenant_id) = seed_user(&db).await;
    let user_id = user_uuid.to_string();
    let conversation_id = seed_conversation(&db, &user_id, tenant_id).await;
    // DM WhatsApp session (NULL channel_conversation_id) — the channel the Meta
    // 24h-window rule applies to.
    seed_session(
        &db,
        &user_id,
        tenant_id,
        "whatsapp",
        "14502244753",
        None,
        &conversation_id,
    )
    .await;

    // Warm the cache (as the backfill does) so the push renders a real list and
    // a channel send is attempted.
    seed_activity_cache(
        &db,
        user_uuid,
        tenant_id,
        &[cached_activity("a1", "Morning Trail Run", 2)],
    )
    .await;

    // The channel rejects every send with the non-retryable 131047 error.
    let failing = Arc::new(FailingChannel::default());
    let resolver = Arc::new(FakeResolver::new(
        failing.clone() as Arc<dyn MessagingChannel>
    ));
    let notifier = ServerBackfillNotifier::with_resolver(repos.clone(), strings(), resolver);

    // `after_ts` = 30 days ago so the 2-day-old cached activity is in-window.
    let after_ts = (Utc::now() - Duration::days(30)).timestamp();
    notifier
        .push_backfill_complete(
            user_uuid,
            tenant_id,
            &conversation_id,
            "strava",
            after_ts,
            1,
        )
        .await;

    // (a) Exactly one send attempt — the notifier itself must not retry; the
    //     durable queue + background worker own retry / backoff / dead-letter.
    assert_eq!(
        failing.attempts.lock().unwrap().len(),
        1,
        "the push must attempt exactly one send (no notifier-side retry loop)"
    );

    // (b) The dropped notice is now a PENDING outbound-queue entry. No channel
    //     link is seeded, so `resolve_channel_owner_tenant` falls back to the
    //     session tenant — the queue row owner is `tenant_id`.
    let pending = repos
        .messaging
        .get_pending_outbound(tenant_id, 10)
        .await
        .unwrap();
    assert!(
        !pending.is_empty(),
        "a failed push must enqueue a pending outbound entry, not drop it"
    );
    assert_eq!(
        pending[0].get("status").and_then(serde_json::Value::as_str),
        Some("pending"),
        "the enqueued entry must be pending for the worker to pick up: {:?}",
        pending[0]
    );
}

/// Guard the other direction: a SUCCESSFUL push must NOT enqueue anything — we
/// only queue on a send failure. Without this, an accidental always-enqueue would
/// flood the retry worker with already-delivered notices.
#[tokio::test]
async fn successful_push_does_not_enqueue() {
    let db = create_test_db().await;
    let repos: Arc<RepositoryRegistry> = Arc::new(db.repositories());
    let (user_uuid, tenant_id) = seed_user(&db).await;
    let user_id = user_uuid.to_string();
    let conversation_id = seed_conversation(&db, &user_id, tenant_id).await;
    seed_session(
        &db,
        &user_id,
        tenant_id,
        "whatsapp",
        "14502244753",
        None,
        &conversation_id,
    )
    .await;
    seed_activity_cache(
        &db,
        user_uuid,
        tenant_id,
        &[cached_activity("a1", "Morning Trail Run", 2)],
    )
    .await;

    // A normal channel that delivers successfully.
    let channel = Arc::new(CapturingChannel::for_channel(ChannelType::WhatsApp));
    let resolver = Arc::new(FakeResolver::new(
        channel.clone() as Arc<dyn MessagingChannel>
    ));
    let notifier = ServerBackfillNotifier::with_resolver(repos.clone(), strings(), resolver);

    let after_ts = (Utc::now() - Duration::days(30)).timestamp();
    notifier
        .push_backfill_complete(
            user_uuid,
            tenant_id,
            &conversation_id,
            "strava",
            after_ts,
            1,
        )
        .await;

    // Exactly one send, and it succeeded.
    assert_eq!(
        channel.sent.lock().unwrap().len(),
        1,
        "a successful push should send exactly once"
    );

    // ...and NOTHING is queued — the enqueue path fires only on failure.
    let pending = repos
        .messaging
        .get_pending_outbound(tenant_id, 10)
        .await
        .unwrap();
    assert!(
        pending.is_empty(),
        "a successful push must not enqueue any outbound entry: {pending:?}"
    );
}

/// Parity for the OTHER proactive push: a failed provider-reauth NUDGE (the "your
/// Garmin/Strava session expired, reconnect here" notice) must likewise be
/// persisted + enqueued for the retry worker, not warn-and-dropped. Same Meta
/// 131047 out-of-24h-window failure mode as the completion push — and the reauth
/// nudge is the one the user most needs: dropping it leaves them silently unable
/// to reconnect while every subsequent backfill keeps failing. Pins the reauth
/// path to the same `enqueue_failed_outbound` rail as the completion push + sync
/// reply.
#[tokio::test]
async fn failed_reauth_nudge_is_enqueued_for_retry_not_dropped() {
    let db = create_test_db().await;
    let repos: Arc<RepositoryRegistry> = Arc::new(db.repositories());
    let (user_uuid, tenant_id) = seed_user(&db).await;
    let user_id = user_uuid.to_string();
    let conversation_id = seed_conversation(&db, &user_id, tenant_id).await;
    // DM WhatsApp session (NULL channel_conversation_id) — the window-gated channel
    // the Meta 24h rule applies to.
    seed_session(
        &db,
        &user_id,
        tenant_id,
        "whatsapp",
        "14502244753",
        None,
        &conversation_id,
    )
    .await;
    // A connected sciotte_garmin: `mark_needs_reauth` UPDATEs an existing row, so
    // the connection must exist for the claim + nudge to fire.
    repos
        .provider_connections
        .register_connection(
            user_uuid,
            tenant_id,
            "sciotte_garmin",
            &ConnectionType::OAuth,
            None,
        )
        .await
        .unwrap();

    // The channel rejects the nudge send with the non-retryable 131047 error.
    let failing = Arc::new(FailingChannel::default());
    let resolver = Arc::new(FakeResolver::new(
        failing.clone() as Arc<dyn MessagingChannel>
    ));
    let notifier = ServerBackfillNotifier::with_resolver(repos.clone(), strings(), resolver);

    notifier
        .push_provider_reauth(user_uuid, tenant_id, &conversation_id, "sciotte_garmin")
        .await;

    // (a) Exactly one send attempt — the notifier itself must not retry; the
    //     durable queue + background worker own retry / backoff / dead-letter.
    assert_eq!(
        failing.attempts.lock().unwrap().len(),
        1,
        "the reauth nudge must attempt exactly one send (no notifier-side retry loop)"
    );

    // (b) The dropped nudge is now a PENDING outbound-queue entry, not lost. No
    //     channel link is seeded, so the queue row owner falls back to the session
    //     tenant — `tenant_id`.
    let pending = repos
        .messaging
        .get_pending_outbound(tenant_id, 10)
        .await
        .unwrap();
    assert!(
        !pending.is_empty(),
        "a failed reauth nudge must enqueue a pending outbound entry, not drop it"
    );
    assert_eq!(
        pending[0].get("status").and_then(serde_json::Value::as_str),
        Some("pending"),
        "the enqueued reauth nudge must be pending for the worker to pick up: {:?}",
        pending[0]
    );
}
