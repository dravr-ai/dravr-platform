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
//! notification sink all build their adapter through it; its linking route
//! asks it where a pairing code's destination is looked up. A test context is
//! built on a factory whose sends stay in the process. There is no flag, no
//! mode, and no fallback path —
//! production has exactly one factory and never consults anything else.

use std::sync::Arc;

use dravr_canot::channel::MessagingChannel;
use dravr_canot::error::MessagingResult;
use dravr_canot::factory::create_adapter_from_config;
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

    /// Base URL the in-channel status bridge posts its placeholder to.
    ///
    /// The status placeholder ("thinking…") does not travel through the
    /// adapter [`Self::build`] returns — canot's status adapters own their own
    /// HTTP client — so an offline factory that captures sends would still let
    /// every placeholder open, edit and finalize reach the live platform API.
    /// `None` is the production answer: the channel's real host. A test
    /// supplies its mock server here so the same turn opens, edits and
    /// finalizes its placeholder against something it can read back.
    fn status_api_base(&self) -> Option<String> {
        None
    }

    /// Origin the linking route asks where `channel_type`'s pairing code
    /// goes: Telegram's `getMe` for the bot's username, the Graph API for a
    /// `WhatsApp` number's dialable digits.
    ///
    /// That lookup is the platform's own request, made with the stored
    /// credential in its path or its header, and it does not travel through
    /// the adapter [`Self::build`] returns. `None` is the production answer:
    /// the channel's real host, or the Graph API the server configuration
    /// names. A test factory names an origin of its own, so a route-driven
    /// test hands no fixture credential to the real one.
    fn destination_api_base(&self, _channel_type: ChannelType) -> Option<String> {
        None
    }
}

/// The production factory: the adapter is whatever the tenant's stored config
/// says it is, built by the canot factory.
#[derive(Debug)]
pub struct ConfigChannelAdapters;

impl ChannelAdapterFactory for ConfigChannelAdapters {
    fn build(
        &self,
        channel_type: ChannelType,
        config: &Value,
    ) -> MessagingResult<Arc<dyn MessagingChannel>> {
        create_adapter_from_config(channel_type, config)
    }
}
