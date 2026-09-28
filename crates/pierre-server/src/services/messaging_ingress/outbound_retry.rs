// ABOUTME: Shared failed-outbound enqueue — renders + persists + queues a dropped outbound message
// ABOUTME: for the retry worker, reused by the synchronous reply, the backfill push and the reconnect notices.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use pierre_core::errors::AppError;
use pierre_core::models::messaging::OutgoingMessage;
use pierre_core::models::TenantId;
use pierre_database::backends::{InsertMessageParams, MessagingRepository};
use pierre_database::repositories::{EnqueueOutboundParams, OutboundReauthGuard};
use pierre_messaging::channel::MessagingChannel;
use tracing::info;
use uuid::Uuid;

use super::content_body_text;

/// The messaging session a failed outbound belongs to, where its message row
/// is recorded.
pub(crate) struct TranscriptRecord<'a> {
    /// Tenant that owns the persisted message row — the session/user tenant, so
    /// the whole turn (inbound + assistant) reads as one unit under one tenant.
    pub tenant_id: TenantId,
    /// Messaging session the persisted outbound row belongs to.
    pub session_id: &'a str,
}

/// Routing primitives for [`enqueue_failed_outbound`], bundled into one struct so
/// the helper stays within clippy's argument-count budget — the tenant / session /
/// channel context it needs is intrinsically wide.
pub(crate) struct FailedOutbound<'a> {
    /// Where the message row is recorded: the session the send answered.
    /// `None` for a send to a linked chat with no messaging session (a
    /// platform-initiated notice), which is queued without a message row.
    pub transcript: Option<TranscriptRecord<'a>>,
    /// Tenant that owns the queue row — the channel-owner/bot tenant, so the
    /// background retry worker loads the right channel config to re-send. Differs
    /// from the transcript tenant for a cross-tenant bot, coincides for a
    /// self-host; the queue->message FK is single-column (`message_id`), so they
    /// may differ.
    pub queue_tenant_id: TenantId,
    /// Platform user UUID (as a string) recorded on the queue row, or `None` —
    /// callers pass `session.user_id`/the push's `Uuid`; the column feeds
    /// analytics `distinct_id`, so it must be the platform id, never the
    /// channel-native one.
    pub user_id: Option<&'a str>,
    /// Channel slug (e.g. `"whatsapp"`) for the message row + queue row.
    pub channel: &'a str,
    /// Instant after which the message must not be re-sent, because a link it
    /// carries has expired by then. `None` when nothing in it expires.
    pub expires_at: Option<DateTime<Utc>>,
    /// Set on a reconnect notice: the provider connection the notice is about.
    /// The worker re-sends only while that connection is still `needs_reauth`.
    pub reauth: Option<OutboundReauthGuard<'a>>,
}

/// Render, persist, and enqueue a failed outbound message for retry delivery.
///
/// The single source of truth for the "a channel send failed — don't drop it"
/// path, shared by the synchronous reply (`dispatch::send_outbound_response`),
/// the backfill-completion push (`ServerBackfillNotifier::push_backfill_complete`)
/// and the reconnect notices. Renders the outgoing message to the channel's
/// native payload, persists the outbound message row when the send belongs to a
/// session (the FK target for the queue entry), then enqueues it so the
/// background retry worker re-sends with backoff and dead-letters after the
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

    let message_id = match &params.transcript {
        Some(transcript) => Some(
            record_outbound_message(db, outgoing, transcript, params.channel, &payload_str).await?,
        ),
        None => None,
    };

    let queue_id = Uuid::new_v4().to_string();
    db.enqueue_outbound(&EnqueueOutboundParams {
        id: &queue_id,
        message_id: message_id.as_deref(),
        tenant_id: params.queue_tenant_id,
        user_id: params.user_id,
        channel_type: params.channel,
        payload: &payload_str,
        expires_at: params.expires_at,
        reauth: params.reauth,
    })
    .await?;

    info!(
        queue_id = %queue_id,
        channel = %params.channel,
        "Outbound message enqueued for retry"
    );
    Ok(())
}

/// Persist the outbound message row a queue entry re-sends (the FK target for
/// the queue entry), and return its id.
async fn record_outbound_message(
    db: &dyn MessagingRepository,
    outgoing: &OutgoingMessage,
    transcript: &TranscriptRecord<'_>,
    channel: &str,
    payload_str: &str,
) -> Result<String, AppError> {
    // Use a unique retry-prefixed ID to avoid colliding with the (tenant_id, channel_message_id)
    // uniqueness constraint — retry messages have no real channel ID yet.
    let out_msg_id = Uuid::new_v4().to_string();
    let retry_channel_msg_id = format!("retry-{out_msg_id}");
    let body = content_body_text(&outgoing.content);
    let correlation_str = outgoing.turn_id.to_string();
    let out_params = InsertMessageParams {
        id: &out_msg_id,
        // Message row shares the session/user tenant; the queue row stays on
        // queue_tenant_id so the retry worker loads the bot's channel config to
        // re-send. The queue->message FK is single-column (message_id), so the
        // differing tenants don't break it.
        tenant_id: transcript.tenant_id,
        session_id: transcript.session_id,
        direction: "outbound",
        channel_type: channel,
        channel_message_id: &retry_channel_msg_id,
        sender_id: "pierre",
        content_type: super::content_type_label(&outgoing.content),
        content_body: body.as_deref(),
        correlation_id: &correlation_str,
        raw_payload: Some(payload_str),
        // The send failed before the channel assigned an id, so this row is
        // keyed on the synthetic `retry-…` id above. No inbound reaction can
        // quote that id, so there is nothing for a rating to resolve through.
        chat_message_id: None,
    };
    let inserted = db.insert_message(&out_params).await?;
    if !inserted {
        return Err(AppError::internal(
            "Failed to persist retry message: duplicate channel_message_id",
        ));
    }
    Ok(out_msg_id)
}
