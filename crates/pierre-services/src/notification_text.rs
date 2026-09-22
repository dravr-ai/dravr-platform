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

use pierre_contremaitre::messaging_strings::{
    MessagingStringsRegistry, KEY_GROUP_DIGEST_ALL_CLEAR, KEY_GROUP_DIGEST_CONCERNS_HEADER,
    KEY_GROUP_DIGEST_CONCERN_DEEP_FATIGUE, KEY_GROUP_DIGEST_CONCERN_HEAVY_BLOCK,
    KEY_GROUP_DIGEST_CONCERN_INACTIVE, KEY_GROUP_DIGEST_CONCERN_OVERTRAINING_RISK,
    KEY_GROUP_DIGEST_CONCERN_VOLUME_DROP, KEY_GROUP_DIGEST_FRESH,
    KEY_GROUP_DIGEST_HIGHLIGHTS_HEADER, KEY_GROUP_DIGEST_MEMBERS_HEADER,
    KEY_GROUP_DIGEST_MEMBER_LINE, KEY_GROUP_DIGEST_MEMBER_LINE_PREV, KEY_GROUP_DIGEST_SUMMARY,
    KEY_GROUP_DIGEST_TREND_DECLINING, KEY_GROUP_DIGEST_TREND_IMPROVING,
    KEY_GROUP_DIGEST_TREND_STABLE,
};
use pierre_notifications::events::{action_label_key, NotificationEvent};
use serde_json::{Map, Value};

/// Group-digest concern code: form in the deepest fatigue band.
pub const CONCERN_DEEP_FATIGUE: &str = "deep_fatigue";
/// Group-digest concern code: form at the deep end of the productive zone.
pub const CONCERN_HEAVY_BLOCK: &str = "heavy_block";
/// Group-digest concern code: no form reading, high overtraining risk.
pub const CONCERN_OVERTRAINING_RISK: &str = "overtraining_risk";
/// Group-digest concern code: no recent activity.
pub const CONCERN_INACTIVE: &str = "inactive";
/// Group-digest concern code: volume below the group average.
pub const CONCERN_VOLUME_DROP: &str = "volume_drop";

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
        if event == NotificationEvent::GroupWeeklyDigest {
            return self.group_digest_body(params);
        }
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

    /// The group weekly digest, one line per fact: the summary, the trend,
    /// then each member's volume, the members in fresh form and the flagged
    /// ones, each block under its own header.
    ///
    /// Composed here rather than stored as a sentence so the feed can render
    /// the same row again in whatever language the reader switches to. An
    /// entry whose code this build does not know is left out rather than
    /// shown as a key.
    fn group_digest_body(&self, params: &Map<String, Value>) -> String {
        let mut lines = vec![self.line(
            KEY_GROUP_DIGEST_SUMMARY,
            &[
                &whole(params.get("active_members")),
                &whole(params.get("total_members")),
                &self.decimal(params.get("avg_volume_km")),
            ],
        )];
        let trend_key = match params.get("trend").and_then(Value::as_str) {
            Some("improving") => Some(KEY_GROUP_DIGEST_TREND_IMPROVING),
            Some("declining") => Some(KEY_GROUP_DIGEST_TREND_DECLINING),
            Some("stable") => Some(KEY_GROUP_DIGEST_TREND_STABLE),
            _ => None,
        };
        if let Some(key) = trend_key {
            lines.push(self.line(key, &[]));
        }

        let members: Vec<String> = entries(params, "members")
            .map(|m| {
                let name = param_text(m.get("name"));
                let km = self.decimal(m.get("km"));
                m.get("prev_km").filter(|v| !v.is_null()).map_or_else(
                    || self.line(KEY_GROUP_DIGEST_MEMBER_LINE, &[&name, &km]),
                    |prev| {
                        self.line(
                            KEY_GROUP_DIGEST_MEMBER_LINE_PREV,
                            &[&name, &km, &self.decimal(Some(prev))],
                        )
                    },
                )
            })
            .collect();
        let highlights: Vec<String> = entries(params, "highlights")
            .map(|h| {
                self.line(
                    KEY_GROUP_DIGEST_FRESH,
                    &[&param_text(h.get("name")), &signed(h.get("form_pct"))],
                )
            })
            .collect();
        let concerns: Vec<String> = entries(params, "concerns")
            .filter_map(|c| self.concern_line(c))
            .collect();

        let all_clear = highlights.is_empty() && concerns.is_empty();
        for (header, block) in [
            (KEY_GROUP_DIGEST_MEMBERS_HEADER, members),
            (KEY_GROUP_DIGEST_HIGHLIGHTS_HEADER, highlights),
            (KEY_GROUP_DIGEST_CONCERNS_HEADER, concerns),
        ] {
            if !block.is_empty() {
                lines.push(String::new());
                lines.push(self.line(header, &[]));
                lines.extend(block);
            }
        }
        if all_clear {
            lines.push(String::new());
            lines.push(self.line(KEY_GROUP_DIGEST_ALL_CLEAR, &[]));
        }
        lines.join("\n")
    }

    /// One flagged member's line, or `None` for a concern code this build
    /// does not know.
    fn concern_line(&self, concern: &Map<String, Value>) -> Option<String> {
        let name = param_text(concern.get("name"));
        let value = concern.get("value");
        let (key, value) = match concern.get("code").and_then(Value::as_str)? {
            CONCERN_DEEP_FATIGUE => (KEY_GROUP_DIGEST_CONCERN_DEEP_FATIGUE, signed(value)),
            CONCERN_HEAVY_BLOCK => (KEY_GROUP_DIGEST_CONCERN_HEAVY_BLOCK, signed(value)),
            CONCERN_OVERTRAINING_RISK => {
                (KEY_GROUP_DIGEST_CONCERN_OVERTRAINING_RISK, String::new())
            }
            CONCERN_INACTIVE => (KEY_GROUP_DIGEST_CONCERN_INACTIVE, whole(value)),
            CONCERN_VOLUME_DROP => (KEY_GROUP_DIGEST_CONCERN_VOLUME_DROP, whole(value)),
            _ => return None,
        };
        Some(self.line(key, &[&name, &value]))
    }

    /// `key` rendered in this renderer's locale with positional `args`.
    fn line(&self, key: &str, args: &[&str]) -> String {
        self.strings.render(key, self.locale, args)
    }

    /// A one-decimal number in this locale's notation: English writes
    /// `211.4`, and every other supported locale writes `211,4`.
    fn decimal(&self, value: Option<&Value>) -> String {
        let Some(number) = value.and_then(Value::as_f64) else {
            return String::new();
        };
        let text = format!("{number:.1}");
        if self.locale.starts_with("en") {
            text
        } else {
            text.replace('.', ",")
        }
    }
}

/// The objects stored under `params[name]`, skipping anything that is not one.
fn entries<'p>(
    params: &'p Map<String, Value>,
    name: &str,
) -> impl Iterator<Item = &'p Map<String, Value>> {
    params
        .get(name)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
}

/// A number rounded to a whole value.
fn whole(value: Option<&Value>) -> String {
    value
        .and_then(Value::as_f64)
        .map_or_else(String::new, |number| format!("{number:.0}"))
}

/// A number rounded to a whole value with its sign always shown.
fn signed(value: Option<&Value>) -> String {
    value
        .and_then(Value::as_f64)
        .map_or_else(String::new, |number| format!("{number:+.0}"))
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
