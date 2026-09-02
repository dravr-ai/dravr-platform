// ABOUTME: The one renderer turning a stored notification event plus its params into a sentence
// ABOUTME: Used at write time by the localizer SPI and at read time by the notification feed

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Notification text rendering.
//!
//! A notification row stores the event that happened and the parameters that
//! describe it; the sentence is this module's job, and it is rendered in the
//! locale of whoever is about to read it. Both the write path (the Expo push
//! and the linked chat channels, through
//! [`crate::notification_localizer::UserLocaleNotificationLocalizer`]) and the
//! read path (`GET /api/notifications`) go through the same renderer, so the
//! push and the notification centre can never say different things.
//!
//! This mirrors [`crate::memory_facts::SentenceRenderer`]: one renderer over
//! the live string catalogue, one locale, no surface gluing English to data.

use pierre_contremaitre::messaging_strings::MessagingStringsRegistry;
use pierre_notifications::events::{action_label_key, NotificationEvent};
use serde_json::{Map, Value};

/// Renders notification events as sentences in one locale.
#[derive(Clone, Copy)]
pub struct NotificationTextRenderer<'a> {
    /// The live catalogue, contremaitre overlays included.
    strings: &'a MessagingStringsRegistry,
    /// The locale every string is rendered in.
    locale: &'a str,
}

impl<'a> NotificationTextRenderer<'a> {
    /// A renderer for `locale` over the live string catalogue.
    #[must_use]
    pub const fn new(strings: &'a MessagingStringsRegistry, locale: &'a str) -> Self {
        Self { strings, locale }
    }

    /// The notification title for `event`, filled from `params`.
    #[must_use]
    pub fn title(&self, event: NotificationEvent, params: &Map<String, Value>) -> String {
        self.render(event.title_key(), event.title_params(), params)
    }

    /// The notification body for `event`, filled from `params`.
    #[must_use]
    pub fn body(&self, event: NotificationEvent, params: &Map<String, Value>) -> String {
        self.render(event.body_key(), event.body_params(), params)
    }

    /// The title and body a *group* of `count` consecutive `event` rows reads
    /// as, or `None` for an event the feed never collapses.
    #[must_use]
    pub fn collapsed(&self, event: NotificationEvent, count: u32) -> Option<(String, String)> {
        let (title_key, body_key) = event.collapsed_keys()?;
        let count = count.to_string();
        Some((
            self.strings.render(title_key, self.locale, &[&count]),
            self.strings.render(body_key, self.locale, &[&count]),
        ))
    }

    /// The label of the action button `id`, or `None` for an id the catalogue
    /// has no word for — that button keeps the label it was stored with.
    #[must_use]
    pub fn action_title(&self, id: &str) -> Option<String> {
        action_label_key(id).map(|key| self.strings.render(key, self.locale, &[]))
    }

    /// Fill `key`'s template with the named parameters, in declaration order.
    ///
    /// A parameter the row does not carry renders as an empty slot rather
    /// than dropping the sentence: the row is what it is, and a missing value
    /// must not cost the athlete the rest of the text.
    fn render(&self, key: &str, names: &[&str], params: &Map<String, Value>) -> String {
        let values: Vec<String> = names
            .iter()
            .map(|name| param_text(params.get(*name)))
            .collect();
        let args: Vec<&str> = values.iter().map(String::as_str).collect();
        self.strings.render(key, self.locale, &args)
    }
}

/// A stored parameter as template text.
///
/// Triggers store display-ready strings, but a JSON number or boolean that
/// reached the row some other way still has to read as itself rather than as
/// its debug form.
fn param_text(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}
