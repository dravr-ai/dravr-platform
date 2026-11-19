// ABOUTME: The channel identity an inbound message is routed on: placeholder ids refused, WhatsApp re-keyed to the BSUID
// ABOUTME: Moves an athlete linked by phone number onto their business-scoped user id so the account and conversation follow
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Which channel identity an inbound message belongs to.
//!
//! A channel identity (`channel_type`, `sender_id`) is the key a Pierre
//! account is bound to, so two rules hold before anything routes on it:
//!
//! - A placeholder id that every author-less event carries — the literal
//!   `"unknown"` an old canot parser substituted, or an empty id — is never
//!   routed. The first athlete to link it would read and write every later
//!   such sender's coaching (carnet#1668). The link write refuses it too.
//! - `WhatsApp` names a sender by their business-scoped user id (BSUID) since
//!   Meta's username rollout, where it used the phone number. An athlete
//!   linked by phone number before canot keyed on the BSUID is moved onto it
//!   the first time a message carries both, keeping their account and
//!   conversation instead of being asked to link again.

use dravr_canot::channels::whatsapp::identity::sender_phone_number;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::messaging::{is_routable_sender_id, ChannelType, IncomingMessage};
use pierre_core::models::TenantId;
use pierre_database::backends::{MessagingRepository, RekeyChannelIdentityParams};
use tracing::{info, warn};
use uuid::Uuid;

use crate::mcp::resources::ServerContext;

/// Settle the channel identity `message` is routed on, before anything reads
/// its sender id as one: `false` when the sender id is a placeholder that is
/// never linked or routed; otherwise a `WhatsApp` athlete linked by phone
/// number is first moved onto the BSUID this message names them by, so the
/// lookups after it find them.
pub(super) async fn admit(
    resources: &ServerContext,
    tenant_id: TenantId,
    channel: &str,
    channel_type: ChannelType,
    message: &IncomingMessage,
) -> bool {
    if !is_routable(channel, message) {
        return false;
    }
    adopt_business_scoped_id(resources, tenant_id, channel, channel_type, message).await;
    true
}

/// Whether `message` may be routed: its sender id names one real sender.
///
/// Logs the refusal, naming the channel and message id but not the sender id.
fn is_routable(channel: &str, message: &IncomingMessage) -> bool {
    if is_routable_sender_id(&message.sender_id) {
        return true;
    }
    warn!(
        channel,
        channel_message_id = %message.channel_message_id,
        "Inbound message names a placeholder sender id; refusing to route or link it"
    );
    false
}

/// Move a `WhatsApp` athlete linked under their phone number onto the BSUID
/// this message names them by.
///
/// A no-op unless the message is a `WhatsApp` message whose sender id is not
/// the phone number Meta also sent, the BSUID holds no link yet, and the phone
/// number holds one. Best effort: a failure is logged and the message goes on
/// under its BSUID, where the athlete is offered linking again rather than
/// routed anywhere else.
async fn adopt_business_scoped_id(
    resources: &ServerContext,
    tenant_id: TenantId,
    channel: &str,
    channel_type: ChannelType,
    message: &IncomingMessage,
) {
    if channel_type != ChannelType::WhatsApp {
        return;
    }
    let Some(phone_number) = sender_phone_number(message) else {
        return;
    };
    if phone_number == message.sender_id {
        return;
    }
    if let Err(e) = rekey_phone_link(
        resources,
        tenant_id,
        channel,
        phone_number,
        &message.sender_id,
    )
    .await
    {
        warn!(
            channel,
            error = %e,
            "Could not move a phone-number link onto the sender's business-scoped id"
        );
    }
}

/// Re-key the link a phone number holds, with its sessions and recorded turns,
/// onto `business_scoped_id`.
async fn rekey_phone_link(
    resources: &ServerContext,
    tenant_id: TenantId,
    channel: &str,
    phone_number: &str,
    business_scoped_id: &str,
) -> AppResult<()> {
    let db: &dyn MessagingRepository = resources.common.repos.messaging.as_ref();

    // The steady state once an athlete has been moved, so it is checked first:
    // the BSUID already holds the link and there is nothing to do.
    if db
        .get_channel_link(tenant_id, channel, business_scoped_id)
        .await?
        .is_some()
    {
        return Ok(());
    }
    let Some(link) = db
        .get_channel_link(tenant_id, channel, phone_number)
        .await?
    else {
        return Ok(());
    };
    let user_id = link["user_id"]
        .as_str()
        .ok_or_else(|| AppError::internal("Channel link missing user_id"))?;
    let user_uuid = Uuid::parse_str(user_id)
        .map_err(|_| AppError::internal("Channel link user_id is not a UUID"))?;

    // The link lives under the bot tenant the webhook carries; a DM session
    // lives under the athlete's own tenant, the one channel auth resolves.
    let session_tenant_id = resources
        .common
        .repos
        .tenants
        .first_for_user(user_uuid)
        .await?
        .unwrap_or(tenant_id);

    let moved = db
        .rekey_channel_identity(&RekeyChannelIdentityParams {
            link_tenant_id: tenant_id,
            session_tenant_id,
            user_id,
            channel_type: channel,
            from_channel_user_id: phone_number,
            to_channel_user_id: business_scoped_id,
        })
        .await?;
    if moved {
        info!(
            channel,
            user_id, "Moved a phone-number channel link onto the sender's business-scoped id"
        );
    }
    Ok(())
}
