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
    /// A conversation with the agent, named by the notification's `id`: an
    /// agent message, or an insight the agent computed while answering in it.
    Coach,
    /// The athlete's connected data providers.
    Connections,
    /// A group's info, opened over the recipient's own copy of the group's
    /// chat, named by the notification's `id`: where a member answers the
    /// coaching-platform link the group's coach proposed.
    GroupInfo,
    /// The athlete's privacy settings, opened on one data access request
    /// named by the notification's `id`: where a due-soon or overdue
    /// reminder's request, its letter and its steps live (carnet#1659).
    Privacy,
}

impl NotificationScreen {
    /// Every screen a notification can name.
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[
            Self::Activity,
            Self::Activities,
            Self::Coach,
            Self::Connections,
            Self::GroupInfo,
            Self::Privacy,
        ]
    }

    /// The token this screen travels as on `data.screen`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Activity => "activity",
            Self::Activities => "activities",
            Self::Coach => "coach",
            Self::Connections => "connections",
            Self::GroupInfo => "group_info",
            Self::Privacy => "privacy",
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
            // Home carries the week's sessions and the latest activities with
            // their routes.
            Self::Activity | Self::Activities => "home",
            // The chat is a list of threads, so it is a destination only for a
            // notification that names one: the clients open nothing for a
            // `coach` or `group_info` payload without an `id`. A group's info
            // is drawn over its chat, so `group_info` opens the thread with
            // the info over it.
            Self::Coach | Self::GroupInfo => "chat",
            Self::Connections => "connections",
            // The privacy settings pane holds the data access requests; the
            // clients open the one a payload names in `id`, and the pane
            // alone for a payload that names none.
            Self::Privacy => "privacy",
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::NotificationScreen;

    #[test]
    fn every_screen_travels_as_its_serde_token() -> Result<(), serde_json::Error> {
        for &screen in NotificationScreen::all() {
            let wire = serde_json::to_value(screen)?;
            assert_eq!(wire, screen.as_str(), "{screen:?} travels as its token");
            let back: NotificationScreen = serde_json::from_value(wire)?;
            assert_eq!(
                back, screen,
                "bug: {screen:?} does not parse back from its token"
            );
        }
        Ok(())
    }

    #[test]
    fn all_lists_each_screen_once() {
        let tokens: HashSet<&str> = NotificationScreen::all()
            .iter()
            .map(|screen| screen.as_str())
            .collect();
        assert_eq!(
            tokens.len(),
            NotificationScreen::all().len(),
            "bug: all() lists a screen twice, or two screens share a token"
        );
    }

    #[test]
    fn a_data_access_reminder_opens_the_privacy_pane() {
        // The pane holds the athlete's data access requests (carnet#1659).
        // `privacy` is a wire value: every shipped client parses it from a
        // delivered notification, so it is pinned as a literal.
        assert!(
            NotificationScreen::all().contains(&NotificationScreen::Privacy),
            "bug: the privacy screen is missing from all(), so the catalogue omits it"
        );
        assert_eq!(
            NotificationScreen::Privacy.as_str(),
            "privacy",
            "bug: the privacy screen's wire token moved"
        );
        assert_eq!(
            NotificationScreen::Privacy.surface(),
            "privacy",
            "bug: the privacy screen does not open the privacy surface"
        );
    }
}
