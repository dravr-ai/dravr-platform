// ABOUTME: Echoes the label of a tapped Telegram inline-keyboard button into the athlete's chat
// ABOUTME: A tap posts nothing visible, so the label is read off the tapped keyboard and sent before the answer

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Tap echo (carnet#829).
//!
//! A Telegram inline-keyboard tap posts nothing into the chat: the bot is
//! handed a `callback_query` carrying the button's data, and the athlete's
//! choice appears nowhere. A typed reply leaves the athlete's words in the
//! chat, and so does a `WhatsApp` reply button, whose title Meta posts as the
//! athlete's message; on Telegram the chat instead reads as the bot answering
//! a question nobody asked. So before anything the tap produces, the ingress
//! echoes the tapped button's label, as a reply to the message the keyboard
//! sat on.
//!
//! The label is read off the tapped message's own keyboard, which the
//! `callback_query` carries back: the button whose data is the tap's is the
//! one pressed, in whatever language it was drawn. A tap the keyboard no
//! longer explains is not echoed, rather than echoed wrong.
//!
//! Only a direct chat is echoed. In a room the echo would be a line everyone
//! reads, and a command button pressed there is answered to its presser
//! alone.

use std::sync::Arc;

use dravr_canot::channel::MessagingChannel;
use pierre_core::models::messaging::{
    ChannelType, IncomingMessage, MessageContent, OutgoingMessage,
};
use pierre_core::models::TenantId;
use serde_json::Value;

use super::outbound_send::{send_channel_response_first, OutboundPersistSpec};
use super::{extract_thread_id, ResolvedSession};
use crate::mcp::resources::ServerContext;

/// The button a tap pressed, as the echo shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct TappedButton {
    /// The label the button showed.
    label: String,
    /// Channel id of the message whose keyboard held the button; the echo
    /// replies to it.
    keyboard_message_id: Option<String>,
}

/// The button a Telegram inline-keyboard tap pressed, or `None` when
/// `message` is no such tap, or the tapped message's keyboard holds no
/// labelled button carrying the tap's data.
fn tapped_button(message: &IncomingMessage) -> Option<TappedButton> {
    if message.channel_type != ChannelType::Telegram {
        return None;
    }
    let callback = message.raw_payload.get("callback_query")?;
    let data = callback.get("data").and_then(Value::as_str)?;
    let label = callback
        .pointer("/message/reply_markup/inline_keyboard")
        .and_then(Value::as_array)?
        .iter()
        .filter_map(Value::as_array)
        .flatten()
        .find(|button| button.get("callback_data").and_then(Value::as_str) == Some(data))?
        .get("text")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|label| !label.is_empty())?;
    let keyboard_message_id = callback
        .pointer("/message/message_id")
        .and_then(Value::as_i64)
        .map(|id| id.to_string());
    Some(TappedButton {
        label: label.to_owned(),
        keyboard_message_id,
    })
}

/// Echo the label of the button `message` tapped into the athlete's direct
/// chat, before anything the tap produces is sent.
///
/// Awaited rather than spawned, so the echo lands ahead of the confirmation,
/// welcome or answer that follows it, and ledgered on `session`, which
/// `session_tenant_id` owns. Nothing happens for a message that is not a
/// tap, or that arrived in a room. Best-effort: a send that fails is logged
/// by the delivery and the tap is still answered.
pub(super) async fn echo_tapped_button(
    resources: &ServerContext,
    tenant_id: TenantId,
    channel: &str,
    adapter: &Arc<dyn MessagingChannel>,
    message: &IncomingMessage,
    session_tenant_id: TenantId,
    session: &ResolvedSession,
) {
    if !message.is_direct_message {
        return;
    }
    let Some(tapped) = tapped_button(message) else {
        return;
    };
    let ledger = OutboundPersistSpec {
        db: Arc::clone(&resources.common.repos.messaging),
        session_tenant_id,
        session_id: session.session_id.clone(),
        chat_message_id: None,
    };
    let echo = OutgoingMessage {
        channel_type: message.channel_type,
        recipient_id: message.sender_id.clone(),
        content: MessageContent::Text { body: tapped.label },
        turn_id: message.turn_id,
        reply_to: tapped.keyboard_message_id,
        thread_id: extract_thread_id(&message.metadata),
    };
    send_channel_response_first(resources, tenant_id, channel, adapter, echo, Some(ledger)).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use dravr_canot::turn::ConversationTurnId;
    use serde_json::json;

    /// A Telegram update as canot hands it on: the `callback_query` of a tap
    /// on `data`, on a message whose keyboard is `keyboard`.
    fn tap(data: &str, keyboard: &Value) -> IncomingMessage {
        let update = json!({
            "update_id": 9001,
            "callback_query": {
                "id": "4382bf0d",
                "from": { "id": 77, "first_name": "Ana" },
                "data": data,
                "message": {
                    "message_id": 512,
                    "chat": { "id": 77, "type": "private" },
                    "reply_markup": { "inline_keyboard": keyboard },
                },
            },
        });
        IncomingMessage {
            channel_type: ChannelType::Telegram,
            sender_id: "77".to_owned(),
            sender_name: Some("Ana".to_owned()),
            content: MessageContent::Text {
                body: data.to_owned(),
            },
            conversation_id: Some("77".to_owned()),
            chat_title: None,
            channel_message_id: "4382bf0d".to_owned(),
            timestamp: Utc::now(),
            raw_payload: update,
            turn_id: ConversationTurnId::new(),
            is_direct_message: true,
            addressed_to_bot: true,
            metadata: Value::Null,
        }
    }

    /// One button per row, the way canot draws a card's actions.
    fn keyboard(buttons: &[(&str, &str)]) -> Value {
        Value::Array(
            buttons
                .iter()
                .map(|(text, data)| json!([{ "text": text, "callback_data": data }]))
                .collect(),
        )
    }

    #[test]
    fn the_tapped_buttons_label_is_read_off_its_own_keyboard() {
        let keyboard = keyboard(&[
            ("What should I eat before a 6am run?", "ex:0:0"),
            ("How do I carb load for a marathon?", "ex:1:1"),
            ("Trail Agent", "2"),
        ]);

        assert_eq!(
            tapped_button(&tap("ex:1:1", &keyboard)),
            Some(TappedButton {
                label: "How do I carb load for a marathon?".to_owned(),
                keyboard_message_id: Some("512".to_owned()),
            })
        );
        assert_eq!(
            tapped_button(&tap("2", &keyboard)).map(|t| t.label),
            Some("Trail Agent".to_owned())
        );
    }

    #[test]
    fn a_tap_its_keyboard_does_not_explain_is_not_echoed() {
        let keyboard = keyboard(&[("Trail Agent", "2"), ("   ", "3")]);

        assert_eq!(
            tapped_button(&tap("ex:0:0", &keyboard)),
            None,
            "no such button"
        );
        assert_eq!(tapped_button(&tap("3", &keyboard)), None, "a blank label");
    }

    #[test]
    fn a_typed_message_is_no_tap() {
        let mut typed = tap("2", &keyboard(&[("Trail Agent", "2")]));
        typed.raw_payload = json!({
            "update_id": 9002,
            "message": { "message_id": 513, "chat": { "id": 77, "type": "private" }, "text": "2" },
        });
        assert_eq!(tapped_button(&typed), None);

        let mut slack = tap("2", &keyboard(&[("Trail Agent", "2")]));
        slack.channel_type = ChannelType::Slack;
        assert_eq!(
            tapped_button(&slack),
            None,
            "only a Telegram tap carries this shape"
        );
    }
}
