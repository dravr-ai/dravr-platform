// ABOUTME: Channel reply addressing — the channel-native id an outbound reply is sent to, and the message it anchors on.
// ABOUTME: Single source of truth for the recipient fallback (dispatch + backfill push) and for where an answer lands.

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::models::messaging::ChannelType;

/// Resolve the channel-native id an outbound reply is addressed to.
///
/// Prefer the conversation/chat/thread id — that is the originating room for a
/// group, a thread, or a channel-based platform (Discord/Slack), so the reply
/// lands exactly where the inbound message arrived. Fall back to the per-user
/// id when the conversation id is absent (`None`) for a DM-only platform like
/// `WhatsApp`, or present-but-empty for a DM session whose
/// `channel_conversation_id` is stored NULL/empty (one DM per user — the
/// group/DM split keys the conversation id, so a DM leaves it blank).
///
/// This unifies four previously-divergent copies of the fallback: the
/// synchronous `deliver_reply` + `send_plain_reply` dispatch paths and the
/// slash-command reply checked only the `None` case, while the backfill
/// notifier's `resolve_route` also filtered the empty string. The
/// empty-string case is the silent-DM-drop class fixed in
/// commit 5df2c1706 — every direct-message backfill-completion push died with no
/// trace because the empty conversation id was treated as a valid recipient.
/// Folding the superset rule (fall back on `None` OR empty) into one helper
/// hardens the dispatch paths too: they never legitimately carry an empty
/// conversation id, so the added empty-filter is a harmless no-op there.
#[must_use]
pub fn reply_recipient<'a>(conversation_id: Option<&'a str>, fallback_user_id: &'a str) -> &'a str {
    conversation_id
        .filter(|id| !id.is_empty())
        .unwrap_or(fallback_user_id)
}

/// The message a piece of the athlete's answer anchors on (`reply_to`).
///
/// On most channels `reply_to` quotes the athlete's message and does not move
/// the answer: Telegram renders a quoted reply, Discord a message reference.
/// On Slack it decides *where* the answer lands, because the renderer turns it
/// into `thread_ts` and so opens a thread under the athlete's message.
///
/// The Slack status placeholder carries the first part of every answer, and it
/// opens on the turn's thread (`thread_id`), which is the top level for a
/// message written in the channel or the DM. Anchoring the rest on the
/// athlete's message would split one answer across two places: a long
/// answer's continuations, and the whole answer whenever the placeholder post
/// fails, would land in a thread the athlete never opens (carnet#1490). So on
/// Slack an answer anchors exactly where its placeholder does.
#[must_use]
pub(crate) fn answer_reply_to(
    channel_type: ChannelType,
    channel_message_id: &str,
    thread_id: Option<&str>,
) -> Option<String> {
    if channel_type == ChannelType::Slack {
        thread_id.map(str::to_owned)
    } else {
        Some(channel_message_id.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_slack_answer_lands_where_its_placeholder_does() {
        assert_eq!(
            answer_reply_to(ChannelType::Slack, "1794316793.587409", None),
            None,
            "a channel-level message is answered at the channel level, never in a new thread"
        );
        assert_eq!(
            answer_reply_to(
                ChannelType::Slack,
                "1794316793.587409",
                Some("1794316700.000100")
            )
            .as_deref(),
            Some("1794316700.000100"),
            "a message inside a thread is answered in that thread"
        );
    }

    #[test]
    fn other_channels_still_quote_the_athletes_message() {
        for channel_type in [ChannelType::Telegram, ChannelType::Discord] {
            assert_eq!(
                answer_reply_to(channel_type, "inbound-7", Some("topic-3")).as_deref(),
                Some("inbound-7"),
                "{channel_type:?} keeps quoting the message it answers"
            );
        }
    }
}
