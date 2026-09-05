// ABOUTME: How a verified webhook gets the channel adapter it dispatches through
// ABOUTME: Production resolves it from the tenant's stored config; a test supplies its own
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Channel-adapter construction for the webhook ingress.
//!
//! The ingress needs an adapter twice per request: once to verify the inbound
//! signature and parse the payload, and once to send whatever the turn decides
//! to say. Production builds it from the tenant's stored channel config, which
//! yields a live transport that posts to the channel's real API.
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
//! So the factory is injected at construction: [`MessagingRoutes::routes`]
//! always installs [`ConfigChannelAdapters`], and a test builds the same router
//! with its own factory. There is no flag, no mode, and no fallback path —
//! production has exactly one factory and never consults anything else.
//!
//! [`MessagingRoutes::routes`]: super::MessagingRoutes::routes

use std::sync::Arc;

use pierre_core::models::messaging::ChannelType;
use pierre_messaging::channel::MessagingChannel;
use pierre_messaging::factory::create_adapter_from_config;
use serde_json::Value;
use tracing::debug;

/// Builds the channel adapter a webhook is verified and dispatched through.
///
/// Called once per candidate channel config while resolving which tenant a
/// webhook belongs to, so it must be cheap and must not fail the request when a
/// single config is unusable — an unbuildable adapter is simply not a match.
pub trait ChannelAdapterFactory: Send + Sync {
    /// Build an adapter for `channel_type` from one stored channel config.
    ///
    /// Returns `None` when this config cannot produce an adapter (missing or
    /// malformed credentials), which the caller treats as "not this tenant"
    /// rather than as an error.
    fn build(&self, channel_type: ChannelType, config: &Value)
        -> Option<Arc<dyn MessagingChannel>>;

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
}

/// The production factory: the adapter is whatever the tenant's stored config
/// says it is, built by the canot factory.
pub struct ConfigChannelAdapters;

impl ChannelAdapterFactory for ConfigChannelAdapters {
    fn build(
        &self,
        channel_type: ChannelType,
        config: &Value,
    ) -> Option<Arc<dyn MessagingChannel>> {
        match create_adapter_from_config(channel_type, config) {
            Ok(adapter) => Some(adapter),
            Err(e) => {
                debug!(
                    channel = %channel_type,
                    error = %e,
                    "Skipping config with missing credentials"
                );
                None
            }
        }
    }
}
