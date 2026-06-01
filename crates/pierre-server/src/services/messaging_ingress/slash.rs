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
use pierre_commands::dispatch::{try_dispatch, DispatchOutcome, DispatchRequest};
use pierre_services::channel_error_reply::ChannelErrorReply;

use super::locale::resolve_messaging_locale;
use super::ResolvedSession;

/// Whether a slash-command reply should be redirected to the caller's
/// private chat instead of being posted back into the room it arrived from.
///
/// A slash command is a personal request/response interaction: the caller
/// asks, the bot answers *them*. When the command arrives from a shared room
/// (Telegram group/supergroup, etc.) the answer would otherwise be visible to
/// every other member, leaking the caller's account state. Redirecting the
/// reply to their DM keeps the room clean and the answer private.
///
/// Only DM-capable channels can do this by swapping the recipient: Telegram,
/// `WhatsApp`, and Messenger address individual users, so a reply sent to the
/// sender id lands in a 1:1 chat. Discord and Slack address channels rather
/// than users (see [`super::otp::apply_conversation_recipient`]), so a private
/// reply there needs a different mechanism (ephemeral responses) and is left
/// in the room for now.
#[must_use]
pub const fn redirect_slash_reply_to_dm(
    channel_type: ChannelType,
    is_direct_message: bool,
) -> bool {
    if is_direct_message {
        return false;
    }
    !matches!(channel_type, ChannelType::Discord | ChannelType::Slack)
}

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

/// Resolve and execute slash commands against [`pierre_commands::dispatch`].
///
/// Returns `Some(OutgoingMessage)` if the message was a recognized command,
/// `None` if it should be passed through to the LLM pipeline.
///
/// Delegates parsing + handler execution + analytics to
/// [`pierre_commands::dispatch::try_dispatch`] — the single
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

    // Slash replies in a shared room go to the caller's DM so other members
    // never see the command output (and, via the thread reset below, so the
    // reply doesn't try to route into a group-only forum topic).
    let route_to_dm = redirect_slash_reply_to_dm(channel_type, is_direct_message);
    let reply_target = if route_to_dm {
        sender_id.to_owned()
    } else {
        conversation_id.unwrap_or(sender_id).to_owned()
    };
    let reply_thread = if route_to_dm { None } else { thread_id };

    // Slash dispatch requires both the command-name catalog and the
    // handler-name map. They live on ServerContext alongside the rest of
    // the registries; the dispatcher takes them as explicit refs so the
    // pierre-commands crate stays free of ServerContext.
    let Some(cmd_registry) = resources.common.command_registry.as_ref() else {
        // Registries not configured (test contexts that skip the
        // commands/ catalog). Treat as "not a command" so the caller
        // falls through to the LLM pipeline.
        return None;
    };
    let handler_registry = resources.common.command_handler_registry.as_ref()?;
    let ctx_dyn: Arc<dyn pierre_runtime_context::CommandCtx> =
        Arc::<ServerContext>::clone(resources);

    let outcome = match try_dispatch(DispatchRequest {
        ctx: &ctx_dyn,
        command_registry: cmd_registry,
        command_handler_registry: handler_registry,
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
            let (body, _correlation_id) = e.to_channel_reply(
                &resources.mcp.messaging_strings_registry,
                &locale,
                "command",
            );
            return Some(OutgoingMessage {
                channel_type,
                recipient_id: reply_target,
                content: MessageContent::Text { body },
                turn_id: CanotTurnId::new(),
                reply_to: None,
                thread_id: reply_thread,
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
            thread_id: reply_thread,
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
            } else if response.is_rich_text {
                MessageContent::RichText {
                    body: response.text,
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
                thread_id: reply_thread,
            })
        }
    }
}
