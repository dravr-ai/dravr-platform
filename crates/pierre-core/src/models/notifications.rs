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
/// over the same seven strings, with nothing checking that the two agreed or
/// that either covered what the server actually emits. They did not: the
/// provider-reauth notification emits [`Self::Connections`], which neither
/// map handled, so tapping it navigated nowhere on both platforms.
///
/// [`Self::surface`] resolves each screen to a surface id in the shared
/// `USER_SURFACES` registry, which already knows each platform's own route
/// for that surface. The clients read the pairing out of the generated
/// capability catalogue instead of restating it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationScreen {
    /// One activity — a sync or a personal record.
    Activity,
    /// The athlete's activity list.
    Activities,
    /// Recovery, sleep and overtraining alerts.
    Recovery,
    /// Training statistics and load trends.
    Stats,
    /// A coach message or plan update.
    Coach,
    /// Account settings.
    Settings,
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
            Self::Recovery,
            Self::Stats,
            Self::Coach,
            Self::Settings,
            Self::Connections,
        ]
    }

    /// The token this screen travels as on `data.screen`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Activity => "activity",
            Self::Activities => "activities",
            Self::Recovery => "recovery",
            Self::Stats => "stats",
            Self::Coach => "coach",
            Self::Settings => "settings",
            Self::Connections => "connections",
        }
    }

    /// The `USER_SURFACES` id this screen opens.
    ///
    /// Surfaces, not routes: the registry holds each platform's own route for
    /// a surface, so this stays the one platform-neutral answer and neither
    /// client needs a table.
    #[must_use]
    pub const fn surface(self) -> &'static str {
        match self {
            // There is no activity, load or recovery dashboard: the coach reads
            // those numbers to the athlete in the conversation, so a sync, a
            // load alert or a recovery score opens the chat where the question
            // can be asked.
            Self::Activity | Self::Activities | Self::Recovery | Self::Stats | Self::Coach => {
                "chat"
            }
            Self::Settings => "profile",
            Self::Connections => "data-providers",
        }
    }
}
