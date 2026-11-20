// ABOUTME: The channel identity an inbound message is routed on: placeholder ids refused, WhatsApp re-keyed to the BSUID
// ABOUTME: Moves an athlete's link onto the id Meta names them by now: their BSUID, or the one a number change regenerates
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
//! - Meta regenerates the BSUID when the athlete changes their phone number,
//!   or on its own, and announces it in a `system` message canot surfaces as
//!   a [`SenderIdentityChange`]. The link moves from the previous BSUID, or
//!   failing that the previous phone number, onto the new id before any
//!   message in the same webhook is routed (carnet#1688).

use dravr_canot::channels::whatsapp::identity::sender_phone_number;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::messaging::{
    is_routable_sender_id, ChannelType, IncomingMessage, SenderIdentityChange,
};
use pierre_core::models::TenantId;
use pierre_database::backends::{MessagingRepository, RekeyChannelIdentityParams};
use tracing::{info, warn};
use uuid::Uuid;

use crate::mcp::resources::ServerContext;

/// Settle the channel identity `message` is routed on, before anything reads
/// its sender id as one: `false` when the sender id names no single sender,
/// which is never linked or routed; otherwise a `WhatsApp` athlete linked by
/// phone number is first moved onto the BSUID this message names them by, so
/// the lookups after it find them.
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
    match move_link(
        resources,
        tenant_id,
        channel,
        phone_number,
        &message.sender_id,
    )
    .await
    {
        Ok(LinkMove::Moved { user_id }) => info!(
            channel,
            user_id, "Moved a phone-number channel link onto the sender's business-scoped id"
        ),
        Ok(LinkMove::AlreadyHeld | LinkMove::NothingLinked) => {}
        Err(e) => warn!(
            channel,
            error = %e,
            "Could not move a phone-number link onto the sender's business-scoped id"
        ),
    }
}

/// Move each athlete a webhook announces a new identifier for onto it, under
/// the bot tenant `tenant_id` the webhook arrived on.
///
/// Applied before the webhook's messages are routed, so a message sent under
/// the new id in the same body finds the link. For each change the link is
/// looked up under the previous business-scoped user id, then under the
/// previous phone number (an athlete linked by phone number before canot
/// keyed on the BSUID); the first that holds one moves. A change whose new id
/// a link already holds moves nothing, and one whose previous ids hold no link
/// leaves the athlete to link again. Best effort: a failure is logged and the
/// webhook goes on.
pub(crate) async fn apply_identity_changes(
    resources: &ServerContext,
    tenant_id: TenantId,
    channel: &str,
    changes: &[SenderIdentityChange],
) {
    for change in changes {
        let Some(to) = change.sender_id().filter(|id| is_routable_sender_id(id)) else {
            warn!(
                channel,
                channel_message_id = %change.channel_message_id,
                "Sender identity change names no routable identifier; ignored"
            );
            continue;
        };
        let outcome = move_onto(resources, tenant_id, channel, change, to).await;
        log_identity_change(channel, change, outcome);
    }
}

/// Log what applying `change` did, naming the user a moved link names but
/// never an identifier.
fn log_identity_change(channel: &str, change: &SenderIdentityChange, outcome: AppResult<LinkMove>) {
    match outcome {
        Ok(outcome) => info!(
            channel,
            kind = ?change.kind,
            outcome = ?outcome,
            "Applied a sender identity change"
        ),
        Err(e) => warn!(
            channel,
            kind = ?change.kind,
            error = %e,
            "Could not move a channel link onto the identifier the platform announced"
        ),
    }
}

/// Move the link the first of `change`'s previous ids that holds one onto
/// `to`.
async fn move_onto(
    resources: &ServerContext,
    tenant_id: TenantId,
    channel: &str,
    change: &SenderIdentityChange,
    to: &str,
) -> AppResult<LinkMove> {
    let previous_ids = change
        .previous_sender_ids()
        .filter(|id| is_routable_sender_id(id));
    for from in previous_ids {
        let outcome = move_link(resources, tenant_id, channel, from, to).await?;
        if outcome != LinkMove::NothingLinked {
            return Ok(outcome);
        }
    }
    Ok(LinkMove::NothingLinked)
}

/// What [`move_link`] found.
#[derive(Debug, Clone, PartialEq, Eq)]
enum LinkMove {
    /// The link naming `user_id` moved onto the new identifier.
    Moved {
        /// The Pierre user the moved link names
        user_id: String,
    },
    /// A link already holds the new identifier, so nothing moved: the athlete
    /// was moved before, or the identifier belongs to another link.
    AlreadyHeld,
    /// No link holds the previous identifier.
    NothingLinked,
}

/// Re-key the link `from` holds, with its sessions and recorded turns, onto
/// `to`.
async fn move_link(
    resources: &ServerContext,
    tenant_id: TenantId,
    channel: &str,
    from: &str,
    to: &str,
) -> AppResult<LinkMove> {
    let db: &dyn MessagingRepository = resources.common.repos.messaging.as_ref();

    // The steady state once an athlete has been moved, so it is checked first:
    // the new identifier already holds a link and there is nothing to do.
    if db.get_channel_link(tenant_id, channel, to).await?.is_some() {
        return Ok(LinkMove::AlreadyHeld);
    }
    let Some(link) = db.get_channel_link(tenant_id, channel, from).await? else {
        return Ok(LinkMove::NothingLinked);
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
            from_channel_user_id: from,
            to_channel_user_id: to,
        })
        .await?;
    // The transaction refuses to move onto an identifier a link holds, so
    // `false` means one took `to` after the check above.
    Ok(if moved {
        LinkMove::Moved {
            user_id: user_id.to_owned(),
        }
    } else {
        LinkMove::AlreadyHeld
    })
}
