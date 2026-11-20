// ABOUTME: Connect an existing coaching group to a messaging group chat, from the web app (carnet#1683)
// ABOUTME: Mint a single-use code, redeem it in the chat (owner or coach only), disconnect; the chat is then members-only

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Connecting a group to a chat.
//!
//! A group made in the web app has no chat. Its owner or human coach mints a
//! connect code here ([`GroupService::mint_chat_connect_code`]); the bot
//! redeems it inside the chat they want ([`GroupService::connect_chat`]), and
//! from then on that chat is the group's room: same agent, same coach, same
//! roster and consent.
//!
//! Three decisions shape it (carnet#1683, Phil's directives D1–D3):
//!
//! - **One chat app per group (D1).** The binding stays the single
//!   `(channel_type, channel_chat_id)` pair on `coaching_groups`; a group
//!   already bound to a chat is refused until it is disconnected.
//! - **Members only.** A connected chat never enrols the people who write in
//!   it, unlike a chat that created its own group: membership goes through
//!   invites and per-member consent, so a sender who is not a member (or the
//!   group's coach) gets no group context ([`GroupService::admits_to_connected_chat`]).
//! - **A chat already bound to another group (D3)** moves only when the
//!   person connecting owns that group; anyone else is refused, because the
//!   owner of a chat-created group is its first sender, who may be an athlete.
//!   The old group keeps its members and history and loses only the chat.

use std::fmt;

use chrono::{Duration, Utc};
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::groups::{
    CoachingGroup, ConnectGroupChat, GroupChatConnectCode, GroupChatConnectOutcome,
    ReleasedGroupChat,
};
use pierre_core::models::TenantId;
use tracing::info;
use uuid::Uuid;

use crate::invites::unambiguous_code;
use crate::GroupService;

/// How long a connect code stays redeemable, the same window a channel
/// pairing code gets: long enough to open the chat app and pick the chat.
pub const CHAT_CONNECT_CODE_TTL_MINUTES: i64 = 10;

/// Length of a connect code. Typed by hand in the fallback (`/group link`),
/// so it uses the unambiguous invite alphabet; twelve of its characters are
/// far beyond guessing inside the code's ten minutes.
const CHAT_CONNECT_CODE_LEN: usize = 12;

/// The chat a connect code was redeemed in, and who redeemed it. `Debug`
/// never prints the code.
#[derive(Clone, Copy)]
pub struct ChatToConnect<'a> {
    /// The code the sender typed (or the add-to-group link carried).
    pub code: &'a str,
    /// Channel of the chat (`telegram`).
    pub channel_type: &'a str,
    /// Channel-native id of the chat.
    pub channel_chat_id: &'a str,
    /// Tenant whose bot received the code: only its own codes redeem.
    pub channel_tenant_id: TenantId,
    /// The chat's title, when the platform sent one.
    pub chat_title: Option<&'a str>,
    /// The Dravr user the sender's chat identity is linked to.
    pub sender: Uuid,
}

impl fmt::Debug for ChatToConnect<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ChatToConnect")
            .field("code", &"<redacted>")
            .field("channel_type", &self.channel_type)
            .field("channel_chat_id", &self.channel_chat_id)
            .field("channel_tenant_id", &self.channel_tenant_id)
            .field("sender", &self.sender)
            .finish_non_exhaustive()
    }
}

/// Why a connect redeemed in a chat was refused. Nothing changed in any of
/// these cases.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatConnectRefusal {
    /// The code is unknown, expired, spent, or was minted for another
    /// channel or another bot.
    CodeInvalid,
    /// The sender is neither the group's owner nor its coach.
    NotManager,
    /// The group is already connected to another chat (D1).
    GroupConnectedElsewhere {
        /// The group's name, for the reply.
        group_name: String,
    },
    /// Another group holds this chat and the sender does not own it (D3).
    ChatTakenByOtherGroup,
    /// The group was archived since the code was minted.
    GroupGone,
}

/// What redeeming a connect code in a chat did.
#[derive(Debug, Clone)]
pub enum ChatConnectResult {
    /// The chat is now the group's room.
    Connected {
        /// The group, as it reads now.
        group: Box<CoachingGroup>,
        /// The group the chat moved off, when it was bound to one the
        /// sender owns (D3).
        moved_from: Option<Uuid>,
    },
    /// The chat already was this group's room; nothing changed.
    AlreadyConnected {
        /// The group the chat belongs to.
        group: Box<CoachingGroup>,
    },
    /// Refused; nothing changed.
    Refused(ChatConnectRefusal),
}

impl GroupService {
    /// Mint a code that connects `group` to a chat on `channel_type`, when the
    /// bot of `channel_tenant_id` redeems it there.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::PermissionDenied`] when `caller` is neither the group's
    /// owner nor its coach; [`ErrorCode::ResourceAlreadyExists`] when the
    /// group is already connected to a chat (D1: one chat app per group);
    /// the database error when the code cannot be stored.
    pub async fn mint_chat_connect_code(
        &self,
        group: &CoachingGroup,
        caller: Uuid,
        channel_type: &str,
        channel_tenant_id: TenantId,
    ) -> AppResult<GroupChatConnectCode> {
        if !group.chat_manager(caller) {
            return Err(AppError::new(
                ErrorCode::PermissionDenied,
                "Only the group's owner or coach can connect it to a chat",
            ));
        }
        if group.channel_type.is_some() {
            return Err(AppError::new(
                ErrorCode::ResourceAlreadyExists,
                "This group is already connected to a chat; disconnect it first",
            ));
        }
        let now = Utc::now();
        let code = GroupChatConnectCode {
            code: unambiguous_code(CHAT_CONNECT_CODE_LEN),
            group_id: group.id,
            channel_type: channel_type.to_owned(),
            channel_tenant_id,
            created_by: caller,
            expires_at: now + Duration::minutes(CHAT_CONNECT_CODE_TTL_MINUTES),
        };
        self.repo().create_chat_connect_code(&code, now).await?;
        info!(
            group_id = %group.id,
            channel_type,
            user_id = %caller,
            "Minted a chat connect code for a group"
        );
        Ok(code)
    }

    /// Redeem a connect code inside the chat it was sent in.
    ///
    /// # Errors
    ///
    /// The database error when a lookup or the connect transaction fails.
    /// Every business refusal is an `Ok(ChatConnectResult::Refused)`.
    pub async fn connect_chat(&self, chat: &ChatToConnect<'_>) -> AppResult<ChatConnectResult> {
        let now = Utc::now();
        let repo = self.repo();
        let Some(code) = repo
            .live_chat_connect_code(chat.code, chat.channel_type, chat.channel_tenant_id, now)
            .await?
        else {
            return Ok(ChatConnectResult::Refused(ChatConnectRefusal::CodeInvalid));
        };
        let group = match repo
            .get_group(&code.group_id.to_string(), chat.channel_tenant_id)
            .await?
        {
            Some(group) if group.is_active => group,
            _ => return Ok(ChatConnectResult::Refused(ChatConnectRefusal::GroupGone)),
        };
        if !group.chat_manager(chat.sender) {
            return Ok(ChatConnectResult::Refused(ChatConnectRefusal::NotManager));
        }
        if group.channel_type.is_some() {
            let same_chat = group.channel_type.as_deref() == Some(chat.channel_type)
                && group.channel_chat_id.as_deref() == Some(chat.channel_chat_id)
                && group.chat_serving_tenant() == Some(chat.channel_tenant_id);
            return Ok(if same_chat {
                ChatConnectResult::AlreadyConnected {
                    group: Box::new(group),
                }
            } else {
                ChatConnectResult::Refused(ChatConnectRefusal::GroupConnectedElsewhere {
                    group_name: group.name,
                })
            });
        }

        // D3: a chat bound to another group moves only off a group the
        // sender owns.
        let release_group_id = match repo
            .get_group_by_channel(
                chat.channel_tenant_id,
                chat.channel_type,
                chat.channel_chat_id,
            )
            .await?
        {
            Some(holder) if holder.owner_id == chat.sender => Some(holder.id),
            Some(_) => {
                return Ok(ChatConnectResult::Refused(
                    ChatConnectRefusal::ChatTakenByOtherGroup,
                ))
            }
            None => None,
        };

        let outcome = repo
            .connect_group_chat(&ConnectGroupChat {
                code: chat.code,
                group_id: group.id,
                channel_type: chat.channel_type,
                channel_chat_id: chat.channel_chat_id,
                channel_tenant_id: chat.channel_tenant_id,
                chat_title: chat.chat_title.map(str::trim).filter(|t| !t.is_empty()),
                release_group_id,
                now,
            })
            .await?;
        let refusal = match outcome {
            GroupChatConnectOutcome::Connected => None,
            GroupChatConnectOutcome::CodeSpent => Some(ChatConnectRefusal::CodeInvalid),
            GroupChatConnectOutcome::GroupAlreadyConnected => {
                Some(ChatConnectRefusal::GroupConnectedElsewhere {
                    group_name: group.name.clone(),
                })
            }
            GroupChatConnectOutcome::ChatTaken => Some(ChatConnectRefusal::ChatTakenByOtherGroup),
        };
        if let Some(refusal) = refusal {
            return Ok(ChatConnectResult::Refused(refusal));
        }

        info!(
            group_id = %group.id,
            channel_type = chat.channel_type,
            moved_from = ?release_group_id,
            user_id = %chat.sender,
            "Connected a group to a chat"
        );
        let connected = repo
            .get_group(&group.id.to_string(), chat.channel_tenant_id)
            .await?
            .ok_or_else(|| AppError::internal("Group vanished right after it was connected"))?;
        Ok(ChatConnectResult::Connected {
            group: Box::new(connected),
            moved_from: release_group_id,
        })
    }

    /// Disconnect `group` from its chat, whichever way the binding was made.
    /// The chat then stays ungrouped: its next message creates no group.
    ///
    /// Returns the binding removed, `None` when the group had no chat.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::PermissionDenied`] when `caller` is neither the group's
    /// owner nor its coach; the database error when the write fails.
    pub async fn disconnect_chat(
        &self,
        group: &CoachingGroup,
        caller: Uuid,
    ) -> AppResult<Option<ReleasedGroupChat>> {
        if !group.chat_manager(caller) {
            return Err(AppError::new(
                ErrorCode::PermissionDenied,
                "Only the group's owner or coach can disconnect its chat",
            ));
        }
        let released = self
            .repo()
            .disconnect_group_chat(group.id, Utc::now())
            .await?;
        if let Some(chat) = released.as_ref() {
            info!(
                group_id = %group.id,
                channel_type = %chat.channel_type,
                user_id = %caller,
                "Disconnected a group from its chat"
            );
        }
        Ok(released)
    }

    /// Whether `user_id` takes part in `group`'s connected chat: a live
    /// member, or the group's human coach (who holds no membership row).
    ///
    /// Nobody else does. Writing in a connected chat enrols no one, unlike a
    /// chat that created its own group (carnet#1167): a sender this refuses
    /// must get no group context, because the context carries consenting
    /// members' training.
    ///
    /// # Errors
    ///
    /// The database error from the membership lookup.
    pub async fn admits_to_connected_chat(
        &self,
        group: &CoachingGroup,
        user_id: Uuid,
    ) -> AppResult<bool> {
        if group.coach_user_id == Some(user_id) {
            return Ok(true);
        }
        Ok(self
            .repo()
            .get_member(&group.id.to_string(), user_id)
            .await?
            .is_some())
    }
}
