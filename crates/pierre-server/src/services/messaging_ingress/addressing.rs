// ABOUTME: Channel reply-recipient addressing helper — picks the channel-native id an outbound reply is sent to.
// ABOUTME: Single source of truth for the conversation-id-with-user-id-fallback rule shared by dispatch + backfill push.

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

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
/// synchronous `deliver_reply` + `send_error_reply` dispatch paths and the
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
