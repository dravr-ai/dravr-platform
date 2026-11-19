// ABOUTME: Re-delivers in the app a proactive notice WhatsApp took and then failed for a closed 24-hour window (131047)
// ABOUTME: A status webhook names the failed message by its wamid; the notice ledger leads back to the athlete and thread
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! What a `WhatsApp` status webhook carrying error 131047 does (carnet#1672).
//!
//! Meta takes a free-form send outside the athlete's 24-hour window with HTTP
//! 200 and fails it afterwards, in a status webhook that names only the
//! message's wamid and its recipient. The platform's proactive notices check
//! the window before they send
//! ([`ResolvedRoute::window_open`](crate::services::backfill_delivery::ResolvedRoute::window_open)),
//! so what reaches this is the edge that check cannot see: the window closing
//! between the check and Meta's own. A notice that failed this way is written
//! into the conversation it was sent from, which the app reads, and
//! `messaging.proactive_held` reports it. Re-sending it on the channel would
//! fail the same way, so nothing is queued.
//!
//! The way back from a wamid is the notice ledger
//! ([`NoticeLedger`](crate::services::backfill_delivery::NoticeLedger)): one
//! outbound row per message a proactive notice sent, stamped with its
//! purpose. A reply has no such row — its athlete had just written, and its
//! text is already the conversation's assistant turn — so it, and every
//! failure with another code, stays the warning the webhook logs.
//!
//! Meta can deliver one status more than once, and two deliveries can race
//! across instances, so the re-delivery is claimed first, once per notice, in
//! the durable once-claim log the backfill push already uses
//! ([`MessagingRepository::claim_backfill_push`]). Only the delivery that wins
//! the claim writes the in-app turn and reports the hold.

use pierre_core::models::messaging::ChannelType;
use pierre_core::models::TenantId;
use pierre_core::transport::TransportPolicy;
use pierre_database::backends::MessagingRepository;
use pierre_database::repositories::ProactiveNotice;
use tracing::{debug, info, warn};
use uuid::Uuid;

use super::connect::resolve_user_and_tenant;
use crate::mcp::resources::ServerContext;
use crate::services::backfill_delivery::InAppDelivery;
use crate::services::post_delivery::report_held;

/// Meta's error for a free-form `WhatsApp` message sent more than 24 hours
/// after the recipient last wrote: a template would be required.
pub(crate) const WHATSAPP_REENGAGEMENT_REQUIRED: i64 = 131_047;

/// Why `messaging.proactive_held` reports a notice the channel took and then
/// failed for its closed window.
const HELD_WINDOW_CLOSED_ON_DELIVERY: &str = "window_closed_on_delivery";

/// The once-claim key prefix of a notice's re-delivery; the opening ledger
/// row's id completes it.
const REDELIVERY_CLAIM: &str = "window_failure";

/// One message a `WhatsApp` status webhook reported failed with
/// [`WHATSAPP_REENGAGEMENT_REQUIRED`].
#[derive(Debug, Clone, Copy)]
pub(crate) struct WindowFailure<'status> {
    /// The recipient's `WhatsApp` id, a phone number: never logged.
    pub recipient_id: &'status str,
    /// The failed message's wamid.
    pub message_id: &'status str,
}

/// Re-deliver in the app the proactive notice `failure` was a message of,
/// for a status webhook verified as `channel_tenant_id`'s bot.
///
/// The recipient resolves through its channel link to the athlete and their
/// own tenant, where a DM's rows are filed; a room's are filed under the bot
/// tenant, which is looked in next. The link alone decides it: the
/// user-status and request-budget gates of `authenticate_channel` refuse a
/// turn the athlete takes, and this is a notice the platform already owes
/// them. The notice is handled once, on its opening message: a closed window
/// fails every message of a notice, and each failure arrives on its own; and
/// by the one delivery of that status that wins its claim. Its text is
/// written into the conversation unless the conversation already holds it —
/// an agent answer the platform post stored. Logged and left alone when the
/// recipient is no linked athlete, the message was not a proactive notice's,
/// or another delivery already handled it.
pub(crate) async fn redeliver_in_app(
    resources: &ServerContext,
    channel_tenant_id: TenantId,
    failure: WindowFailure<'_>,
) {
    let Some((tenant_id, notice)) = failed_notice(resources, channel_tenant_id, failure).await
    else {
        return;
    };
    if !claim_redelivery(resources, tenant_id, &notice).await {
        return;
    }
    if notice
        .parts
        .iter()
        .any(|part| part.chat_message_id.is_some())
    {
        info!(
            purpose = %notice.purpose,
            "WhatsApp window failure: the notice is already in the conversation the app reads"
        );
    } else {
        write_in_app(resources, tenant_id, &notice).await;
    }
    report_held(
        tenant_id,
        ChannelType::WhatsApp,
        &notice.purpose,
        HELD_WINDOW_CLOSED_ON_DELIVERY,
    );
}

/// The proactive notice `failure` opened, sent to the athlete its recipient
/// is linked to, and the tenant that files it; `None` (logged) for anything
/// else.
async fn failed_notice(
    resources: &ServerContext,
    channel_tenant_id: TenantId,
    failure: WindowFailure<'_>,
) -> Option<(TenantId, ProactiveNotice)> {
    let channel = ChannelType::WhatsApp.to_string();
    // The recipient is a phone number: the line names only the message.
    let Some((user_id, user_tenant)) =
        resolve_user_and_tenant(resources, channel_tenant_id, &channel, failure.recipient_id).await
    else {
        debug!(
            message_id = %failure.message_id,
            "WhatsApp window failure: the recipient resolves to no linked athlete; left as logged"
        );
        return None;
    };
    let Some((tenant_id, notice)) = find_notice(
        resources,
        [TenantId::from_uuid(user_tenant), channel_tenant_id],
        &channel,
        failure.message_id,
    )
    .await
    else {
        debug!(
            message_id = %failure.message_id,
            "WhatsApp window failure: not a proactive notice's message; left as logged"
        );
        return None;
    };
    opened_by(notice, failure, user_id).map(|notice| (tenant_id, notice))
}

/// `notice`, when it was sent to `recipient` and `failure` is its opening
/// message; `None` (logged) otherwise.
fn opened_by(
    notice: ProactiveNotice,
    failure: WindowFailure<'_>,
    recipient: Uuid,
) -> Option<ProactiveNotice> {
    if notice.user_id != recipient.to_string() {
        warn!(
            message_id = %failure.message_id,
            "WhatsApp window failure: the notice's session is not the recipient's; left as logged"
        );
        return None;
    }
    if notice.parts.first()?.channel_message_id != failure.message_id {
        debug!(
            message_id = %failure.message_id,
            "WhatsApp window failure: a later message of a notice; handled with its opening one"
        );
        return None;
    }
    Some(notice)
}

/// Claim the one re-delivery of `notice` across deliveries and instances,
/// keyed by its opening ledger row. `false` (logged) when another delivery
/// holds it, or the claim cannot be written: a second in-app copy is worse
/// than none, and the notice stays in the ledger either way. The claim is
/// never released, so a winner whose write then fails leaves no copy and no
/// later delivery retries it.
async fn claim_redelivery(
    resources: &ServerContext,
    tenant_id: TenantId,
    notice: &ProactiveNotice,
) -> bool {
    let Some(opening) = notice.parts.first() else {
        return false;
    };
    let key = format!("{REDELIVERY_CLAIM}:{}", opening.id);
    let messaging: &dyn MessagingRepository = resources.common.repos.messaging.as_ref();
    match messaging
        .claim_backfill_push(tenant_id, &notice.user_id, &key, 0)
        .await
    {
        Ok(true) => true,
        Ok(false) => {
            debug!(
                purpose = %notice.purpose,
                "WhatsApp window failure: another delivery of this status already handled the notice"
            );
            false
        }
        Err(e) => {
            warn!(error = %e, "WhatsApp window failure: the re-delivery claim failed; nothing written");
            false
        }
    }
}

/// The notice whose message is `message_id`, in the first of `tenants` that
/// files it, and that tenant.
async fn find_notice(
    resources: &ServerContext,
    tenants: [TenantId; 2],
    channel: &str,
    message_id: &str,
) -> Option<(TenantId, ProactiveNotice)> {
    let messaging = resources.common.repos.messaging.as_ref();
    let [first, second] = tenants;
    let candidates = if first == second {
        vec![first]
    } else {
        vec![first, second]
    };
    for tenant_id in candidates {
        match messaging
            .find_proactive_notice(tenant_id, channel, message_id)
            .await
        {
            Ok(Some(notice)) => return Some((tenant_id, notice)),
            Ok(None) => {}
            Err(e) => {
                warn!(error = %e, "WhatsApp window failure: the notice ledger could not be read");
                return None;
            }
        }
    }
    None
}

/// Write the notice's text into its conversation as one assistant turn, and
/// name that turn on its opening ledger row, where the ledger records the
/// chat row a notice is stored as.
///
/// Stamped first-party only: the ledger keeps no stamp, and a notice can list
/// data whose provider allows it on Dravr's own surfaces alone — the
/// messaging channel it was written for is one of them. Stored without an
/// app push; the turn's unread badge is the ping.
async fn write_in_app(resources: &ServerContext, tenant_id: TenantId, notice: &ProactiveNotice) {
    let (Some(conversation_id), Ok(user_id), Some(opening)) = (
        notice.conversation_id.as_deref(),
        Uuid::parse_str(&notice.user_id),
        notice.parts.first(),
    ) else {
        warn!(
            purpose = %notice.purpose,
            "WhatsApp window failure: the notice's session names no conversation or user to write to"
        );
        return;
    };
    let Some(body) = notice_text(notice) else {
        info!(
            purpose = %notice.purpose,
            "WhatsApp window failure: the notice carried no text to write in the app"
        );
        return;
    };
    let repos = &resources.common.repos;
    let Some(stored) = (InAppDelivery {
        repos,
        strings: &resources.mcp.messaging_strings_registry,
        #[cfg(feature = "client-notifications")]
        notifications: resources.common.notification_service.as_ref(),
        transport_policy: TransportPolicy::FirstPartyOnly,
    })
    .persist(user_id, tenant_id, conversation_id, body, 0)
    .await
    else {
        return;
    };
    if let Err(e) = repos
        .messaging
        .stamp_notice_chat_message(tenant_id, &opening.id, &stored)
        .await
    {
        warn!(error = %e, "WhatsApp window failure: the in-app copy could not be named on the ledger");
    }
}

/// The notice's text, its messages' in send order and a blank line apart;
/// `None` when no message carried any (attachments with no caption).
fn notice_text(notice: &ProactiveNotice) -> Option<String> {
    let texts: Vec<&str> = notice
        .parts
        .iter()
        .filter_map(|part| part.content_body.as_deref())
        .filter(|text| !text.trim().is_empty())
        .collect();
    (!texts.is_empty()).then(|| texts.join("\n\n"))
}
