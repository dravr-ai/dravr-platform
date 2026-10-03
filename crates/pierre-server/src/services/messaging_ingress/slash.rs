// ABOUTME: Slash-command entry point for messaging ingress — addressing, /connect, and rendering
// ABOUTME: The dispatch itself is the turn service's; this file only frames the answer for a channel

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![cfg(feature = "client-messaging")]

use std::sync::Arc;

use dravr_canot::channel::MessagingChannel;
use dravr_canot::rich_text::{parse_markdown, render_rich_text};
use dravr_canot::turn::ConversationTurnId as CanotTurnId;
use pierre_core::models::messaging::{CardAction, ChannelType, MessageContent, OutgoingMessage};
use pierre_core::models::TenantId;
use tracing::info;

use pierre_auth::auth::AuthResult;

use crate::mcp::resources::ServerContext;
use pierre_chat_pipeline::stages::command_persistence::is_room_visible;
use pierre_chat_pipeline::{
    dispatch_slash, CommandPersistence, CommandTurn, RenderCapabilities, SlashRequest, TurnOrigin,
};
use pierre_services::channel_error_reply::ChannelErrorReply;
use pierre_services::conversation_forge::conversation_cap_reply;

use super::addressing::reply_recipient;
use super::card_or_rich_text;
use super::connect::build_connect_card_direct;
use super::outbound_send::{send_channel_response, OutboundPersistSpec};
use super::surface::messaging_render_profile;
use super::ResolvedSession;
use pierre_contremaitre::messaging_strings::KEY_NO_PROVIDER_CONNECTED;
use pierre_services::locale::resolve_channel_locale;

/// True when `text` is the `/connect` command (first whitespace-delimited token,
/// case-insensitive). The hosted page is a provider picker, so no argument is
/// needed — `/connect strava` and bare `/connect` both land on the same picker.
fn is_connect_command(text: &str) -> bool {
    text.split_whitespace()
        .next()
        .is_some_and(|tok| tok.eq_ignore_ascii_case("/connect"))
}

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
/// — the set is [`pierre_chat_pipeline::stages::command_persistence::ROOM_VISIBLE_COMMANDS`],
/// shared with the transcript policy so what the room sees is what the room
/// keeps. `None` — the `/connect` card and the unknown-command reply — keeps
/// the private default. Membership/consent/invite commands stay private: they
/// expose one person's data or a redeemable code.
#[must_use]
pub fn slash_reply_should_be_private(is_direct_message: bool, command_name: Option<&str>) -> bool {
    if is_direct_message {
        return false;
    }
    !is_room_visible(command_name)
}

/// The channel message a slash reply threads onto, when it threads at all.
///
/// Only a room-visible reply posted back into the room anchors — onto the
/// command echo, so a body the channel splits keeps its attribution on every
/// part, not only the one carrying the header. A 1:1 DM has no room echo (the
/// conversation IS the private chat), and a privately-redirected reply must
/// not anchor to a room message its recipient may never see — both get `None`.
#[must_use]
pub fn room_reply_thread_anchor(
    is_direct_message: bool,
    command_name: Option<&str>,
    channel_message_id: &str,
) -> Option<String> {
    if is_direct_message || slash_reply_should_be_private(is_direct_message, command_name) {
        return None;
    }
    Some(channel_message_id.to_owned())
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
    /// The assistant `chat_messages` row this reply delivers, when the
    /// surface's transcript policy persisted the command turn — what the
    /// outbound ledger row stamps so an emoji reaction resolves to a message
    /// to rate. `None` for the `/connect` card, the error funnel, and every
    /// command the transcript does not hold.
    pub(super) assistant_message_id: Option<String>,
    /// The agent's welcome, when the command bound an agent into the thread
    /// (carnet#735): sent after [`Self::message`], with its starters listed,
    /// and ledgered against its own chat row.
    pub(super) welcome: Option<WelcomeReply>,
}

/// An agent's welcome as a messaging channel delivers it.
pub(super) struct WelcomeReply {
    /// The outbound message, addressed like the reply it follows.
    pub(super) message: OutgoingMessage,
    /// The welcome's `chat_messages` row.
    pub(super) chat_message_id: String,
}

/// An agent welcome's channel text as the content a channel carries: its
/// inline markdown — the starters are a list — in the channel's dialect.
pub(super) fn welcome_content(channel_text: &str) -> MessageContent {
    MessageContent::RichText {
        body: render_rich_text(&parse_markdown(channel_text)),
    }
}

/// Post a room-visible (or DM) slash reply, then the agent welcome the
/// command produced, in that order, each ledgered by `ledger_spec`.
pub(super) async fn send_visible_reply(
    resources: &ServerContext,
    tenant_id: TenantId,
    channel: &str,
    adapter: &Arc<dyn MessagingChannel>,
    mut reply: SlashReply,
    ledger_spec: impl Fn(Option<String>) -> OutboundPersistSpec,
) {
    let spec = Some(ledger_spec(reply.assistant_message_id.take()));
    let welcome = reply
        .welcome
        .map(|welcome| welcome.into_send(reply.message.turn_id, &ledger_spec));
    send_channel_response(resources, tenant_id, channel, adapter, reply.message, spec).await;
    if let Some((message, spec)) = welcome {
        send_channel_response(resources, tenant_id, channel, adapter, message, Some(spec)).await;
    }
}

impl WelcomeReply {
    /// The welcome as the thread's send: in the command's turn `turn_id`,
    /// ledgered against its own chat row.
    ///
    /// Always a room (or DM) send, never a private one: even when the
    /// command's answer went privately to the caller, the welcome is the
    /// thread's agent opening the thread — its row is in the thread's
    /// transcript and its introduction is recorded there — so the thread
    /// hears it.
    pub(super) fn into_send(
        self,
        turn_id: CanotTurnId,
        ledger_spec: impl Fn(Option<String>) -> OutboundPersistSpec,
    ) -> (OutgoingMessage, OutboundPersistSpec) {
        let Self {
            mut message,
            chat_message_id,
        } = self;
        message.turn_id = turn_id;
        (message, ledger_spec(Some(chat_message_id)))
    }
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

/// Answer a channel message that is a slash command.
///
/// Returns `Some(SlashReply)` when the text was a command, `None` when it
/// should be passed through to a coaching turn.
///
/// Parsing, handler execution and analytics belong to
/// [`pierre_chat_pipeline::dispatch_slash`] — the one dispatch every chat
/// surface reaches a command through. What is messaging-specific stays here:
/// resolving auth, tenant and locale from the channel link, minting the
/// `/connect` card (which needs in-process token minting), addressing the
/// answer to the room it came from, and choosing a card, rich text or plain
/// text for it.
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
    // shared room under the channel tenant — each member holding their own
    // conversation row there — and `persist_single_message` recomputes exactly
    // this expression for the reads and writes it performs. Commands dispatched
    // below resolve `session.conversation` (and the coaching group it names)
    // with it; `user_tenant` stays the tenant for the caller's own data.
    // Without the split, a member of a shared bot's group looks the
    // conversation up under a tenant that owns no such row.
    let conversation_tenant = if is_direct_message {
        user_tenant
    } else {
        webhook_tenant_id
    };
    let locale = resolve_channel_locale(
        resources.common.repos.messaging.as_ref(),
        resources.common.repos.users.as_ref(),
        user_tenant,
        channel,
        sender_id,
        Some(user_uuid),
    )
    .await;

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
                assistant_message_id: None,
                welcome: None,
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
            assistant_message_id: None,
            welcome: None,
        });
    }

    // The surface's capabilities, resolved from the real transport: whether an
    // action renders as a button or as an autolinked line. The character
    // ceiling is not a handler's concern — a reply past it is split into
    // ordered messages by `send_channel_response`, not trimmed here.
    let profile = messaging_render_profile(channel_type, &locale);

    // The dispatch itself belongs to the turn service, which every chat
    // surface reaches it through, so a command behaves the same wherever it is
    // typed. What is left here is a channel's own: who the answer is addressed
    // to, and whether its controls render as buttons.
    let command = match dispatch_slash(
        &resources.chat_pipeline_context(),
        &SlashRequest {
            user_id: user_uuid,
            tenant_id: user_tenant,
            conversation_id: &session.conversation,
            conversation_tenant_id: conversation_tenant,
            channel_type: channel,
            locale: &locale,
            is_direct_message,
            // A DM with the bot is the athlete's one thread, so a `/group`
            // command typed there means the group they are in.
            ambient_group_fallback: true,
            // A DM keeps every command turn, as the channel itself does. A
            // shared room keeps only what it saw: the replies announced in the
            // room, never the ones delivered privately to the caller.
            persistence: if is_direct_message {
                CommandPersistence::Always
            } else {
                CommandPersistence::RoomVisibleOnly
            },
            sender_id: Some(sender_id),
            text,
            // The athlete typed it into the channel.
            origin: TurnOrigin::Athlete,
        },
    )
    .await
    {
        Ok(Some(command)) => command,
        Ok(None) => return None,
        Err(e) => {
            // Single centralized funnel: logs the full error with a
            // correlation id and returns a channel-safe body. Never
            // interpolate the raw error into the reply text by hand —
            // the grep gate in architectural-validation.sh blocks it.
            let strings = &resources.mcp.messaging_strings_registry;
            // A command refused at the conversation cap names the cap in the
            // athlete's words, as `/reset` does.
            let body = conversation_cap_reply(&e, strings, &locale)
                .unwrap_or_else(|| e.to_channel_reply(strings, &locale, "command").0);
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
                assistant_message_id: None,
                welcome: None,
            });
        }
    };

    if let Some(name) = command.command_name.as_deref() {
        info!(
            command = %name,
            user_id = %session.user_id,
            channel = %channel,
            "Slash command reply addressed"
        );
    }
    let command_name = command.command_name.clone();
    let assistant_message_id = command
        .persisted
        .as_ref()
        .map(|p| p.assistant_message.id.clone());
    let welcome = command.welcome.as_ref().map(|posted| WelcomeReply {
        message: OutgoingMessage {
            channel_type,
            recipient_id: reply_target.clone(),
            content: welcome_content(&posted.channel_text),
            turn_id: CanotTurnId::new(),
            reply_to: None,
            thread_id: thread_id.clone(),
        },
        chat_message_id: posted.message.id.clone(),
    });
    Some(SlashReply {
        message: OutgoingMessage {
            channel_type,
            recipient_id: reply_target,
            content: command_content(&profile.render, command),
            turn_id: CanotTurnId::new(),
            reply_to: None,
            thread_id,
        },
        command_name,
        assistant_message_id,
        welcome,
    })
}

/// Frame a command's answer as the content this channel will carry.
///
/// A card where the handler asked for one and the channel draws controls; the
/// channel's rich-text dialect, converted from the handler's inline markdown,
/// where it asked for formatting; plain text otherwise. The choice is the
/// surface's capability, never its name — a channel without buttons gets the
/// same answer with its actions written out as text by [`card_or_rich_text`].
fn command_content(render: &RenderCapabilities, command: CommandTurn) -> MessageContent {
    let CommandTurn {
        text,
        is_rich_text,
        card_title,
        actions,
        ..
    } = command;
    if let Some(title) = card_title {
        return card_or_rich_text(
            render,
            title,
            text,
            actions
                .into_iter()
                .map(|action| CardAction {
                    label: action.label,
                    action_type: action.kind.as_str().to_owned(),
                    value: action.value,
                })
                .collect(),
        );
    }
    if is_rich_text {
        return MessageContent::RichText {
            body: render_rich_text(&parse_markdown(&text)),
        };
    }
    MessageContent::Text { body: text }
}
