// ABOUTME: Who reads what is said in a conversation: the caller alone, a group's room, or a chat not confirmed private
// ABOUTME: The one room classifier every tool, listing, chat route, history replay and transcript read asks (carnet#1422)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Call room
//!
//! A tool that reads a coach's roster, another athlete's data or calendar, or
//! acts on an athlete's plan answers only where the caller alone reads the
//! reply. [`CallRoom`] is the one place that tells: every such tool asks
//! it, whether it pins the room's group (`call_room`) or refuses anything but
//! a private conversation (`coach_scope::room_refusal_in`,
//! `plan_scope::resolve_plan_scope`, and a read naming an athlete with
//! `athlete=`, through [`turn_room`]). The chat route asks it whether a turn
//! is a direct message, a turn's listing what to offer, and the history
//! replay which stored rows a room may be given.
//!
//! The conversation row alone cannot tell. A messaging group chat whose group
//! binding failed (the group is full, the plan has no group coaching, no
//! agent) keeps no group, exactly as a direct message does, and its row may
//! resolve under the caller's own tenant when that is the bot tenant. So a
//! row with no group is private only on a turn marked a direct message
//! ([`ToolExecutionContext::turn_direct_message`], carnet#1421,
//! carnet#1422), and only when every messaging session writing into it is
//! recorded as a direct message's: each member's next turn in a group chat
//! replays the row's history into a reply the whole chat reads, and a
//! session whose chat is not on record may be a group chat's, the personal
//! thread included. A DM's session keeps its thread a direct message. Nor is
//! a row anyone else participates in private: a participant added to the
//! thread (`POST /api/chat/conversations/{id}/participants`) reads every
//! reply stored in it. The verdict is taken again each time it is asked, never
//! carried from the turn that wrote a row, because a row's audience changes:
//! a session is repointed, a group is bound.
//!
//! The classifier lives here, below the tool runtime, so the transcript
//! reads ([`crate::transcript_reading`]) ask the same one the tools do;
//! `pierre_tool_runtime::implementations::call_room` reads a tool call's
//! facts off its context and asks it.

use pierre_core::models::{ConversationRecord, TenantId};
use pierre_database::repositories::SessionChat;
use pierre_database::RepositoryRegistry;
use tracing::warn;

/// Where a call was made, as the tools that keep data out of a room see it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallRoom {
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

/// What the messaging chats writing into a conversation say about who reads
/// it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChatSessions {
    /// No messaging chat writes into it.
    None,
    /// Only a direct message's session writes into it.
    DirectMessage,
    /// A group chat's session writes into it; or a session whose chat is not
    /// on record does, whatever thread it writes into, the owner's personal
    /// one included (a group chat's row designated personal before
    /// carnet#1422 is one); or the sessions could not be read.
    Room,
}

impl ChatSessions {
    /// The verdict of `chats`, the sessions writing into a conversation.
    ///
    /// Only a chat recorded as a direct message keeps the thread its owner's:
    /// a session whose chat is not on record may be a group chat's, and the
    /// personal flag proves nothing, since a group chat's row could be
    /// designated personal before the adoption read the sessions.
    #[must_use]
    pub(crate) fn of(chats: &[SessionChat]) -> Self {
        if chats.is_empty() {
            return Self::None;
        }
        let room = chats.iter().any(|chat| match chat {
            SessionChat::Room | SessionChat::Unrecorded => true,
            SessionChat::DirectMessage => false,
        });
        if room {
            Self::Room
        } else {
            Self::DirectMessage
        }
    }

    /// The sessions writing into `conversation`, read under the row's own
    /// tenant, which is the one its sessions live under. A tenant that does
    /// not parse, or a lookup that fails, is taken for a room's.
    async fn read(repos: &RepositoryRegistry, conversation: &ConversationRecord) -> Self {
        let Ok(tenant) = TenantId::parse_str(&conversation.tenant_id) else {
            warn!("call room: the conversation's tenant is not a uuid; serving it as a room");
            return Self::Room;
        };
        Self::read_in(repos, tenant, &conversation.id).await
    }

    /// The sessions writing into `conversation_id`, a row of `tenant`. A
    /// lookup that fails is taken for a room's.
    async fn read_in(repos: &RepositoryRegistry, tenant: TenantId, conversation_id: &str) -> Self {
        match repos.messaging.session_chats(tenant, conversation_id).await {
            Ok(chats) => Self::of(&chats),
            Err(e) => {
                warn!(error = %e, "call room: the thread's messaging sessions could not be read; serving it as a room");
                Self::Room
            }
        }
    }
}

/// Whether anyone but one participant reads a conversation's stored rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RowParticipants {
    /// One participant alone: the caller, whose row it resolved as.
    Alone,
    /// Someone else participates as well, or the participants could not be
    /// read.
    Shared,
}

impl RowParticipants {
    /// The participants of `conversation`, read under the row's own tenant.
    /// A tenant that does not parse, or a lookup that fails, is taken for a
    /// shared row's.
    async fn read(repos: &RepositoryRegistry, conversation: &ConversationRecord) -> Self {
        let Ok(tenant) = TenantId::parse_str(&conversation.tenant_id) else {
            warn!("call room: the conversation's tenant is not a uuid; serving it as shared");
            return Self::Shared;
        };
        match repos.chat.list_participants(&conversation.id, tenant).await {
            Ok(participants) if participants.len() <= 1 => Self::Alone,
            Ok(_) => Self::Shared,
            Err(e) => {
                warn!(error = %e, "call room: the thread's participants could not be read; serving it as shared");
                Self::Shared
            }
        }
    }
}

/// The facts of a conversation row the classifier reads: whether it is bound
/// to a group, and which.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RoomRow<'conv> {
    /// The group whose room the row is, when bound to one.
    pub(crate) group_id: Option<&'conv str>,
}

impl<'conv> RoomRow<'conv> {
    /// The facts of a stored conversation row.
    #[must_use]
    pub(crate) fn of(conversation: &'conv ConversationRecord) -> Self {
        Self {
            group_id: conversation.group_id.as_deref(),
        }
    }
}

impl CallRoom {
    /// The room a reader reads `conversation_id`'s stored rows in on their own
    /// screen, the row filed under `tenant` and bound to `group_id`.
    ///
    /// The classifier over the same facts a turn's room reads, but one: the
    /// screen is the reader's own, so whoever else participates reads their
    /// own screen under their own rule ([`RowParticipants::Alone`]), and the
    /// reader reads as on a direct message. The messaging chats writing into
    /// the row are read as for a turn, so a row a group chat writes into is
    /// never read as private: no group's consent says who may be read there
    /// (carnet#1422).
    pub async fn of_screen_read(
        repos: &RepositoryRegistry,
        tenant: TenantId,
        conversation_id: &str,
        group_id: Option<&str>,
    ) -> Self {
        let row = RoomRow { group_id };
        let sessions = if group_id.is_none() {
            ChatSessions::read_in(repos, tenant, conversation_id).await
        } else {
            ChatSessions::None
        };
        Self::classify(
            true,
            Some(row),
            Some(true),
            sessions,
            RowParticipants::Alone,
        )
    }

    /// The room a turn on `conversation` runs in, the row read for the
    /// turn's sender, when the turn is a direct message or not: what
    /// `turn_room` answers a call from that turn, without reading the row
    /// again. The chat route asks it whether a turn is a direct message
    /// (`direct_message` true: the app and an API key read their reply
    /// alone, unless the thread says otherwise), and the history replay
    /// which rows the turn may be given.
    pub async fn of_conversation(
        repos: &RepositoryRegistry,
        conversation: &ConversationRecord,
        direct_message: bool,
    ) -> Self {
        Self::read(repos, true, Some(conversation), Some(direct_message)).await
    }

    /// [`Self::classify`] over the row, its sessions read only when they can
    /// change the answer.
    pub async fn read(
        repos: &RepositoryRegistry,
        named: bool,
        conversation: Option<&ConversationRecord>,
        direct_message: Option<bool>,
    ) -> Self {
        let (sessions, participants) = match conversation {
            Some(row) if named && row.group_id.is_none() && direct_message == Some(true) => (
                ChatSessions::read(repos, row).await,
                RowParticipants::read(repos, row).await,
            ),
            _ => (ChatSessions::None, RowParticipants::Alone),
        };
        Self::classify(
            named,
            conversation.map(RoomRow::of),
            direct_message,
            sessions,
            participants,
        )
    }

    /// The one rule, over its parts: whether the call names a conversation,
    /// the row read for the caller, the turn's audience, the messaging chats
    /// writing into the row, and whether anyone else participates in it.
    ///
    /// A row bound to a group is that group's room. One with no group is
    /// private only on a turn the caller alone reads, only when no group
    /// chat writes into it, and only when no one else participates in it: a
    /// row a group chat's session points at is never private, whatever its
    /// personal flag, a thread a DM's session points at stays a direct
    /// message, and a thread shared with another participant is read by them.
    #[must_use]
    pub(crate) fn classify(
        named: bool,
        conversation: Option<RoomRow<'_>>,
        direct_message: Option<bool>,
        sessions: ChatSessions,
        participants: RowParticipants,
    ) -> Self {
        if !named {
            return Self::Private;
        }
        let Some(conversation) = conversation else {
            return Self::Unresolved;
        };
        if let Some(group) = conversation.group_id {
            return Self::Group(group.to_owned());
        }
        match (direct_message, sessions, participants) {
            (
                Some(true),
                ChatSessions::None | ChatSessions::DirectMessage,
                RowParticipants::Alone,
            ) => Self::Private,
            _ => Self::Unresolved,
        }
    }

    /// Whether only the caller reads the reply.
    #[must_use]
    pub const fn is_private(&self) -> bool {
        matches!(self, Self::Private)
    }
}

#[cfg(test)]
mod tests {
    use pierre_core::models::ConversationRecord;
    use serde_json::json;

    use pierre_database::repositories::SessionChat;

    use super::{CallRoom, ChatSessions, RoomRow, RowParticipants};

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
            CallRoom::classify(
                false,
                None,
                None,
                ChatSessions::None,
                RowParticipants::Alone
            ),
            CallRoom::Private,
            "a call outside any conversation (MCP-direct) is read by the caller alone"
        );
    }

    #[test]
    fn a_row_with_no_group_is_private_only_on_a_direct_message() {
        let ungrouped = row(None);
        assert_eq!(
            CallRoom::classify(
                true,
                Some(RoomRow::of(&ungrouped)),
                Some(true),
                ChatSessions::None,
                RowParticipants::Alone
            ),
            CallRoom::Private,
            "a direct message is read by the caller alone"
        );
        assert_eq!(
            CallRoom::classify(
                true,
                Some(RoomRow::of(&ungrouped)),
                Some(false),
                ChatSessions::None,
                RowParticipants::Alone
            ),
            CallRoom::Unresolved,
            "an ungrouped messaging group chat is read by everyone in it (carnet#1422)"
        );
        assert_eq!(
            CallRoom::classify(
                true,
                Some(RoomRow::of(&ungrouped)),
                None,
                ChatSessions::None,
                RowParticipants::Alone
            ),
            CallRoom::Unresolved,
            "a call that does not say who reads it fails closed"
        );
    }

    #[test]
    fn a_group_bound_row_is_its_group_s_room_whoever_reads_the_turn() {
        let bound = row(Some("squad"));
        for direct_message in [Some(true), Some(false), None] {
            assert_eq!(
                CallRoom::classify(
                    true,
                    Some(RoomRow::of(&bound)),
                    direct_message,
                    ChatSessions::Room,
                    RowParticipants::Alone
                ),
                CallRoom::Group("squad".to_owned()),
                "a group-bound row is its group's room on a {direct_message:?} turn"
            );
        }
    }

    #[test]
    fn a_conversation_that_does_not_resolve_for_the_caller_is_unresolved() {
        assert_eq!(
            CallRoom::classify(
                true,
                None,
                Some(true),
                ChatSessions::None,
                RowParticipants::Alone
            ),
            CallRoom::Unresolved,
            "a row the caller's tenant cannot resolve may be any room, even on a direct message"
        );
    }

    #[test]
    fn a_row_a_group_chat_writes_into_is_never_private() {
        let mut personal = row(None);
        personal.personal = true;
        for chats in [
            vec![SessionChat::Room],
            vec![SessionChat::DirectMessage, SessionChat::Room],
        ] {
            assert_eq!(
                CallRoom::classify(
                    true,
                    Some(RoomRow::of(&personal)),
                    Some(true),
                    ChatSessions::of(&chats),
                    RowParticipants::Alone
                ),
                CallRoom::Unresolved,
                "a group chat's session makes the row a room, personal or not: {chats:?}"
            );
        }
    }

    #[test]
    fn a_thread_a_dm_writes_into_stays_a_direct_message() {
        let legacy = row(None);
        assert_eq!(
            CallRoom::classify(
                true,
                Some(RoomRow::of(&legacy)),
                Some(true),
                ChatSessions::of(&[SessionChat::DirectMessage]),
                RowParticipants::Alone
            ),
            CallRoom::Private,
            "a DM's older thread is still its owner's alone"
        );
    }

    #[test]
    fn a_session_whose_chat_is_not_on_record_is_a_room_on_any_thread() {
        let mut personal = row(None);
        personal.personal = true;
        assert_eq!(
            ChatSessions::of(&[SessionChat::Unrecorded]),
            ChatSessions::Room,
            "a session whose chat is not on record may be a group chat's"
        );
        assert_eq!(
            CallRoom::classify(
                true,
                Some(RoomRow::of(&personal)),
                Some(true),
                ChatSessions::of(&[SessionChat::Unrecorded]),
                RowParticipants::Alone
            ),
            CallRoom::Unresolved,
            "a personal thread an unrecorded session writes into is not private"
        );
        assert_eq!(
            ChatSessions::of(&[]),
            ChatSessions::None,
            "no session reads as no chat sessions"
        );
    }

    #[test]
    fn a_thread_shared_with_another_participant_is_never_private() {
        let shared = row(None);
        for chats in [vec![], vec![SessionChat::DirectMessage]] {
            assert_eq!(
                CallRoom::classify(
                    true,
                    Some(RoomRow::of(&shared)),
                    Some(true),
                    ChatSessions::of(&chats),
                    RowParticipants::Shared
                ),
                CallRoom::Unresolved,
                "a participant added to the thread reads every reply in it: {chats:?}"
            );
        }
    }
}
