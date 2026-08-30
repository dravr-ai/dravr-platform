// ABOUTME: Decides what a shared room is left seeing after a slash command is answered privately
// ABOUTME: Deletes the command echo where the channel allows it, and says where the answer went when it cannot

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![cfg(feature = "client-messaging")]

use std::sync::Arc;

use pierre_contremaitre::messaging_strings::{DEFAULT_LOCALE, KEY_SLASH_ANSWERED_PRIVATELY};
use pierre_core::errors::messaging::MessagingError;
use pierre_core::models::messaging::{ChannelType, MessageContent, OutgoingMessage};
use pierre_core::models::TenantId;
use pierre_database::backends::MessagingRepository;
use pierre_messaging::channel::MessagingChannel;
use pierre_messaging::turn::ConversationTurnId as CanotTurnId;
use tracing::{debug, info};
use uuid::Uuid;

use crate::mcp::resources::ServerContext;

use super::dispatch::load_channel_config;
use super::locale::resolve_messaging_locale;

/// Best-effort removal of a user's slash-command echo from a shared room.
///
/// When a slash command arrives in a shared room the reply is delivered
/// privately to the caller (see `slash_reply_should_be_private`). This deletes
/// the original command message so the room never shows it. Failures (bot not
/// an admin with delete rights, message already gone, channel can't delete)
/// never affect the turn — the command was still handled and answered privately.
///
/// The outcome decides what the room is left looking at, so it is reported
/// rather than swallowed. `None` means the echo is gone: the room shows nothing
/// and needs nothing. `Some(reason)` means it is still on screen — the room
/// shows a command with no answer, the caller is owed a word about where the
/// answer went (see [`KEY_SLASH_ANSWERED_PRIVATELY`]), and `reason` says why it
/// survived, for the operator log.
async fn delete_room_command_echo(
    db: &dyn MessagingRepository,
    tenant_id: TenantId,
    channel: &str,
    adapter: &Arc<dyn MessagingChannel>,
    room_id: &str,
    channel_message_id: &str,
) -> Option<String> {
    let Some(config) = load_channel_config(db, tenant_id, channel).await else {
        return Some("channel config unavailable".to_owned());
    };
    if let Err(e) = adapter
        .delete_message(room_id, channel_message_id, &config)
        .await
    {
        debug!(
            channel = %channel,
            error = %e,
            "Could not delete slash-command echo from room (bot may lack admin rights)"
        );
        return Some(deletion_failure_label(&e));
    }
    None
}

/// A bounded, secret-free description of why a deletion did not happen.
///
/// The error itself is not safe to log above debug: a transport failure renders
/// the request URL, and a Telegram URL carries the bot token in its path. This
/// maps each failure to a fixed phrase, plus the HTTP status where the channel
/// supplied one — enough to tell "the bot lacks admin rights" from "this
/// channel cannot delete at all" without putting a credential in the log.
fn deletion_failure_label(error: &MessagingError) -> String {
    match error {
        MessagingError::OperationNotSupported { .. } => {
            "channel has no message-deletion API".to_owned()
        }
        MessagingError::ChannelApiError { status_code, .. } => {
            format!("channel API refused it (HTTP {status_code})")
        }
        MessagingError::ChannelNotConfigured { .. } => "channel not configured".to_owned(),
        _ => "delete request failed".to_owned(),
    }
}

/// Everything [`settle_room_echo`] needs to decide what a room is left seeing.
pub struct RoomEchoSettlement<'a> {
    /// Server context: strings registry and locale resolution.
    pub resources: &'a ServerContext,
    /// Messaging repository, for the channel config the delete needs.
    pub db: &'a dyn MessagingRepository,
    /// Tenant owning the channel config.
    pub tenant_id: TenantId,
    /// Channel slug (`"telegram"`, `"slack"`, …).
    pub channel: &'a str,
    /// Strongly-typed channel kind, for addressing the notice.
    pub channel_type: ChannelType,
    /// Adapter that performs the delete.
    pub adapter: &'a Arc<dyn MessagingChannel>,
    /// The shared room the command arrived in.
    pub room_id: &'a str,
    /// The command message to remove.
    pub channel_message_id: &'a str,
    /// Pierre user id of the caller, for their locale.
    pub user_id: &'a str,
    /// Channel-native id of the caller, for their locale.
    pub sender_id: &'a str,
}

/// Remove the command echo, and say where the answer went if it could not be.
///
/// Returns the room-visible notice to send, or `None` when the echo was
/// deleted and the room, showing nothing at all, needs nothing.
pub async fn settle_room_echo(args: RoomEchoSettlement<'_>) -> Option<OutgoingMessage> {
    let RoomEchoSettlement {
        resources,
        db,
        tenant_id,
        channel,
        channel_type,
        adapter,
        room_id,
        channel_message_id,
        user_id,
        sender_id,
    } = args;

    let survived =
        delete_room_command_echo(db, tenant_id, channel, adapter, room_id, channel_message_id)
            .await;

    // The one line that says what the room is now looking at. Without it the
    // whole settlement is invisible in production — the delete logs only at
    // debug and the notice send is fire-and-forget, so a room reported as
    // silent could not be told apart from one that was correctly answered.
    info!(
        channel = %channel,
        room_id = %room_id,
        echo_deleted = survived.is_none(),
        reason = survived.as_deref().unwrap_or("deleted"),
        "Settled slash-command echo in shared room"
    );

    // Still on screen: the room shows a command with no answer under it, so it
    // is owed the one line saying where the answer went.
    if survived.is_some() {
        return Some(
            answered_privately_notice(
                resources,
                tenant_id,
                channel_type,
                channel,
                user_id,
                sender_id,
                room_id,
            )
            .await,
        );
    }

    None
}

/// A room-visible note that the answer went to the caller's direct chat.
///
/// Carries no part of the answer — a room is bound to a group that can hold
/// several athletes, which is the whole reason the reply was redirected. It says
/// only where to look.
///
/// Sent only when the command echo could not be deleted. With the echo gone the
/// room shows nothing and is owed nothing; with it still there the room shows a
/// member's command and no response, which reads as the bot ignoring them in
/// front of everyone. That is what a `/plan` in a group looked like on
/// 2026-08-29 — twice, while both answers sat delivered in the callers' DMs.
///
/// Addressed to the room, in the caller's locale: they are the one who needs to
/// know where their answer went.
pub async fn answered_privately_notice(
    resources: &ServerContext,
    tenant_id: TenantId,
    channel_type: ChannelType,
    channel: &str,
    user_id: &str,
    sender_id: &str,
    room_id: &str,
) -> OutgoingMessage {
    let locale = match Uuid::parse_str(user_id) {
        Ok(uuid) => resolve_messaging_locale(resources, tenant_id, uuid, channel, sender_id).await,
        Err(_) => DEFAULT_LOCALE.to_owned(),
    };
    let body = resources
        .mcp
        .messaging_strings_registry
        .get(KEY_SLASH_ANSWERED_PRIVATELY, &locale);

    OutgoingMessage {
        channel_type,
        recipient_id: room_id.to_owned(),
        content: MessageContent::Text { body },
        turn_id: CanotTurnId::new(),
        reply_to: None,
        thread_id: None,
    }
}
