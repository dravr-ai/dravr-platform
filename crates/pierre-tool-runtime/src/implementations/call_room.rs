// ABOUTME: Who reads a tool call's reply: the caller alone, a group's room, or a chat not confirmed private
// ABOUTME: The one room classifier every tool, listing, chat route and history replay that keeps data out of a room asks
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Call room, for a tool call
//!
//! The classifier is [`CallRoom`], in `pierre_services::call_room` so the
//! transcript reads ask the same one. A tool call's room is read here, off
//! its [`ToolExecutionContext`]: whether it names a conversation, the row
//! read for the caller, and whether the turn is a direct message.

use pierre_core::errors::AppResult;
use pierre_core::models::{ConversationRecord, TenantId};
use pierre_database::RepositoryRegistry;
use uuid::Uuid;

pub(crate) use pierre_services::call_room::CallRoom;

use crate::context::ToolExecutionContext;

/// The room a call from `context` was made in, when its conversation's row,
/// read for the caller, is `conversation` (`None` when it does not resolve
/// for them).
///
/// A call that does not say who reads it is not taken for private, so a
/// context built outside a turn fails closed unless its builder states the
/// audience.
pub(super) async fn of(
    context: &ToolExecutionContext,
    conversation: Option<&ConversationRecord>,
) -> CallRoom {
    CallRoom::read(
        context.resources.repos(),
        context.conversation_id.is_some(),
        conversation,
        context.turn_direct_message,
    )
    .await
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
    turn_room(
        context.resources.repos(),
        TurnAudience {
            caller: context.user_id,
            conversation_id: context.conversation_id.as_deref(),
            conversation_tenant: context.conversation_ref().map(|(_, tenant)| tenant),
            caller_tenant: context.tenant_id.map(TenantId::from_uuid),
            direct_message: context.turn_direct_message,
        },
    )
    .await
}

/// What a call knows about who reads its reply, for a caller that holds no
/// [`ToolExecutionContext`] yet (the dispatch chokepoint resolving whose
/// data a call reads).
#[derive(Debug, Clone, Copy)]
pub(crate) struct TurnAudience<'turn> {
    /// The caller.
    pub caller: Uuid,
    /// The originating conversation, for a call from a chat turn.
    pub conversation_id: Option<&'turn str>,
    /// The tenant the conversation's row lives under, when the turn says.
    pub conversation_tenant: Option<TenantId>,
    /// The caller's own tenant.
    pub caller_tenant: Option<TenantId>,
    /// Whether the turn is a direct message, when the turn says.
    pub direct_message: Option<bool>,
}

/// The room a call described by `audience` was made in: `call_room`'s
/// answer, from the same row lookup and the same classifier.
///
/// # Errors
///
/// Returns the repository error from the conversation lookup.
pub(crate) async fn turn_room(
    repos: &RepositoryRegistry,
    audience: TurnAudience<'_>,
) -> AppResult<CallRoom> {
    let Some(conversation_id) = audience.conversation_id else {
        return Ok(CallRoom::Private);
    };
    let conversation = match audience.conversation_tenant.or(audience.caller_tenant) {
        Some(tenant) => {
            repos
                .chat
                .get_conversation(conversation_id, &audience.caller.to_string(), tenant)
                .await?
        }
        None => None,
    };
    Ok(CallRoom::read(repos, true, conversation.as_ref(), audience.direct_message).await)
}
