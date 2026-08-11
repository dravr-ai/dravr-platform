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
use pierre_tool_runtime::runtime::ToolRuntime;

use super::addressing::reply_recipient;
use super::card_or_rich_text;
use super::connect::build_connect_card_direct;
use super::locale::resolve_messaging_locale;
use super::ResolvedSession;
use pierre_contremaitre::messaging_strings::KEY_NO_PROVIDER_CONNECTED;

/// True when `text` is the `/connect` command (first whitespace-delimited token,
/// case-insensitive). The hosted page is a provider picker, so no argument is
/// needed — `/connect strava` and bare `/connect` both land on the same picker.
fn is_connect_command(text: &str) -> bool {
    text.split_whitespace()
        .next()
        .is_some_and(|tok| tok.eq_ignore_ascii_case("/connect"))
}

/// Commands whose reply belongs to the **room**, not to the caller alone.
///
/// The default is private (see [`slash_reply_should_be_private`]) because a
/// slash reply usually carries the caller's own account state. These two are
/// different in kind: they change a setting every member then experiences, so
/// announcing the change in the room is the point — a member who watches the
/// coach fall silent after someone ran `/group respond mentions` privately has
/// no way to know why. Membership/consent/invite commands stay private: they
/// expose one person's data or a redeemable code.
///
/// Entries are the command-definition `name:` values (hyphenated ids like
/// `group-respond`), which is what `DispatchOutcome::Executed.command_name`
/// carries — canot's matcher returns `def.name`, not the spaced `/group
/// respond` trigger. Spelling these with a space silently matches nothing and
/// the reply keeps going private; `group_setting_changes_are_announced_in_the_room`
/// pins the real names against the loaded `commands/` catalog.
const ROOM_VISIBLE_COMMANDS: [&str; 2] = ["group-respond", "group-coach"];

/// Whether a slash-command reply should be delivered privately to the caller
/// rather than posted back into the room it arrived from.
///
/// A slash command is normally a personal request/response interaction: the
/// caller asks, the bot answers *them*. Any non-DM context is a shared room
/// where the answer (and the caller's account state) would otherwise be visible
/// to every other member, so the reply is delivered privately on **every**
/// channel. The per-channel mechanism lives in canot's `send_private_reply` — a
/// 1:1 DM for Telegram/`WhatsApp`/Messenger, an ephemeral message for Slack, an
/// opened DM channel for Discord. A 1:1 DM is already private, so nothing to
/// redirect.
///
/// `command_name` opts a group-wide *setting change* out of that redirection
/// (see [`ROOM_VISIBLE_COMMANDS`]). `None` — the `/connect` card and the
/// unknown-command reply — keeps the private default.
#[must_use]
pub fn slash_reply_should_be_private(is_direct_message: bool, command_name: Option<&str>) -> bool {
    if is_direct_message {
        return false;
    }
    !command_name.is_some_and(|name| ROOM_VISIBLE_COMMANDS.contains(&name))
}

/// A slash-command reply plus the identity of the command that produced it.
///
/// The command name is what decides room-vs-private delivery
/// ([`slash_reply_should_be_private`]), so it has to survive the trip back to
/// `persist_single_message` instead of being dropped at the dispatch boundary.
pub(super) struct SlashReply {
    /// The outbound message, already addressed to the originating room.
    pub(super) message: OutgoingMessage,
    /// Canonical command name when a registered handler ran; `None` for the
    /// `/connect` card, the unknown-command body, and the error funnel — all of
    /// which keep the private default.
    pub(super) command_name: Option<String>,
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
    /// Tenant that owns the webhook this message arrived on — the fallback when
    /// `auth_result.active_tenant_id` is absent. Passed in rather than
    /// recomputed so this path and `persist_single_message`'s `user_tenant_id`
    /// cannot disagree about which tenant a command runs under.
    pub webhook_tenant_id: TenantId,
}

/// Resolve and execute slash commands against [`pierre_commands::dispatch`].
///
/// Returns `Some(SlashReply)` if the message was a recognized command,
/// `None` if it should be passed through to the LLM pipeline.
///
/// Delegates parsing + handler execution + analytics to
/// [`pierre_commands::dispatch::try_dispatch`] — the single
/// authority for every chat surface. This function's remaining job is
/// messaging-specific: resolving auth/tenant/locale from the channel link
/// and wrapping the outcome into a [`SlashReply`] for the renderer.
pub(super) async fn try_handle_slash_command(
    resources: &Arc<ServerContext>,
    ctx: SlashCommandContext<'_>,
) -> Option<SlashReply> {
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
        webhook_tenant_id,
    } = ctx;

    // Fast path: not a command. Avoids any auth/tenant lookups.
    if !text.trim().starts_with('/') {
        return None;
    }

    let user_uuid = auth_result.user_id;
    // Tenant comes straight from AuthResult — same path as the JWT middleware,
    // falling back to the webhook tenant exactly as `persist_single_message`
    // does. It used to fall back to `TenantId::from_uuid(user_uuid)` instead: a
    // user with no tenant membership then ran commands under a tenant id that
    // owns no rows, so a conversation-scoped write (e.g. `/pillars` activating
    // onboarding) matched nothing while the caller saw a success reply.
    let user_tenant = auth_result
        .active_tenant_id
        .map_or(webhook_tenant_id, TenantId::from_uuid);
    // The tenant that owns this turn's session, conversation and messages.
    // `resolve_linked_session` files a DM under the user's own tenant and a
    // shared room under the channel tenant so every member reads one
    // conversation, and `persist_single_message` recomputes exactly this
    // expression for the reads and writes it performs. Commands dispatched
    // below resolve `session.conversation` (and the coaching group it names)
    // with it; `user_tenant` stays the tenant for the caller's own data.
    // Without the split, a member of a shared bot's group looks the
    // conversation up under a tenant that owns no such row.
    let conversation_tenant = if is_direct_message {
        user_tenant
    } else {
        webhook_tenant_id
    };
    let locale =
        resolve_messaging_locale(resources, user_tenant, user_uuid, channel, sender_id).await;

    // Address the reply to the room/conversation it came from. When that's a
    // shared room, `persist_single_message` redelivers it privately to the
    // caller via `send_private_reply` (and deletes the echo); for a 1:1 DM the
    // conversation is already the private chat.
    let reply_target = reply_recipient(conversation_id, sender_id).to_owned();

    // `/connect` is handled here (not in the transport-agnostic command crate)
    // because building the connect link needs in-process token minting
    // (ServerContext + the DM flag). In a direct message the user gets a tappable
    // connect Card; in a group (where a user-scoped link must never be posted) or
    // on a mint failure, fall back to the plain web providers page.
    if is_connect_command(text) {
        if let Some(card) = build_connect_card_direct(
            resources,
            user_uuid,
            user_tenant.as_uuid(),
            channel,
            channel_type,
            &reply_target,
            thread_id.clone(),
            is_direct_message,
            &locale,
        )
        .await
        {
            return Some(SlashReply {
                message: card,
                command_name: None,
            });
        }
        let web_url = format!(
            "{}/providers",
            resources
                .common
                .config
                .frontend_url
                .as_deref()
                .unwrap_or(&resources.common.config.base_url)
        );
        let body = resources.mcp.messaging_strings_registry.render(
            KEY_NO_PROVIDER_CONNECTED,
            &locale,
            &[&web_url],
        );
        return Some(SlashReply {
            message: OutgoingMessage {
                channel_type,
                recipient_id: reply_target,
                content: MessageContent::Text { body },
                turn_id: CanotTurnId::new(),
                reply_to: None,
                thread_id,
            },
            command_name: None,
        });
    }

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
    let tool_runtime: Arc<dyn ToolRuntime> = Arc::<ServerContext>::clone(resources);

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
        conversation_tenant_id: conversation_tenant,
        sender_id: Some(sender_id),
        text,
        tool_runtime: &tool_runtime,
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
            return Some(SlashReply {
                message: OutgoingMessage {
                    channel_type,
                    recipient_id: reply_target,
                    content: MessageContent::Text { body },
                    turn_id: CanotTurnId::new(),
                    reply_to: None,
                    thread_id,
                },
                command_name: None,
            });
        }
    };

    match outcome {
        DispatchOutcome::NotACommand => None,
        DispatchOutcome::UnknownCommand { body } => Some(SlashReply {
            message: OutgoingMessage {
                channel_type,
                recipient_id: reply_target,
                content: MessageContent::Text { body },
                turn_id: CanotTurnId::new(),
                reply_to: None,
                thread_id,
            },
            command_name: None,
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
                card_or_rich_text(
                    channel_type,
                    response.card_title.unwrap_or_default(),
                    response.text,
                    response
                        .actions
                        .into_iter()
                        .map(|a| CardAction {
                            label: a.label,
                            action_type: a.action_type,
                            value: a.value,
                        })
                        .collect(),
                )
            } else if response.is_rich_text {
                MessageContent::RichText {
                    body: response.text,
                }
            } else {
                MessageContent::Text {
                    body: response.text,
                }
            };
            Some(SlashReply {
                message: OutgoingMessage {
                    channel_type,
                    recipient_id: reply_target,
                    content,
                    turn_id: CanotTurnId::new(),
                    reply_to: None,
                    thread_id,
                },
                command_name: Some(command_name),
            })
        }
    }
}
