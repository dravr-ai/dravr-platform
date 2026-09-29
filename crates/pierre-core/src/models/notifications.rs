// ABOUTME: Notification screen vocabulary — where a push notification wants to land in the app
// ABOUTME: Resolves each screen to a USER_SURFACES id so web and mobile read one platform-neutral answer
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// The notification records themselves — categories, device tokens,
// preferences, the feed, schedules — are dravr-commere's models, reached as
// `pierre_notifications::models`. This module holds only the app-side
// vocabulary the platform adds on top of them.

use serde::{Deserialize, Serialize};

/// Where a notification wants to land in the app.
///
/// One vocabulary, declared once. The value travels on a notification's
/// `data.screen` field, and every client has to turn it into a destination —
/// which web and mobile each did with a hand-written switch of their own,
/// over the same strings, with nothing checking that the two agreed or
/// that either covered what the server actually emits. They did not: the
/// provider-reauth notification emits [`Self::Connections`], which neither
/// map handled, so tapping it navigated nowhere on both platforms.
///
/// [`Self::surface`] resolves each screen to a surface id in the shared
/// `USER_SURFACES` registry, which already knows each platform's own route
/// for that surface. The clients read the pairing out of the generated
/// capability catalogue instead of restating it.
///
/// Every token names a destination that shows the notification's subject. A
/// notification with nothing to show carries no screen at all, and the clients
/// render it as information rather than as a link.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationScreen {
    /// One activity — a sync or a personal record.
    Activity,
    /// The athlete's recent training — a weekly summary or a workout reminder.
    Activities,
    /// The athlete's training plan.
    Plan,
    /// A conversation with the agent, named by the notification's `id`: an
    /// agent message, or an insight the agent computed while answering in it.
    Coach,
    /// The athlete's connected data providers.
    Connections,
}

impl NotificationScreen {
    /// Every screen a notification can name.
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[
            Self::Activity,
            Self::Activities,
            Self::Plan,
            Self::Coach,
            Self::Connections,
        ]
    }

    /// The token this screen travels as on `data.screen`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Activity => "activity",
            Self::Activities => "activities",
            Self::Plan => "plan",
            Self::Coach => "coach",
            Self::Connections => "connections",
        }
    }

    /// The destination id this screen opens: a `USER_SURFACES` surface or a
    /// `SETTINGS_PANES` pane.
    ///
    /// Destinations, not routes: the registry holds each platform's own route
    /// for a destination, so this stays the one platform-neutral answer and
    /// neither client needs a table.
    #[must_use]
    pub const fn surface(self) -> &'static str {
        match self {
            // Home carries today's session, the plan's week around it and the
            // latest activities with their routes.
            Self::Activity | Self::Activities | Self::Plan => "home",
            // The chat is a list of threads, so it is a destination only for a
            // notification that names one: the clients open nothing for a
            // `coach` payload without an `id`.
            Self::Coach => "chat",
            Self::Connections => "connections",
        }
    }
}
