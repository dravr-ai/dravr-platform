// ABOUTME: Who reads a tool call's reply: the caller alone, a group's room, or a chat not confirmed private
// ABOUTME: The one room classifier every tool that keeps a coach's or another athlete's data out of a room asks
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Call room
//!
//! A tool that reads a coach's roster, another athlete's data or calendar, or
//! acts on an athlete's plan answers only where the caller alone reads the
//! reply. [`CallRoom::of`] is the one place that tells: every such tool asks
//! it, whether it pins the room's group (`call_room`) or refuses anything but
//! a private conversation (`coach_scope::room_refusal_in`,
//! `plan_scope::resolve_plan_scope`).
//!
//! The conversation row alone cannot tell. A messaging group chat whose group
//! binding failed (the group is full, the plan has no group coaching, no
//! agent) keeps no group, exactly as a direct message does, and its row may
//! resolve under the caller's own tenant when that is the bot tenant. So a
//! row with no group is private only on a turn marked a direct message
//! ([`ToolExecutionContext::turn_direct_message`], carnet#1421,
//! carnet#1422).

#[cfg(feature = "tools-groups")]
use pierre_core::errors::AppResult;
use pierre_core::models::ConversationRecord;
#[cfg(feature = "tools-groups")]
use pierre_core::models::TenantId;

use crate::context::ToolExecutionContext;

/// Where a call was made, as the tools that keep data out of a room see it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum CallRoom {
    /// No conversation, or one private to the caller.
    Private,
    /// A group's room, read by every member: the group's id.
    Group(String),
    /// A conversation confirmed neither private to the caller nor a group's
    /// room: one that does not resolve for them, which may be any room; a
    /// chat other people read that no group is bound to — a messaging group
    /// chat whose binding failed (carnet#1421); or a call that names a
    /// conversation without saying who reads it. Nothing about an athlete is
    /// published into it.
    Unresolved,
}

impl CallRoom {
    /// The room a call from `context` was made in, when its conversation's
    /// row, read for the caller, is `conversation` (`None` when it does not
    /// resolve for them).
    ///
    /// A row with no group is private only on a turn the caller alone reads
    /// ([`ToolExecutionContext::turn_direct_message`] is `Some(true)`). A
    /// call that does not say is not taken for private, so a context built
    /// outside a turn fails closed unless its builder states the audience.
    #[must_use]
    pub(super) fn of(
        context: &ToolExecutionContext,
        conversation: Option<&ConversationRecord>,
    ) -> Self {
        Self::classify(
            context.conversation_id.is_some(),
            conversation,
            context.turn_direct_message,
        )
    }

    /// [`Self::of`] over its parts: whether the call names a conversation,
    /// the row read for the caller, and the turn's audience.
    fn classify(
        named: bool,
        conversation: Option<&ConversationRecord>,
        direct_message: Option<bool>,
    ) -> Self {
        if !named {
            return Self::Private;
        }
        conversation.map_or(Self::Unresolved, |conversation| {
            match &conversation.group_id {
                Some(group) => Self::Group(group.clone()),
                None if direct_message == Some(true) => Self::Private,
                None => Self::Unresolved,
            }
        })
    }

    /// Whether only the caller reads the reply.
    #[must_use]
    pub(super) const fn is_private(&self) -> bool {
        matches!(self, Self::Private)
    }
}

/// The room `context`'s conversation is, its row resolved under the
/// conversation's own tenant (a shared room's row lives under the bot
/// tenant), else the caller's.
///
/// # Errors
///
/// Returns the repository error from the conversation lookup.
#[cfg(feature = "tools-groups")]
pub(super) async fn call_room(context: &ToolExecutionContext) -> AppResult<CallRoom> {
    let Some(conversation_id) = context.conversation_id.as_deref() else {
        return Ok(CallRoom::Private);
    };
    let tenant = context
        .conversation_ref()
        .map(|(_, tenant)| tenant)
        .or_else(|| context.tenant_id.map(TenantId::from_uuid));
    let conversation = match tenant {
        Some(tenant) => {
            context
                .resources
                .repos()
                .chat
                .get_conversation(conversation_id, &context.user_id.to_string(), tenant)
                .await?
        }
        None => None,
    };
    Ok(CallRoom::of(context, conversation.as_ref()))
}

#[cfg(test)]
mod tests {
    use pierre_core::models::ConversationRecord;
    use serde_json::json;

    use super::CallRoom;

    /// A messaging conversation row, bound to `group` when one is given.
    fn row(group: Option<&str>) -> ConversationRecord {
        serde_json::from_value(json!({
            "id": "conversation",
            "user_id": "coach",
            "tenant_id": "bot-tenant",
            "title": "",
            "model": "model",
            "total_tokens": 0,
            "created_at": "",
            "updated_at": "",
            "channel_type": "telegram",
            "group_id": group,
        }))
        .expect("a conversation row deserializes")
    }

    #[test]
    fn a_call_naming_no_conversation_is_private() {
        assert_eq!(
            CallRoom::classify(false, None, None),
            CallRoom::Private,
            "a call outside any conversation (MCP-direct) is read by the caller alone"
        );
    }

    #[test]
    fn a_row_with_no_group_is_private_only_on_a_direct_message() {
        let ungrouped = row(None);
        assert_eq!(
            CallRoom::classify(true, Some(&ungrouped), Some(true)),
            CallRoom::Private,
            "a direct message is read by the caller alone"
        );
        assert_eq!(
            CallRoom::classify(true, Some(&ungrouped), Some(false)),
            CallRoom::Unresolved,
            "an ungrouped messaging group chat is read by everyone in it (carnet#1422)"
        );
        assert_eq!(
            CallRoom::classify(true, Some(&ungrouped), None),
            CallRoom::Unresolved,
            "a call that does not say who reads it fails closed"
        );
    }

    #[test]
    fn a_group_bound_row_is_its_group_s_room_whoever_reads_the_turn() {
        let bound = row(Some("squad"));
        for direct_message in [Some(true), Some(false), None] {
            assert_eq!(
                CallRoom::classify(true, Some(&bound), direct_message),
                CallRoom::Group("squad".to_owned()),
                "a group-bound row is its group's room on a {direct_message:?} turn"
            );
        }
    }

    #[test]
    fn a_conversation_that_does_not_resolve_for_the_caller_is_unresolved() {
        assert_eq!(
            CallRoom::classify(true, None, Some(true)),
            CallRoom::Unresolved,
            "a row the caller's tenant cannot resolve may be any room, even on a direct message"
        );
    }
}
