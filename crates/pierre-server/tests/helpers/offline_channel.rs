// ABOUTME: A real channel adapter with only its outbound sends captured, for route-driven tests
// ABOUTME: Keeps production inbound parsing while removing the live POST that made CI runs flaky
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
#![allow(dead_code)]

//! Offline outbound for tests that drive the real webhook route.
//!
//! A test that posts to `/api/messaging/webhook/{channel}` gets whatever
//! adapter the tenant's stored config builds — in production a live transport.
//! `group_transcript_test` seeded a fixture bot token and therefore attempted a
//! real POST to `api.telegram.org` on every turn: answered `401` in ~100 ms on a
//! developer's machine, and left hanging on a CI runner that cannot reach
//! Telegram, which on 2026-09-01 consumed the whole 10 s budget those tests wait
//! on (run 33462729445).
//!
//! Wrapping rather than replacing the adapter is the point. Signature
//! verification, payload parsing and rendering stay the production ones, so the
//! test still exercises our ingress; only the egress is removed.

use std::collections::HashSet;
use std::sync::{Arc, LazyLock, Mutex};

use async_trait::async_trait;
use chrono::Utc;
use dravr_canot::channel::{dm_redirect, MessagingChannel};
use dravr_canot::channels::telegram::transport::TelegramTransport;
use dravr_canot::error::{MessagingError, MessagingResult};
use dravr_canot::factory::create_adapter_from_config_with_api_base;
use dravr_canot::turn::ConversationTurnId;
use http::HeaderMap;
use pierre_core::models::messaging::{
    ChannelConfig, ChannelType, DeliveryReceipt, DeliveryStatus, InboundReaction, IncomingMessage,
    OutgoingMessage, SenderIdentityChange,
};
use pierre_services::channel_adapters::ChannelAdapterFactory;
use serde_json::Value;

/// Where an offline factory sends the status placeholder, the calls of the
/// adapters it builds and the linking route's destination lookup: a
/// loopback port nothing listens on (1 is reserved for TCPMUX, and no test
/// binds it).
///
/// The connection is refused at once, so the turn takes the path it took when
/// the real host answered a fixture token with `401`: the placeholder does
/// not open and the reply goes out as a send, which the factory captures. A
/// test that needs the placeholder to open supplies its own stand-in, as
/// `slack_answer_placement_test` does.
pub(crate) const REFUSED_STATUS_API: &str = "http://127.0.0.1:1";

/// Where an offline factory sends the Discord Gateway client: the same
/// refused loopback port, as a WebSocket URL.
pub(crate) const REFUSED_GATEWAY: &str = "ws://127.0.0.1:1";

/// Telegram bot ids whose username a test or the offline factory has named.
static NAMED_BOTS: LazyLock<Mutex<HashSet<i64>>> = LazyLock::new(Mutex::default);

/// Tell canot the username of the fixture bot `bot_id`.
///
/// The Telegram adapter learns its bot's username from `getMe` the first
/// time a message could mention the bot, and keeps it for the process. That
/// call is made inside the inbound parse the offline adapter leaves
/// untouched; the offline factory builds the adapter on a base of its own,
/// so the call is refused and never reaches Telegram, but a refused `getMe`
/// leaves the adapter without a username and mention detection degraded.
/// So the username is answered beforehand: a test whose messages mention
/// its bot names it here, and the offline factory names every other bot it
/// is asked to build.
pub(crate) fn name_fixture_bot(bot_id: i64, username: &str) {
    NAMED_BOTS.lock().unwrap().insert(bot_id);
    // The transport is not kept: building it is what fills canot's cache.
    let _seed = TelegramTransport::with_bot_identity("unused-secret".to_owned(), bot_id, username);
}

/// Name the bot a stored Telegram config belongs to, unless a test already
/// did, so that parsing its inbound messages has a username to match and
/// asks no `getMe`.
fn name_unnamed_bot(config: &Value) {
    let Some(bot_id) = config
        .get("bot_token")
        .and_then(Value::as_str)
        .and_then(|token| token.split(':').next())
        .and_then(|id| id.parse::<i64>().ok())
    else {
        return;
    };
    if !NAMED_BOTS.lock().unwrap().contains(&bot_id) {
        name_fixture_bot(bot_id, &format!("offline_fixture_bot_{bot_id}"));
    }
}

/// A real channel adapter with its outbound sends captured instead of sent.
///
/// Inbound is delegated verbatim — signature verification, payload parsing and
/// rendering all run the production adapter — so a route-driven test still
/// exercises our ingress rather than a stand-in for it. The adapter's own
/// calls out of the process are intercepted. Three are recorded as the kind
/// of call each is ([`Egress`]): `send`, a private reply on the channels
/// that have a private mechanism of their own, and a deletion on the
/// channels that can delete. `send_raw`, the retry worker's re-send of a
/// stored payload, is answered with a receipt and not recorded, refusing
/// adapter or not. The ones that are not the wrapper's to intercept, the
/// status placeholder and what the inbound parse calls (`getMe`,
/// `answerCallbackQuery`), are kept off the network by
/// [`OfflineSendAdapters`], which builds the adapter and names the
/// placeholder's base on an origin of its own.
///
/// This exists because `group_transcript_test` seeds a fixture bot token and
/// drives the real Telegram webhook route, so every turn attempted a live POST
/// to `api.telegram.org`. A developer's machine gets a 401 in ~100 ms and the
/// turn finishes; a CI runner that cannot reach Telegram hangs instead, and on
/// 2026-09-01 that consumed the entire 10 s budget the transcript tests wait on.
/// The tests were passing or failing on a third party's reachability.
pub(crate) struct OfflineSendChannel {
    /// The production adapter every inbound method delegates to.
    inner: Arc<dyn MessagingChannel>,
    /// Shared with the factory that built this adapter, so a test holding the
    /// factory reads every call across every adapter the ingress constructed.
    sent: EgressLog,
    /// Whether `send` and a private reply are refused instead of captured:
    /// the channel that rejected a fixture token, without the channel.
    refuses_sends: bool,
}

/// One call out of the process an offline adapter took in place of the wire.
#[derive(Clone, Debug)]
pub(crate) enum Egress {
    /// `send`: a message to the room or chat it names.
    Send(OutgoingMessage),
    /// `send_private_reply` on a channel with a private mechanism of its own
    /// (Slack's in-channel ephemeral, Discord's opened DM): the message as
    /// the ingress addressed it, and the user it is for.
    PrivateReply {
        message: OutgoingMessage,
        recipient_user_id: String,
    },
    /// `delete_message` on a channel that can delete (Telegram, Discord).
    Delete {
        conversation_id: String,
        channel_message_id: String,
    },
}

/// The calls captured in place of delivery, newest last.
type EgressLog = Arc<Mutex<Vec<Egress>>>;

impl OfflineSendChannel {
    /// Wrap a real adapter, capturing its calls out of the process into `sent`.
    fn wrapping(inner: Arc<dyn MessagingChannel>, sent: EgressLog, refuses_sends: bool) -> Self {
        Self {
            inner,
            sent,
            refuses_sends,
        }
    }

    /// The delivery error a refusing adapter answers every send with.
    fn refusal(&self) -> MessagingError {
        MessagingError::DeliveryFailed {
            channel: self.inner.channel_type().to_string(),
            reason: "the offline channel refuses every send".to_owned(),
            retryable: true,
        }
    }

    /// The receipt a captured call answers with: sent, with no channel id.
    fn receipt(turn_id: ConversationTurnId) -> DeliveryReceipt {
        DeliveryReceipt {
            message_id: "offline-send".to_owned(),
            channel_message_id: None,
            status: DeliveryStatus::Sent,
            timestamp: Utc::now(),
            turn_id,
        }
    }
}

#[async_trait]
impl MessagingChannel for OfflineSendChannel {
    fn channel_type(&self) -> ChannelType {
        self.inner.channel_type()
    }

    fn verify_signature(&self, headers: &HeaderMap, body: &[u8]) -> MessagingResult<()> {
        self.inner.verify_signature(headers, body)
    }

    async fn receive(
        &self,
        headers: &HeaderMap,
        body: &[u8],
    ) -> MessagingResult<Vec<IncomingMessage>> {
        self.inner.receive(headers, body).await
    }

    async fn receive_reactions(
        &self,
        headers: &HeaderMap,
        body: &[u8],
    ) -> MessagingResult<Vec<InboundReaction>> {
        self.inner.receive_reactions(headers, body).await
    }

    async fn receive_identity_changes(
        &self,
        headers: &HeaderMap,
        body: &[u8],
    ) -> MessagingResult<Vec<SenderIdentityChange>> {
        self.inner.receive_identity_changes(headers, body).await
    }

    fn render(&self, msg: &OutgoingMessage) -> MessagingResult<Value> {
        self.inner.render(msg)
    }

    async fn send(
        &self,
        msg: &OutgoingMessage,
        _config: &ChannelConfig,
    ) -> MessagingResult<DeliveryReceipt> {
        if self.refuses_sends {
            return Err(self.refusal());
        }
        self.sent.lock().unwrap().push(Egress::Send(msg.clone()));
        Ok(Self::receipt(msg.turn_id))
    }

    async fn send_raw(
        &self,
        _payload: &Value,
        turn_id: ConversationTurnId,
        _config: &ChannelConfig,
    ) -> MessagingResult<DeliveryReceipt> {
        Ok(Self::receipt(turn_id))
    }

    /// Slack answers privately with an ephemeral inside the room and Discord
    /// by opening a DM: calls of their own, recorded as a private reply.
    /// Every other channel's private reply is canot's default, the message
    /// re-addressed to the user and handed to `send`, so it is recorded as
    /// the send it is. Never delegated: the real adapter would post it.
    async fn send_private_reply(
        &self,
        msg: &OutgoingMessage,
        recipient_user_id: &str,
        config: &ChannelConfig,
    ) -> MessagingResult<DeliveryReceipt> {
        match self.inner.channel_type() {
            ChannelType::Slack | ChannelType::Discord => {
                if self.refuses_sends {
                    return Err(self.refusal());
                }
                self.sent.lock().unwrap().push(Egress::PrivateReply {
                    message: msg.clone(),
                    recipient_user_id: recipient_user_id.to_owned(),
                });
                Ok(Self::receipt(msg.turn_id))
            }
            ChannelType::Telegram | ChannelType::WhatsApp | ChannelType::Messenger => {
                self.send(&dm_redirect(msg, recipient_user_id), config)
                    .await
            }
        }
    }

    /// Telegram and Discord can delete a message, so the deletion is
    /// recorded and answered as done. Slack, `WhatsApp` and Messenger keep
    /// canot's default, which reports that the channel has no deletion API:
    /// what the ingress does next depends on that answer. Never delegated:
    /// the real adapter would call the channel.
    async fn delete_message(
        &self,
        conversation_id: &str,
        channel_message_id: &str,
        config: &ChannelConfig,
    ) -> MessagingResult<()> {
        match self.inner.channel_type() {
            ChannelType::Telegram | ChannelType::Discord => {
                self.sent.lock().unwrap().push(Egress::Delete {
                    conversation_id: conversation_id.to_owned(),
                    channel_message_id: channel_message_id.to_owned(),
                });
                Ok(())
            }
            ChannelType::Slack | ChannelType::WhatsApp | ChannelType::Messenger => {
                Err(MessagingError::OperationNotSupported {
                    channel: format!("{:?}", config.channel_type),
                    operation: "message deletion".to_owned(),
                })
            }
        }
    }
}

/// A [`ChannelAdapterFactory`] that builds the production adapter and wraps it
/// so its sends stay in the process.
///
/// [`ChannelAdapterFactory`]: pierre_services::channel_adapters::ChannelAdapterFactory
/// Holds the shared [`EgressLog`] every adapter it builds writes into, so a test
/// can still assert the turn attempted delivery — the coverage the live
/// transport used to carry implicitly, and which taking the network out would
/// otherwise remove silently.
pub(crate) struct OfflineSendAdapters {
    sent: EgressLog,
    refuses_sends: bool,
    /// The base every adapter is built on and the status placeholder is
    /// posted to: [`REFUSED_STATUS_API`] unless the test names a stand-in.
    api_base: String,
}

impl Default for OfflineSendAdapters {
    fn default() -> Self {
        Self {
            sent: EgressLog::default(),
            refuses_sends: false,
            api_base: REFUSED_STATUS_API.to_owned(),
        }
    }
}

impl OfflineSendAdapters {
    /// A factory whose adapters call `api_base`, a stand-in the test reads
    /// back, for what the wrapper does not capture: the status placeholder
    /// and the calls of the inbound parse (Telegram's `answerCallbackQuery`
    /// and `getMe`). Sends are captured as on any offline factory.
    pub(crate) fn on_api_base(api_base: &str) -> Self {
        Self {
            api_base: api_base.to_owned(),
            ..Self::default()
        }
    }

    /// A factory whose adapters refuse every `send` and every private reply
    /// with a delivery error, as the real host refused a fixture token, so a
    /// route-driven test reaches what the ingress does with a reply it could
    /// not deliver. `send_raw` and a deletion answer as on any offline adapter.
    pub(crate) fn refusing_sends() -> Self {
        Self {
            refuses_sends: true,
            ..Self::default()
        }
    }

    /// Every call out of the process an adapter of this factory took, in order.
    pub(crate) fn egress(&self) -> Vec<Egress> {
        self.sent.lock().unwrap().clone()
    }

    /// Every message handed to a channel's `send`, in order. A private reply
    /// on a channel whose private mechanism is a re-addressed send (Telegram,
    /// `WhatsApp`, Messenger) is one of them, addressed to the user.
    pub(crate) fn sends(&self) -> Vec<OutgoingMessage> {
        self.egress()
            .into_iter()
            .filter_map(|call| match call {
                Egress::Send(message) => Some(message),
                Egress::PrivateReply { .. } | Egress::Delete { .. } => None,
            })
            .collect()
    }

    /// Every private reply a channel with a private mechanism of its own was
    /// asked for, as `(message, recipient user id)`, in order.
    pub(crate) fn private_replies(&self) -> Vec<(OutgoingMessage, String)> {
        self.egress()
            .into_iter()
            .filter_map(|call| match call {
                Egress::PrivateReply {
                    message,
                    recipient_user_id,
                } => Some((message, recipient_user_id)),
                Egress::Send(_) | Egress::Delete { .. } => None,
            })
            .collect()
    }

    /// Every deletion a channel that can delete was asked for, as
    /// `(conversation id, channel message id)`, in order.
    pub(crate) fn deletes(&self) -> Vec<(String, String)> {
        self.egress()
            .into_iter()
            .filter_map(|call| match call {
                Egress::Delete {
                    conversation_id,
                    channel_message_id,
                } => Some((conversation_id, channel_message_id)),
                Egress::Send(_) | Egress::PrivateReply { .. } => None,
            })
            .collect()
    }
}

impl ChannelAdapterFactory for OfflineSendAdapters {
    fn build(
        &self,
        channel_type: ChannelType,
        config: &Value,
    ) -> MessagingResult<Arc<dyn MessagingChannel>> {
        if channel_type == ChannelType::Telegram {
            name_unnamed_bot(config);
        }
        // On this factory's base, never the channel's real one: the calls
        // the wrapper delegates (the inbound parse) are made by `inner`.
        let inner =
            create_adapter_from_config_with_api_base(channel_type, config, Some(&self.api_base))?;
        Ok(Arc::new(OfflineSendChannel::wrapping(
            inner,
            Arc::clone(&self.sent),
            self.refuses_sends,
        )))
    }

    /// The status placeholder is posted by canot's status adapters, not by
    /// the adapter [`Self::build`] wraps, so capturing sends alone left every
    /// "thinking…" open going to the channel's real host (carnet#1600).
    fn status_api_base(&self) -> Option<String> {
        Some(self.api_base.clone())
    }

    /// The Discord Gateway is a WebSocket no proxy sees: a context that
    /// starts its client dials a loopback port nothing listens on.
    fn discord_gateway_url(&self) -> Option<String> {
        Some(REFUSED_GATEWAY.to_owned())
    }

    /// The linking routes' own requests (Telegram's `getMe`, the Graph API's
    /// number, the code exchange of a Slack or Discord OAuth link) are
    /// refused the same way, so a fixture credential never reaches the real
    /// host: the route answers that it cannot name the destination, or that
    /// the exchange could not be made. So are the Slack calls the server
    /// makes with the ops bot token (a card redraw, a modal, the feedback
    /// screenshot upload). A test that needs the link, or reads the redraw
    /// back, stands its own origin in.
    fn destination_api_base(&self, _channel_type: ChannelType) -> Option<String> {
        Some(REFUSED_STATUS_API.to_owned())
    }
}
