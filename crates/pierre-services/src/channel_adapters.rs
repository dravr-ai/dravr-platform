// ABOUTME: How the messaging surfaces get the channel adapter they send through
// ABOUTME: Production resolves it from the tenant's stored config; a test supplies its own
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Channel-adapter construction for the webhook ingress, the turn-resume
//! sweeper and the outbound retry worker.
//!
//! Each of them needs an adapter built from a stored channel config: the
//! ingress to verify an inbound signature and send the turn's reply, the
//! sweeper to finish a turn a drained instance left, and the retry worker to
//! re-send a queued outbound message. Production builds it from the tenant's
//! stored channel config, which yields a live transport that posts to the
//! channel's real API.
//!
//! That is correct in production and is exactly what makes a route-driven test
//! unreliable. `group_transcript_test` seeds a fixture bot token and drives the
//! real Telegram webhook route, so every turn attempted a live POST to
//! `api.telegram.org`. On a developer's machine Telegram rejects the fake token
//! in about 100 ms and the turn finishes; on a CI runner that cannot reach
//! Telegram at all the connect attempt hangs, and on 2026-09-01 it consumed the
//! whole 10 s budget those tests wait on (run 33462729445). The tests were
//! green or red according to a third party's reachability, and the failure
//! presented as backend-specific because only the SQLite shard happened to be
//! running when it hit.
//!
//! So the factory is injected at construction: the server context is built on
//! one factory, [`ConfigChannelAdapters`] in production, and its webhook
//! routes, its retry and resume workers, its proactive senders and its
//! notification sink all build their adapter through it; its linking routes
//! ask it where a pairing code's destination is looked up and where an OAuth
//! link's code is exchanged. A test context is
//! built on a factory whose sends stay in the process. There is no flag, no
//! mode, and no fallback path —
//! production has exactly one factory and never consults anything else.

use std::sync::Arc;

use dravr_canot::channel::MessagingChannel;
use dravr_canot::error::MessagingResult;
use dravr_canot::factory::create_adapter_from_config_with_api_base;
use pierre_core::models::messaging::ChannelType;
use serde_json::Value;

/// Builds the channel adapter a message is verified or sent through.
///
/// The webhook ingress calls it once per candidate channel config while
/// resolving which tenant a webhook belongs to, so it must be cheap.
pub trait ChannelAdapterFactory: Send + Sync {
    /// Build an adapter for `channel_type` from one stored channel config.
    ///
    /// The factory logs nothing: only the caller knows whose config this is
    /// and what an unusable one means there, so the reason is returned for
    /// the caller to report beside the tenant. The ingress skips such a
    /// config as "not this tenant"; a sender warns that its message cannot
    /// leave.
    ///
    /// # Errors
    ///
    /// Returns why this config cannot produce an adapter: a credential the
    /// channel needs is missing, or the build carries no such channel. The
    /// error names the missing field and never a credential's value.
    fn build(
        &self,
        channel_type: ChannelType,
        config: &Value,
    ) -> MessagingResult<Arc<dyn MessagingChannel>>;

    /// Base URL of the channel's API, as dravr-canot takes it: what the
    /// in-channel status bridge posts its placeholder to, and what the
    /// transport of an adapter [`Self::build`] returns calls.
    ///
    /// The status placeholder ("thinking…") does not travel through the
    /// adapter [`Self::build`] returns — canot's status adapters own their own
    /// HTTP client — so an offline factory that captures sends would still let
    /// every placeholder open, edit and finalize reach the live platform API.
    /// Nor do the calls an adapter makes while it parses an inbound update
    /// (Telegram's `answerCallbackQuery` for a tapped button, its `getMe`
    /// for the bot's username), which no wrapper around `send` intercepts.
    ///
    /// One rule covers both: an adapter is built on the base this names, so
    /// the placeholder and the transport always ask the same server.
    /// `None` is the production answer: each channel's real base, as canot
    /// defines it (`https://api.telegram.org`, `https://slack.com/api`,
    /// `https://discord.com/api/v10`, the Graph API version each Meta
    /// transport sends through). A test supplies one base for every channel
    /// here, the text canot appends `/bot<token>/<method>`, `/<method>`,
    /// `/channels/…` or `/<id>/messages` to: a refused loopback origin, or a
    /// mock server so the same turn opens, edits and finalizes its
    /// placeholder against something it can read back.
    ///
    /// For Slack this base ends in `/api` (`https://slack.com/api`), because
    /// canot appends the bare method name. [`Self::destination_api_base`]
    /// names Slack by scheme and authority alone (`https://slack.com`),
    /// because its callers append the whole path, `/api` included. Both
    /// shapes stay: each is what its consumer takes.
    fn status_api_base(&self) -> Option<String> {
        None
    }

    /// WebSocket URL the Discord Gateway client connects to.
    ///
    /// The Gateway is a socket the server dials, not a call of the adapter
    /// [`Self::build`] returns, and no HTTP proxy sees it. `None` is the
    /// production answer: Discord's own Gateway, as canot defines it. A test
    /// factory names a loopback address, so a context that starts the
    /// Gateway client never identifies to Discord with a fixture token.
    fn discord_gateway_url(&self) -> Option<String> {
        None
    }

    /// Origin of the platform's own requests to `channel_type`'s API, the
    /// ones no adapter makes. While linking an account: Telegram's `getMe`
    /// for the bot's username, the Graph API for a `WhatsApp` number's
    /// dialable digits, and the code exchange and identity read of a Slack
    /// or Discord OAuth link. For Slack, also the Web API calls the server
    /// makes with the deployment's bot token: an ops card redraw, a modal
    /// opened from one, the feedback screenshot upload.
    ///
    /// Those requests are made with a credential in the path, a header or
    /// the form (a bot token, an access token, the OAuth client secret), and
    /// they do not travel through the adapter [`Self::build`] returns.
    /// `None` is the production answer: the channel's real host, or the
    /// Graph API the server configuration names. A test factory names an
    /// origin of its own, so a route-driven test hands no fixture credential
    /// to the real one.
    ///
    /// For Slack and Discord this is a scheme and authority
    /// (`https://slack.com`): the caller keeps the provider's paths, `/api`
    /// included. That is not the shape of [`Self::status_api_base`], which
    /// is the base a channel's methods are appended to (`…/api` for Slack).
    fn destination_api_base(&self, _channel_type: ChannelType) -> Option<String> {
        None
    }
}

/// The production factory: the adapter is whatever the tenant's stored config
/// says it is, built by the canot factory on the channel's real API.
#[derive(Debug)]
pub struct ConfigChannelAdapters;

impl ChannelAdapterFactory for ConfigChannelAdapters {
    fn build(
        &self,
        channel_type: ChannelType,
        config: &Value,
    ) -> MessagingResult<Arc<dyn MessagingChannel>> {
        // The base this factory names is none, so canot builds the adapter
        // on the channel's own: what `create_adapter_from_config` builds.
        create_adapter_from_config_with_api_base(
            channel_type,
            config,
            self.status_api_base().as_deref(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A deployment names no base, origin or Gateway of its own: every
    /// adapter, placeholder, linking request and Gateway session goes to the
    /// host dravr-canot, or the caller, defines for the channel.
    #[test]
    fn the_production_factory_names_no_host_of_its_own() {
        let production = ConfigChannelAdapters;
        assert_eq!(production.status_api_base(), None);
        assert_eq!(production.discord_gateway_url(), None);
        for channel in [
            ChannelType::Telegram,
            ChannelType::Slack,
            ChannelType::Discord,
            ChannelType::WhatsApp,
            ChannelType::Messenger,
        ] {
            assert_eq!(production.destination_api_base(channel), None);
        }
    }
}
