// ABOUTME: Persists a slash-command turn — the athlete's line and the platform's answer — as chat rows
// ABOUTME: One policy per surface decides which command turns land; the rows never replay into a prompt
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Command replies are conversation history.
//!
//! Telegram keeps a bot's answer in the thread; the in-app transcript used to
//! lose it on the first reload, because a slash reply was rendered for the
//! turn and written nowhere. This stage writes the pair — the `/…` line as a
//! `user` row, the answer as an `assistant` row — both stamped with
//! [`COMMAND_FINISH_REASON`] so the replay path drops them by marker, and the
//! answer's controls as an [`PersistedReplyBlock::Actions`] entry in
//! `content_blocks` so a reload shows the same buttons the live turn did.
//!
//! Which command turns land is the surface's call ([`CommandPersistence`]):
//! everything on web, mobile and a messaging DM; in a shared messaging room
//! only the commands whose reply is posted to the room, because a private
//! answer — the caller's provider list, an invite code — is not part of what
//! the room saw.

use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{
    AddMessageParams, ConversationRecord, MessageRecord, PersistedAction, PersistedReplyBlock,
    COMMAND_FINISH_REASON,
};
use pierre_database::database::repositories::ChatRepository;
use tracing::warn;

use crate::envelope::TurnAction;
use crate::turn_service::{CommandTurn, SlashRequest};

/// Commands whose reply belongs to the **room**, not to the caller alone.
///
/// A slash reply usually carries the caller's own account state, so a shared
/// room delivers it privately. These change a setting every member then
/// experiences — the respond mode, the group's coach, the coach bound to the
/// thread — so announcing the change in the room is the point: a member who
/// watches the coach fall silent after someone ran `/group respond mentions`
/// privately has no way to know why. The same set decides what a shared room
/// persists, because what the room saw is what its transcript holds.
///
/// Entries are the command-definition `name:` values (hyphenated ids like
/// `group-respond`), which is what `DispatchOutcome::Executed.command_name`
/// carries — canot's matcher returns `def.name`, not the spaced `/group
/// respond` trigger. Spelling these with a space silently matches nothing;
/// `group_setting_changes_are_announced_in_the_room` pins the real names
/// against the loaded `commands/` catalog.
///
/// LIMITATION(registre#132): `ROOM_VISIBLE_COMMANDS` carries no `plan` entry, so
/// `/plan` in a shared room is answered in the caller's DM and is left out of the
/// room transcript; an athlete and a coach have no way to read or edit a training
/// plan together there. Adding `plan` on its own would publish one member's plan
/// to every other member of the coaching group, so a consent rule and a
/// plan-write path have to land with it.
pub const ROOM_VISIBLE_COMMANDS: [&str; 3] = ["group-respond", "group-coach", "coach-add"];

/// `true` when a command's reply is posted to the room it was typed in.
///
/// `None` — the `/connect` card, the unknown-command reply — is private.
#[must_use]
pub fn is_room_visible(command_name: Option<&str>) -> bool {
    command_name.is_some_and(|name| ROOM_VISIBLE_COMMANDS.contains(&name))
}

/// Which slash-command turns a surface writes to the transcript.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandPersistence {
    /// Every command turn: the in-app surfaces and a messaging DM, where the
    /// conversation is the athlete's own thread and the reply is theirs.
    Always,
    /// Only the commands in [`ROOM_VISIBLE_COMMANDS`]: a shared messaging
    /// room, where a private reply never reached the room's transcript.
    RoomVisibleOnly,
}

impl CommandPersistence {
    /// Whether a turn answered by `command_name` (`None` for the unknown-command
    /// reply) is written under this policy.
    #[must_use]
    pub fn persists(self, command_name: Option<&str>) -> bool {
        match self {
            Self::Always => true,
            Self::RoomVisibleOnly => is_room_visible(command_name),
        }
    }
}

/// The rows a persisted command turn produced.
#[derive(Debug, Clone)]
pub struct PersistedCommandReply {
    /// The athlete's `/…` line.
    pub user_message: MessageRecord,
    /// The platform's answer.
    pub assistant_message: MessageRecord,
    /// The conversation as it stands after both rows landed — its
    /// `updated_at` moved with them.
    pub conversation: ConversationRecord,
}

/// Encode a reply's controls as the `content_blocks` entry a reload reads.
///
/// `None` when the reply carried no controls, so a plain text answer keeps a
/// `NULL` column like any other prose row.
///
/// # Errors
///
/// Returns the serialization error when the block cannot be encoded.
pub fn actions_content_blocks(
    title: Option<&str>,
    actions: &[TurnAction],
) -> Result<Option<String>, serde_json::Error> {
    if actions.is_empty() {
        return Ok(None);
    }
    let block = PersistedReplyBlock::Actions {
        title: title.map(ToOwned::to_owned),
        actions: actions
            .iter()
            .map(|action| PersistedAction {
                label: action.label.clone(),
                action_type: action.kind.as_str().to_owned(),
                value: action.value.clone(),
            })
            .collect(),
    };
    serde_json::to_string(&[block]).map(Some)
}

/// Write a command turn's two rows and advance the caller's read marker.
///
/// The caller has just been shown the answer, so the marker moves to the
/// assistant row the same way it does after a coaching turn; the command
/// therefore never counts as unread for the athlete who ran it.
///
/// # Errors
///
/// Returns the repository error when a row cannot be written or the
/// conversation cannot be re-read; the caller treats that as best-effort and
/// still delivers the reply.
pub async fn persist_command_turn(
    chat: &dyn ChatRepository,
    request: &SlashRequest<'_>,
    command: &CommandTurn,
) -> AppResult<PersistedCommandReply> {
    let user_id = request.user_id.to_string();
    let user_message = chat
        .add_message(&AddMessageParams {
            tenant_id: request.conversation_tenant_id,
            conversation_id: request.conversation_id,
            user_id: &user_id,
            role: "user",
            content: request.text,
            token_count: None,
            finish_reason: Some(COMMAND_FINISH_REASON),
            prompt_tokens: None,
            model: None,
            content_blocks: None,
        })
        .await?;

    let content_blocks = actions_content_blocks(command.card_title.as_deref(), &command.actions)
        .unwrap_or_else(|e| {
            warn!(error = %e, "command actions could not be encoded; persisting the reply without controls");
            None
        });
    let assistant_message = chat
        .add_message(&AddMessageParams {
            tenant_id: request.conversation_tenant_id,
            conversation_id: request.conversation_id,
            user_id: &user_id,
            role: "assistant",
            content: &command.text,
            token_count: None,
            finish_reason: Some(COMMAND_FINISH_REASON),
            prompt_tokens: None,
            model: None,
            content_blocks: content_blocks.as_deref(),
        })
        .await?;

    if !chat
        .mark_conversation_read(
            request.conversation_id,
            &user_id,
            request.conversation_tenant_id,
            Some(&assistant_message.id),
        )
        .await?
    {
        warn!(
            conversation_id = %request.conversation_id,
            "command reply persisted but the caller's read marker did not advance"
        );
    }

    let conversation = chat
        .get_conversation(
            request.conversation_id,
            &user_id,
            request.conversation_tenant_id,
        )
        .await?
        .ok_or_else(|| {
            AppError::internal("Conversation vanished after its command reply was persisted")
        })?;

    Ok(PersistedCommandReply {
        user_message,
        assistant_message,
        conversation,
    })
}
