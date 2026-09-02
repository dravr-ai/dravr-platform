// ABOUTME: The NotificationLocalizer SPI impl — renders a dispatched event in the recipient's locale
// ABOUTME: Sits beside the messaging sink and the persona gate as the third thing hanging off dispatch

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Notification localization at dispatch time.
//!
//! An Expo push and a message on a linked chat channel are read once and
//! cannot be re-rendered later, so the sentence they carry has to be right the
//! first time. This gate resolves the recipient's stored locale and renders
//! the event through [`NotificationTextRenderer`] before the pipeline
//! persists and pushes.
//!
//! It hangs off the [`NotificationLocalizer`] SPI — like
//! [`crate::notification_channel_sink::MessagingChannelSink`] hangs off the
//! channel-sink SPI — because the user repository and the string registry both
//! live above `pierre-notifications`.

use std::sync::Arc;

use async_trait::async_trait;
use pierre_contremaitre::messaging_strings::MessagingStringsRegistry;
use pierre_core::models::default_locale;
use pierre_database::RepositoryRegistry;
use pierre_notifications::{EventDispatch, NotificationLocalizer, NotificationText};
use serde_json::Map;
use tracing::debug;

use crate::notification_text::NotificationTextRenderer;

/// Renders a dispatched event in the recipient's stored locale.
pub struct UserLocaleNotificationLocalizer {
    /// Repository registry — supplies the user row carrying the locale.
    repos: Arc<RepositoryRegistry>,
    /// Localized string registry every notification sentence comes from.
    strings: Arc<MessagingStringsRegistry>,
}

impl UserLocaleNotificationLocalizer {
    /// Build the localizer from the assembled repositories and string registry.
    #[must_use]
    pub const fn new(
        repos: Arc<RepositoryRegistry>,
        strings: Arc<MessagingStringsRegistry>,
    ) -> Self {
        Self { repos, strings }
    }
}

#[async_trait]
impl NotificationLocalizer for UserLocaleNotificationLocalizer {
    /// Render the event in the recipient's language.
    ///
    /// A user row that cannot be read (deleted mid-dispatch, database error)
    /// falls back to the default locale rather than to English text baked into
    /// the code — the sentence still comes from the catalogue, so a locale
    /// override pushed through contremaitre still reaches it.
    async fn localize(&self, dispatch: &EventDispatch) -> NotificationText {
        let locale = match self.repos.users.get_global(dispatch.user_id).await {
            Ok(Some(user)) => user.locale,
            Ok(None) => default_locale(),
            Err(e) => {
                debug!(
                    user_id = %dispatch.user_id,
                    error = %e,
                    "notification localizer: user lookup failed; default locale"
                );
                default_locale()
            }
        };
        let renderer = NotificationTextRenderer::new(&self.strings, &locale);
        let empty = Map::new();
        let params = dispatch.params.as_object().unwrap_or(&empty);
        NotificationText {
            title: renderer.title(dispatch.event, params),
            body: renderer.body(dispatch.event, params),
            action_titles: dispatch.actions.as_ref().map_or_else(Vec::new, |specs| {
                specs
                    .iter()
                    .map(|spec| {
                        renderer
                            .action_title(spec.id)
                            .unwrap_or_else(|| spec.id.to_owned())
                    })
                    .collect()
            }),
        }
    }
}
