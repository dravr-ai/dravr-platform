// ABOUTME: Shared failed-outbound enqueue — renders + persists + queues a dropped outbound message
// ABOUTME: for the retry worker, reused by the synchronous reply path and the backfill-completion push.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::AppError;
use pierre_core::models::messaging::OutgoingMessage;
use pierre_core::models::TenantId;
use pierre_database::backends::{InsertMessageParams, MessagingRepository};
use pierre_messaging::channel::MessagingChannel;
use tracing::info;
use uuid::Uuid;

use super::content_body_text;

/// Routing primitives for [`enqueue_failed_outbound`], bundled into one struct so
/// the helper stays within clippy's argument-count budget — the tenant / session /
/// channel context it needs is intrinsically wide.
pub(crate) struct FailedOutbound<'a> {
    /// Tenant that owns the persisted message row — the session/user tenant, so
    /// the whole turn (inbound + assistant) reads as one unit under one tenant.
    pub message_tenant_id: TenantId,
    /// Tenant that owns the queue row — the channel-owner/bot tenant, so the
    /// background retry worker loads the right channel config to re-send. Differs
    /// from `message_tenant_id` for a cross-tenant bot, coincides for a self-host;
    /// the queue->message FK is single-column (`message_id`), so they may differ.
    pub queue_tenant_id: TenantId,
    /// Messaging session the persisted outbound row belongs to.
    pub session_id: &'a str,
    /// Channel-native user id recorded on the queue row, or `None`.
    pub user_id: Option<&'a str>,
    /// Channel slug (e.g. `"whatsapp"`) for the message row + queue row.
    pub channel: &'a str,
}

/// Render, persist, and enqueue a failed outbound message for retry delivery.
///
/// The single source of truth for the "a channel send failed — don't drop it"
/// path, shared by the synchronous reply (`dispatch::send_outbound_response`) and
/// the backfill-completion push (`ServerBackfillNotifier::push_backfill_complete`).
/// Renders the outgoing message to the channel's native payload, persists the
/// outbound message row (the FK target for the queue entry), then enqueues it so
/// the background retry worker re-sends with backoff and dead-letters after the
/// max attempts. Returns an error if any step fails (rendering, persistence, or
/// enqueue) so the caller can log it.
pub(crate) async fn enqueue_failed_outbound(
    db: &dyn MessagingRepository,
    adapter: &dyn MessagingChannel,
    outgoing: &OutgoingMessage,
    params: &FailedOutbound<'_>,
) -> Result<(), AppError> {
    let payload = adapter
        .render(outgoing)
        .map_err(|e| AppError::internal(format!("Failed to render for retry: {e}")))?;

    let payload_str = payload.to_string();

    // Persist the outbound message record first (FK requirement for queue entry).
    // Use a unique retry-prefixed ID to avoid colliding with the (tenant_id, channel_message_id)
    // uniqueness constraint — retry messages have no real channel ID yet.
    let out_msg_id = Uuid::new_v4().to_string();
    let retry_channel_msg_id = format!("retry-{out_msg_id}");
    let body = content_body_text(&outgoing.content);
    let correlation_str = outgoing.turn_id.to_string();
    let out_params = InsertMessageParams {
        id: &out_msg_id,
        // Message row shares the session/user tenant; the queue row below stays on
        // queue_tenant_id so the retry worker loads the bot's channel config to
        // re-send. The queue->message FK is single-column (message_id), so the
        // differing tenants don't break it.
        tenant_id: params.message_tenant_id,
        session_id: params.session_id,
        direction: "outbound",
        channel_type: params.channel,
        channel_message_id: &retry_channel_msg_id,
        sender_id: "pierre",
        content_type: "text",
        content_body: body.as_deref(),
        correlation_id: &correlation_str,
        raw_payload: Some(&payload_str),
    };
    let inserted = db.insert_message(&out_params).await?;
    if !inserted {
        return Err(AppError::internal(
            "Failed to persist retry message: duplicate channel_message_id",
        ));
    }

    let queue_id = Uuid::new_v4().to_string();
    db.enqueue_outbound(
        &queue_id,
        &out_msg_id,
        params.queue_tenant_id,
        params.user_id,
        params.channel,
        &payload_str,
    )
    .await?;

    info!(
        queue_id = %queue_id,
        channel = %params.channel,
        "Outbound message enqueued for retry"
    );
    Ok(())
}
