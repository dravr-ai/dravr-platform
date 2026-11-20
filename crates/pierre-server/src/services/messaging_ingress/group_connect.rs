// ABOUTME: A group chat connected to a web coaching group (carnet#1683): redeem the connect code, admit members only
// ABOUTME: `/start <code>` from the add-to-group link or `/group link <code>`; a non-member is told once how to join
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The two moments the messaging ingress meets a chat connected to a group
//! from the web app.
//!
//! **Connecting.** The group's owner or coach mints a code in the web app and
//! either taps the add-to-group link, which makes Telegram deliver
//! `/start <code>` in the chat they pick, or types `/group link <code>` in a
//! chat the bot is already in. [`detect_connect_code`] recognises both in a
//! group chat only, and [`handle_connect_code`] redeems it through
//! [`GroupService::connect_chat`](pierre_groups::GroupService::connect_chat)
//! and answers in the chat.
//!
//! **Taking part.** A connected chat is the group's room for its members
//! only. [`admit_to_connected_room`] runs before anything else touches a
//! message there: a sender who is not a member (or the group's coach),
//! linked or not, gets no session, no stored message and no turn, and is told
//! once how to join. Their words never reach the room, because the room
//! carries peer context they never consented to share in.

use std::sync::Arc;

use chrono::Utc;
use pierre_contremaitre::messaging_strings::{
    DEFAULT_LOCALE, KEY_ERROR_GENERIC, KEY_GROUP_CHAT_ALREADY_CONNECTED, KEY_GROUP_CHAT_CHAT_TAKEN,
    KEY_GROUP_CHAT_CODE_INVALID, KEY_GROUP_CHAT_CONNECTED,
    KEY_GROUP_CHAT_GROUP_CONNECTED_ELSEWHERE, KEY_GROUP_CHAT_GROUP_GONE,
    KEY_GROUP_CHAT_NON_MEMBER_PROMPT, KEY_GROUP_CHAT_NOT_LINKED, KEY_GROUP_CHAT_NOT_MANAGER,
};
use pierre_core::models::groups::{CoachingGroup, GroupRespondMode};
use pierre_core::models::messaging::{
    channel_label, ChannelType, IncomingMessage, MessageContent, OutgoingMessage,
};
use pierre_core::models::TenantId;
use pierre_database::backends::MessagingRepository;
use pierre_groups::chat_connect::{ChatConnectRefusal, ChatConnectResult, ChatToConnect};
use pierre_services::locale::resolve_channel_locale;
use pierre_services::messaging_broadcast::proactive_text;
use tracing::{error, info, warn};
use uuid::Uuid;

use super::addressing::reply_recipient;
use super::content::content_body_text;
use crate::mcp::resources::ServerContext;

/// The connect code a group-chat message carries, when it is one:
/// `/start <code>` (what Telegram delivers after the add-to-group link) or
/// `/group link <code>` (typed by hand in a chat the bot already sits in).
///
/// Only a group chat counts. A direct message's `/start <code>` is a pairing
/// code and keeps going to the channel-linking path. The platforms strip the
/// `@bot` suffix of a command before it gets here.
#[must_use]
pub(super) fn detect_connect_code(message: &IncomingMessage) -> Option<String> {
    if message.is_direct_message {
        return None;
    }
    let MessageContent::Text { body } = &message.content else {
        return None;
    };
    let words: Vec<&str> = body.split_whitespace().collect();
    let code = match words.as_slice() {
        ["/start", code] => code,
        ["/group", verb, code] if verb.eq_ignore_ascii_case("link") => code,
        _ => return None,
    };
    code.chars()
        .all(|c| c.is_ascii_alphanumeric())
        .then(|| (*code).to_owned())
}

/// The sender's Dravr user, when their chat identity is linked under the bot
/// tenant. A lookup failure reads as unlinked, and is logged.
async fn linked_user(
    resources: &ServerContext,
    tenant_id: TenantId,
    channel: &str,
    sender_id: &str,
) -> Option<Uuid> {
    let db: &dyn MessagingRepository = resources.common.repos.messaging.as_ref();
    match db.get_channel_link(tenant_id, channel, sender_id).await {
        Ok(Some(link)) => link["user_id"]
            .as_str()
            .and_then(|id| Uuid::parse_str(id).ok()),
        Ok(None) => None,
        Err(e) => {
            warn!(error = %e, channel, "connected room: channel link lookup failed");
            None
        }
    }
}

/// A text reply into the chat the message came from.
fn chat_reply(
    message: &IncomingMessage,
    channel_type: ChannelType,
    body: String,
) -> OutgoingMessage {
    let recipient = reply_recipient(message.conversation_id.as_deref(), &message.sender_id);
    let mut reply = proactive_text(channel_type, recipient.to_owned(), body);
    reply.thread_id = message
        .metadata
        .get("message_thread_id")
        .and_then(serde_json::Value::as_i64)
        .map(|id| id.to_string());
    reply
}

/// Redeem a connect code sent in a group chat, and the reply to post there.
pub(super) async fn handle_connect_code(
    resources: &Arc<ServerContext>,
    tenant_id: TenantId,
    channel: &str,
    channel_type: ChannelType,
    message: &IncomingMessage,
    code: &str,
) -> OutgoingMessage {
    let registry = &resources.mcp.messaging_strings_registry;
    let Some(chat_id) = message.conversation_id.as_deref().filter(|c| !c.is_empty()) else {
        let body = registry.render(KEY_GROUP_CHAT_CODE_INVALID, DEFAULT_LOCALE, &[]);
        return chat_reply(message, channel_type, body);
    };
    let Some(sender) = linked_user(resources, tenant_id, channel, &message.sender_id).await else {
        let body = registry.render(
            KEY_GROUP_CHAT_NOT_LINKED,
            DEFAULT_LOCALE,
            &[channel_label(channel)],
        );
        return chat_reply(message, channel_type, body);
    };
    let locale = resolve_channel_locale(
        resources.common.repos.messaging.as_ref(),
        resources.common.repos.users.as_ref(),
        tenant_id,
        channel,
        &message.sender_id,
        Some(sender),
    )
    .await;

    let result = resources
        .group_service()
        .connect_chat(&ChatToConnect {
            code,
            channel_type: channel,
            channel_chat_id: chat_id,
            channel_tenant_id: tenant_id,
            chat_title: message.chat_title.as_deref(),
            sender,
        })
        .await;
    let body = match result {
        Ok(ChatConnectResult::Connected { group, moved_from }) => {
            info!(
                group_id = %group.id,
                channel,
                moved_from = ?moved_from,
                "Group chat connected from its connect code"
            );
            registry.render(KEY_GROUP_CHAT_CONNECTED, &locale, &[&group.name])
        }
        Ok(ChatConnectResult::AlreadyConnected { group }) => {
            registry.render(KEY_GROUP_CHAT_ALREADY_CONNECTED, &locale, &[&group.name])
        }
        Ok(ChatConnectResult::Refused(refusal)) => {
            info!(channel, refusal = ?refusal, "Group chat connect refused");
            match refusal {
                ChatConnectRefusal::CodeInvalid => {
                    registry.render(KEY_GROUP_CHAT_CODE_INVALID, &locale, &[])
                }
                ChatConnectRefusal::NotManager => {
                    registry.render(KEY_GROUP_CHAT_NOT_MANAGER, &locale, &[])
                }
                ChatConnectRefusal::GroupConnectedElsewhere { group_name } => registry.render(
                    KEY_GROUP_CHAT_GROUP_CONNECTED_ELSEWHERE,
                    &locale,
                    &[&group_name],
                ),
                ChatConnectRefusal::ChatTakenByOtherGroup => {
                    registry.render(KEY_GROUP_CHAT_CHAT_TAKEN, &locale, &[])
                }
                ChatConnectRefusal::GroupGone => {
                    registry.render(KEY_GROUP_CHAT_GROUP_GONE, &locale, &[])
                }
            }
        }
        Err(e) => {
            error!(error = %e, channel, "Group chat connect failed");
            registry.render(KEY_ERROR_GENERIC, &locale, &[])
        }
    };
    chat_reply(message, channel_type, body)
}

/// What [`admit_to_connected_room`] decided about one message.
#[derive(Debug)]
pub(super) enum RoomAdmission {
    /// The chat is not connected to a group from the web app (or the message
    /// is a direct one): the usual rules apply.
    NotConnected,
    /// The sender takes part in the connected chat's group.
    Admitted,
    /// The sender does not: the message goes no further and is not stored.
    /// Carries the one-time note on how to join, the first time they are
    /// refused while asking for an answer.
    Refused(Option<Box<OutgoingMessage>>),
}

/// Whether the message may go on into a chat connected to a group from the
/// web app.
///
/// A lookup failure lets the message through to the usual path, which binds
/// a sender it cannot place to no group, so a transient error costs nobody's
/// privacy.
pub(super) async fn admit_to_connected_room(
    resources: &Arc<ServerContext>,
    tenant_id: TenantId,
    channel: &str,
    channel_type: ChannelType,
    message: &IncomingMessage,
) -> RoomAdmission {
    if message.is_direct_message {
        return RoomAdmission::NotConnected;
    }
    let Some(chat_id) = message.conversation_id.as_deref().filter(|c| !c.is_empty()) else {
        return RoomAdmission::NotConnected;
    };
    let group = match resources
        .group_service()
        .get_group_by_channel(tenant_id, channel, chat_id)
        .await
    {
        Ok(Some(group)) if group.chat_connection.is_some() => group,
        Ok(_) => return RoomAdmission::NotConnected,
        Err(e) => {
            warn!(error = %e, channel, "connected room: group lookup failed; usual path");
            return RoomAdmission::NotConnected;
        }
    };
    let sender = linked_user(resources, tenant_id, channel, &message.sender_id).await;
    if let Some(user_id) = sender {
        match resources
            .group_service()
            .admits_to_connected_chat(&group, user_id)
            .await
        {
            Ok(true) => return RoomAdmission::Admitted,
            Ok(false) => {}
            Err(e) => {
                // Refused rather than let through: the usual path would ask
                // the same membership question of the same database.
                warn!(error = %e, group_id = %group.id, "connected room: membership lookup failed");
                return RoomAdmission::Refused(None);
            }
        }
    }
    RoomAdmission::Refused(
        join_note_once(
            resources,
            tenant_id,
            channel,
            channel_type,
            message,
            &group,
            sender,
        )
        .await
        .map(Box::new),
    )
}

/// The note telling a non-member how to join, the first time they are
/// refused while asking for an answer; `None` after that, and for ambient
/// chatter in a room that answers mentions only, which asked nothing.
async fn join_note_once(
    resources: &Arc<ServerContext>,
    tenant_id: TenantId,
    channel: &str,
    channel_type: ChannelType,
    message: &IncomingMessage,
    group: &CoachingGroup,
    sender: Option<Uuid>,
) -> Option<OutgoingMessage> {
    let asked = message.addressed_to_bot
        || group.respond_mode == GroupRespondMode::All
        || content_body_text(&message.content).is_some_and(|t| t.trim_start().starts_with('/'));
    if !asked {
        return None;
    }
    match resources
        .common
        .repos
        .groups
        .record_chat_invite_prompt(group.id, channel, &message.sender_id, Utc::now())
        .await
    {
        Ok(true) => {}
        Ok(false) => return None,
        Err(e) => {
            warn!(error = %e, group_id = %group.id, "connected room: could not record the join note; not sent");
            return None;
        }
    }
    let locale = match sender {
        Some(user_id) => {
            resolve_channel_locale(
                resources.common.repos.messaging.as_ref(),
                resources.common.repos.users.as_ref(),
                tenant_id,
                channel,
                &message.sender_id,
                Some(user_id),
            )
            .await
        }
        None => DEFAULT_LOCALE.to_owned(),
    };
    let body = resources.mcp.messaging_strings_registry.render(
        KEY_GROUP_CHAT_NON_MEMBER_PROMPT,
        &locale,
        &[&group.name, channel_label(channel)],
    );
    info!(group_id = %group.id, channel, "connected room: told a non-member how to join");
    Some(chat_reply(message, channel_type, body))
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use dravr_canot::turn::ConversationTurnId;

    use super::*;

    fn message(text: &str, is_direct_message: bool) -> IncomingMessage {
        IncomingMessage {
            channel_type: ChannelType::Telegram,
            sender_id: "42".to_owned(),
            sender_name: None,
            content: MessageContent::Text {
                body: text.to_owned(),
            },
            conversation_id: Some("-100777".to_owned()),
            chat_title: Some("Les Rouleurs".to_owned()),
            channel_message_id: "1".to_owned(),
            timestamp: Utc::now(),
            raw_payload: serde_json::Value::Null,
            turn_id: ConversationTurnId::new(),
            is_direct_message,
            addressed_to_bot: true,
            metadata: serde_json::Value::Null,
        }
    }

    #[test]
    fn the_add_to_group_start_carries_a_connect_code() {
        assert_eq!(
            detect_connect_code(&message("/start ABCD2345EFGH", false)).as_deref(),
            Some("ABCD2345EFGH")
        );
    }

    #[test]
    fn group_link_typed_by_hand_carries_a_connect_code() {
        assert_eq!(
            detect_connect_code(&message("/group link ABCD2345EFGH", false)).as_deref(),
            Some("ABCD2345EFGH")
        );
        assert_eq!(
            detect_connect_code(&message("  /group  LINK   ABCD2345EFGH ", false)).as_deref(),
            Some("ABCD2345EFGH"),
            "spacing and the verb's case do not matter"
        );
    }

    #[test]
    fn a_direct_message_start_is_a_pairing_code_not_a_connect_code() {
        assert_eq!(
            detect_connect_code(&message("/start ABCD2345EFGH", true)),
            None
        );
    }

    #[test]
    fn other_group_commands_and_odd_codes_are_not_connect_codes() {
        for text in [
            "/start",
            "/group link",
            "/group join ABCD2345EFGH",
            "/start ABCD 2345",
            "/start ABC-D2345",
            "hello /start ABCD2345EFGH",
        ] {
            assert_eq!(detect_connect_code(&message(text, false)), None, "{text}");
        }
    }
}
