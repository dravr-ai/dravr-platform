// ABOUTME: The messaging sink for dispatched notifications — the third sink beside persist and push
// ABOUTME: Renders through the messaging-strings registry and sends on every channel the user linked

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Notification delivery to a user's linked chat channels.
//!
//! `dravr-commere`'s dispatcher persists the notification row and pushes to
//! Expo devices. Those are its only two sinks, so an athlete who talks to Dravr
//! on Telegram, Slack or `WhatsApp` and never installed the mobile app got
//! nothing — for any category, not only Social.
//!
//! [`MessagingChannelSink`] is the third sink, plugged in through the
//! [`NotificationChannelSink`] SPI so the dispatch pipeline keeps deciding
//! *whether* a notification is allowed (category enabled, quiet hours,
//! frequency cap) and this only decides *where else* it goes. Because it hangs
//! off `dispatch` rather than off one caller, every category that raises a
//! notification reaches messaging at once.

use std::sync::Arc;

use async_trait::async_trait;
use pierre_contremaitre::messaging_strings::{
    MessagingStringsRegistry, KEY_NOTIFICATION_CHANNEL_BODY,
};
use pierre_core::models::TenantId;
use pierre_database::RepositoryRegistry;
use pierre_notifications::{DispatchRequest, NotificationChannelSink};
use tracing::debug;

use crate::messaging_broadcast::send_to_linked_channels;

/// Delivers accepted notifications on every messaging channel the target user
/// has linked, in that link's locale.
pub struct MessagingChannelSink {
    /// Repository registry — supplies the channel-link and channel-config rows.
    repos: Arc<RepositoryRegistry>,
    /// Localized string registry the notification body is rendered through.
    strings: Arc<MessagingStringsRegistry>,
}

impl MessagingChannelSink {
    /// Build the sink from the assembled repositories and string registry.
    #[must_use]
    pub const fn new(
        repos: Arc<RepositoryRegistry>,
        strings: Arc<MessagingStringsRegistry>,
    ) -> Self {
        Self { repos, strings }
    }
}

#[async_trait]
impl NotificationChannelSink for MessagingChannelSink {
    async fn deliver(&self, request: &DispatchRequest) {
        // The commere `TenantId` newtype wraps the same UUID the platform's
        // does; channel links are stored under the platform tenant.
        let tenant_id = TenantId::from_uuid(request.tenant_id.0);
        let title = request.title.clone();
        let body = request.body.clone();

        let delivered = send_to_linked_channels(
            self.repos.messaging.as_ref(),
            tenant_id,
            request.user_id,
            |locale| {
                self.strings
                    .render(KEY_NOTIFICATION_CHANNEL_BODY, locale, &[&title, &body])
            },
        )
        .await;

        debug!(
            user_id = %request.user_id,
            category = %request.category,
            notification_type = %request.notification_type,
            delivered,
            "Notification fanned out to linked messaging channels"
        );
    }
}
