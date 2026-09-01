// ABOUTME: Writes the messaging_messages outbound row for a delivered or failed channel send
// ABOUTME: The single insert path shared by coaching dispatch, AG-UI finalize, and slash egress

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use pierre_core::models::messaging::{IncomingMessage, OutgoingMessage};
use pierre_core::models::TenantId;
use pierre_database::backends::{InsertMessageParams, MessagingRepository};
use tracing::{debug, error};
use uuid::Uuid;

use super::outbound_send::OutboundPersistSpec;
use super::{content_body_text, content_type_label};
use crate::mcp::resources::ServerContext;

/// The identifiers an outbound ledger row carries beyond the message itself.
pub struct OutboundRowParams<'a> {
    /// Tenant that owns the session/conversation (the user's own tenant for a
    /// DM, the channel tenant for a shared room) — the tenant the whole turn
    /// reads under, matching the inbound row's filing.
    pub session_tenant_id: TenantId,
    /// The `messaging_sessions` row this send belongs to.
    pub session_id: &'a str,
    /// Channel slug (`"slack"`, `"telegram"`, …).
    pub channel: &'a str,
    /// The channel's delivery-receipt message id; `None` or empty when the
    /// channel returned none, or when the send failed.
    pub receipt_id: Option<&'a str>,
    /// Whether the channel accepted the send. A failed send still gets a row
    /// — keyed `failed-…` — so the attempt is visible in the ledger.
    pub delivered: bool,
    /// The assistant `chat_messages` row this send delivers, when the
    /// transcript policy persisted one — the join an emoji reaction's rating
    /// resolves through.
    pub chat_message_id: Option<&'a str>,
}

/// Insert one outbound `messaging_messages` row, best-effort.
///
/// A channel that returns no message id still gets a unique key: an empty
/// `channel_message_id` would collide on the `(tenant_id, channel_message_id)`
/// uniqueness index after the first such row and be dropped by the idempotent
/// insert. `sent-…` / `failed-…` are siblings of `outbound_retry`'s `retry-…`
/// idiom; no inbound reaction can quote a synthetic id, so nothing resolves to
/// those rows for rating.
pub(super) async fn persist_outbound_row(
    db: &dyn MessagingRepository,
    params: &OutboundRowParams<'_>,
    outgoing: &OutgoingMessage,
) {
    let row_id = Uuid::new_v4().to_string();
    let synthetic;
    let channel_message_id = match params.receipt_id {
        Some(id) if !id.is_empty() => id,
        _ => {
            synthetic = if params.delivered {
                format!("sent-{row_id}")
            } else {
                format!("failed-{row_id}")
            };
            synthetic.as_str()
        }
    };
    let body = content_body_text(&outgoing.content);
    let correlation_str = outgoing.turn_id.to_string();
    let out_params = InsertMessageParams {
        id: &row_id,
        tenant_id: params.session_tenant_id,
        session_id: params.session_id,
        direction: "outbound",
        channel_type: params.channel,
        channel_message_id,
        sender_id: "pierre",
        content_type: content_type_label(&outgoing.content),
        content_body: body.as_deref(),
        correlation_id: &correlation_str,
        raw_payload: None,
        chat_message_id: params.chat_message_id,
    };
    match db.insert_message(&out_params).await {
        Ok(true) => {}
        Ok(false) => debug!(
            channel_message_id = %channel_message_id,
            "Outbound row already present; idempotent insert skipped the duplicate"
        ),
        Err(e) => error!(error = %e, "Failed to persist outbound message"),
    }
}

/// The outbound-ledger spec for a pre-dispatch reply to `message`'s sender,
/// when their session resolves — the same tenant walk the live dispatch does:
/// the link authenticates under the channel tenant, a DM session lives under
/// the user's own tenant, a room session under the channel's.
///
/// `None` when any step fails to resolve (an unlinked sender, no session yet):
/// the reply is still sent, merely unledgered — exactly the pre-existing
/// behaviour for a sender the ledger has nothing to attach to.
pub(super) async fn logout_persist_spec(
    resources: &ServerContext,
    tenant_id: TenantId,
    channel: &str,
    message: &IncomingMessage,
) -> Option<OutboundPersistSpec> {
    let auth = resources
        .auth
        .auth_middleware
        .authenticate_channel(tenant_id, channel, &message.sender_id)
        .await
        .ok()?;
    let user_tenant = auth.active_tenant_id.map_or(tenant_id, TenantId::from_uuid);
    let session_tenant = if message.is_direct_message {
        user_tenant
    } else {
        tenant_id
    };
    let session = resources
        .common
        .repos
        .messaging
        .get_session_by_channel_identity(
            session_tenant,
            channel,
            &message.sender_id,
            message.conversation_id.as_deref(),
        )
        .await
        .ok()??;
    let session_id = session["id"].as_str()?.to_owned();
    Some(OutboundPersistSpec {
        db: Arc::clone(&resources.common.repos.messaging),
        session_tenant_id: session_tenant,
        session_id,
        chat_message_id: None,
    })
}
