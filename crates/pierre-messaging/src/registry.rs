// ABOUTME: Channel registry mapping ChannelType to adapter instances
// ABOUTME: Thread-safe lookup for routing inbound webhooks to the correct channel adapter
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashMap;
use std::sync::Arc;

use pierre_core::models::messaging::ChannelType;
use tracing::warn;

use crate::channel::MessagingChannel;

/// Registry of active messaging channel adapters
///
/// Maps `ChannelType` to `Arc<dyn MessagingChannel>` for thread-safe lookups.
pub struct ChannelRegistry {
    /// Registered channel adapters by type
    channels: HashMap<ChannelType, Arc<dyn MessagingChannel>>,
}

impl ChannelRegistry {
    /// Create an empty registry
    #[must_use]
    pub fn new() -> Self {
        Self {
            channels: HashMap::new(),
        }
    }

    /// Register a channel adapter
    ///
    /// Returns `false` if a channel of the same type is already registered.
    pub fn register(&mut self, channel: Arc<dyn MessagingChannel>) -> bool {
        let channel_type = channel.channel_type();
        if self.channels.contains_key(&channel_type) {
            warn!("Channel {channel_type} already registered, skipping");
            return false;
        }
        self.channels.insert(channel_type, channel);
        true
    }

    /// Look up a channel adapter by type
    #[must_use]
    pub fn get(&self, channel_type: &ChannelType) -> Option<&Arc<dyn MessagingChannel>> {
        self.channels.get(channel_type)
    }

    /// List all registered channel types
    #[must_use]
    pub fn registered_channels(&self) -> Vec<ChannelType> {
        self.channels.keys().copied().collect()
    }

    /// Number of registered channels
    #[must_use]
    pub fn len(&self) -> usize {
        self.channels.len()
    }

    /// Whether any channels are registered
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.channels.is_empty()
    }
}

impl Default for ChannelRegistry {
    fn default() -> Self {
        Self::new()
    }
}
