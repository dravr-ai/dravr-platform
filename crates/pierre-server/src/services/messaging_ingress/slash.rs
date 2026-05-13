// ABOUTME: Slash-command dispatch entry point for messaging ingress
// ABOUTME: Wraps services::commands::dispatch::try_dispatch with channel-link auth/locale resolution

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![cfg(feature = "client-messaging")]

use std::sync::Arc;

use pierre_core::models::messaging::{CardAction, ChannelType, MessageContent, OutgoingMessage};
use pierre_core::models::TenantId;
use pierre_messaging::turn::ConversationTurnId as CanotTurnId;
use tracing::info;

use pierre_auth::auth::AuthResult;

use crate::mcp::resources::ServerContext;
use crate::services::channel_error_reply::ChannelErrorReply;
use crate::services::commands::dispatch::{try_dispatch, DispatchOutcome, DispatchRequest};

use super::locale::resolve_messaging_locale;
use super::ResolvedSession;

/// Bundled inputs for [`try_handle_slash_command`]. Combines the channel
/// identifiers, the message text, and the per-message metadata so the
/// dispatcher doesn't need an eight-arg positional signature.
pub(super) struct SlashCommandContext<'a> {
    /// Channel identifier (`"slack"`, `"telegram"`, etc.).
    pub channel: &'a str,
    /// Strongly-typed channel kind for downstream routing.
    pub channel_type: ChannelType,
    /// Authenticated principal — same source the JWT middleware emits,
    /// produced by `authenticate_channel` at the top of
    /// `persist_single_message`. Carries `active_tenant_id` for tool execution.
    pub auth_result: &'a AuthResult,
    /// Resolved Pierre session (user/tenant + conversation binding).
    pub session: &'a ResolvedSession,
    /// Inbound message text.
    pub text: &'a str,
    /// Channel-native sender identifier.
    pub sender_id: &'a str,
    /// Pierre conversation id when one is already bound.
    pub conversation_id: Option<&'a str>,
    /// Forum-topic thread identifier for channels that expose them.
    pub thread_id: Option<String>,
    /// True when the inbound message is a 1:1 DM (vs. a group room).
    pub is_direct_message: bool,
}

/// Resolve and execute slash commands against [`crate::services::commands::dispatch`].
///
/// Returns `Some(OutgoingMessage)` if the message was a recognized command,
/// `None` if it should be passed through to the LLM pipeline.
///
/// Delegates parsing + handler execution + analytics to
/// [`crate::services::commands::dispatch::try_dispatch`] — the single
/// authority for every chat surface. This function's remaining job is
/// messaging-specific: resolving auth/tenant/locale from the channel link
/// and wrapping the outcome into an [`OutgoingMessage`] for the renderer.
pub(super) async fn try_handle_slash_command(
    resources: &Arc<ServerContext>,
    ctx: SlashCommandContext<'_>,
) -> Option<OutgoingMessage> {
    let SlashCommandContext {
        channel,
        channel_type,
        auth_result,
        session,
        text,
        sender_id,
        conversation_id,
        thread_id,
        is_direct_message,
    } = ctx;

    // Fast path: not a command. Avoids any auth/tenant lookups.
    if !text.trim().starts_with('/') {
        return None;
    }

    let user_uuid = auth_result.user_id;
    // Tenant comes straight from AuthResult — same path as the JWT middleware.
    let user_tenant = auth_result
        .active_tenant_id
        .map_or_else(|| TenantId::from_uuid(user_uuid), TenantId::from_uuid);
    let locale =
        resolve_messaging_locale(resources, user_tenant, user_uuid, channel, sender_id).await;
    let reply_target = conversation_id.unwrap_or(sender_id).to_owned();

    let outcome = match try_dispatch(DispatchRequest {
        resources,
        user_id: user_uuid,
        tenant_id: user_tenant,
        channel_type: channel,
        locale: &locale,
        is_direct_message,
        conversation_id: Some(&session.conversation),
        text,
    })
    .await
    {
        Ok(outcome) => outcome,
        Err(e) => {
            // Single centralized funnel: logs the full error with a
            // correlation id and returns a channel-safe body. Never
            // interpolate the raw error into the reply text by hand —
            // the grep gate in architectural-validation.sh blocks it.
            let (body, _correlation_id) = e.to_channel_reply(resources, &locale, "command");
            return Some(OutgoingMessage {
                channel_type,
                recipient_id: reply_target,
                content: MessageContent::Text { body },
                turn_id: CanotTurnId::new(),
                reply_to: None,
                thread_id,
            });
        }
    };

    match outcome {
        DispatchOutcome::NotACommand => None,
        DispatchOutcome::UnknownCommand { body } => Some(OutgoingMessage {
            channel_type,
            recipient_id: reply_target,
            content: MessageContent::Text { body },
            turn_id: CanotTurnId::new(),
            reply_to: None,
            thread_id,
        }),
        DispatchOutcome::Executed {
            command_name,
            response,
        } => {
            info!(
                command = %command_name,
                user_id = %session.user_id,
                channel = %channel,
                "Slash command executed"
            );
            let content = if response.is_card() {
                MessageContent::Card {
                    title: response.card_title.unwrap_or_default(),
                    body: response.text,
                    actions: response
                        .actions
                        .into_iter()
                        .map(|a| CardAction {
                            label: a.label,
                            action_type: a.action_type,
                            value: a.value,
                        })
                        .collect(),
                }
            } else {
                MessageContent::Text {
                    body: response.text,
                }
            };
            Some(OutgoingMessage {
                channel_type,
                recipient_id: reply_target,
                content,
                turn_id: CanotTurnId::new(),
                reply_to: None,
                thread_id,
            })
        }
    }
}
