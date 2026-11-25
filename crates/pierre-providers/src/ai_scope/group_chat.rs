// ABOUTME: The members-only audience — whether a call feeds a messaging group chat, and what it may read there
// ABOUTME: A group-chat call reads nothing a members-only provider's terms keep to Dravr's members (carnet#1170)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The group-chat scope (carnet#1170).
//!
//! A Telegram, Slack or Discord group is read by whoever its chat app admits,
//! people with no Dravr account included. A call whose output is posted into
//! one runs under [`for_group_chat`]: inside it every read crosses the
//! audience gate ([`Exposure::group_chat`](pierre_core::ai_policy::Exposure::group_chat)),
//! so a members-only provider's items, and stored rows derived from them, are
//! withheld. Everywhere else a derivation that serves such data is marked
//! ([`mark_members_only_served`]), so what it writes or posts can be kept out
//! of a group chat later ([`derived_members_only`]). The parent module
//! re-exports the public surface.

use std::future::Future;

use pierre_core::ai_policy::members_only;
use pierre_core::transport::Transport;

use super::{current_provenance, flag, Provenance, PROVENANCE, UNFILTERED};
use crate::registry::global_registry;

tokio::task_local! {
    /// Set while what the current call feeds is posted into a messaging group
    /// chat.
    static GROUP_CHAT: bool;
}

/// Mark the derivation running here as having served data kept to Dravr's
/// members (carnet#1170).
///
/// Nothing is marked under [`unfiltered`], for the reason
/// [`mark_first_party_only_served`] marks nothing there.
pub fn mark_members_only_served() {
    if flag(&UNFILTERED) {
        return;
    }
    let _ = PROVENANCE.try_with(Provenance::mark_members_only);
}

/// Whether the derivation running here has served data kept to Dravr's
/// members (carnet#1170).
///
/// Nothing derived from it may then be posted into a messaging group chat.
/// `false` outside any derivation.
#[must_use]
pub fn derived_members_only() -> bool {
    current_provenance().is_some_and(|provenance| provenance.members_only())
}

/// Whether data whose AI-consent key (or provider name) is `key` is kept to
/// Dravr's members by its provider's declared terms (carnet#1170).
///
/// How a stored row's consent sources say whether it may reach a group chat.
#[must_use]
pub fn members_only_key(key: &str) -> bool {
    members_only(global_registry().as_ref(), &key.to_ascii_lowercase(), None)
}

/// Whether any of a stored row's consent sources is kept to Dravr's members
/// ([`members_only_key`]).
#[must_use]
pub fn names_members_only(sources: &[String]) -> bool {
    sources.iter().any(|key| members_only_key(key))
}

/// Run `fut` as a call whose output is posted into a messaging group chat
/// when `group_chat` holds (carnet#1170).
///
/// A turn in a Telegram, Slack or Discord group, whose readers the chat app
/// decides. Inside it, a members-only provider's items, and stored rows
/// derived from them, are withheld from every read. A declaration only
/// narrows: `false` inside a group-chat call keeps it one.
pub async fn for_group_chat<F: Future>(group_chat: bool, fut: F) -> F::Output {
    GROUP_CHAT
        .scope(group_chat || serving_group_chat(), fut)
        .await
}

/// Whether what the current call feeds is posted into a messaging group chat.
#[must_use]
pub fn serving_group_chat() -> bool {
    flag(&GROUP_CHAT)
}

/// Whether a turn served over `transport` is posted into a group chat
/// (carnet#1170).
///
/// A messaging turn that is not a direct message is. Dravr's own apps, a
/// group thread in them included, are read only by members.
#[must_use]
pub fn posts_to_group_chat(transport: Option<Transport>, is_direct_message: bool) -> bool {
    transport == Some(Transport::Messaging) && !is_direct_message
}

#[cfg(test)]
#[cfg(all(feature = "provider-garmin", feature = "provider-intervals-icu"))]
mod tests {
    use std::collections::BTreeSet;

    use dravr_tronc::dates::Timestamp;
    use pierre_core::errors::ErrorCode;
    use pierre_core::models::{Activity, ActivityBuilder, SportType};

    use super::*;
    use crate::ai_scope::{
        ai_read, filter_activities, first_party_only_read, serve_over, tracking, with_ai_consent,
    };
    use crate::registry::ProviderRegistry;

    fn ids(activities: &[Activity]) -> Vec<&str> {
        activities.iter().map(Activity::id).collect()
    }

    /// A Garmin session read through the API, and one intervals.icu relays.
    fn garmin_and_strava_sessions() -> Vec<Activity> {
        let start = Timestamp::parse_from_rfc3339("2026-11-20T06:00:00Z").unwrap();
        vec![
            ActivityBuilder::new("garmin", "Hills", SportType::Run, start, 3600, "garmin").build(),
            ActivityBuilder::new(
                "relayed",
                "Tempo",
                SportType::Run,
                start,
                3600,
                "intervals_icu",
            )
            .source("garmin")
            .build(),
            ActivityBuilder::new("strava", "Easy", SportType::Run, start, 3600, "strava").build(),
        ]
    }

    /// carnet#1170, through the registry the server builds: a turn posted into
    /// a messaging group chat reads no Garmin-recorded session, whoever relays
    /// it, and refuses an unstamped Garmin read; the member's own direct
    /// message reads all of it and is marked as drawn from it.
    #[tokio::test]
    async fn a_group_chat_turn_reads_no_garmin_data_and_a_direct_message_reads_it_all() {
        let registry = ProviderRegistry::new();
        let group_turn = Provenance::new();
        let (in_group, withheld) = tracking(
            group_turn.clone(),
            serve_over(
                Transport::Messaging,
                for_group_chat(
                    true,
                    with_ai_consent(
                        BTreeSet::new(),
                        ai_read(async {
                            filter_activities(&registry, garmin_and_strava_sessions())
                        }),
                    ),
                ),
            ),
        )
        .await;
        assert_eq!(ids(&in_group), vec!["strava"]);
        assert_eq!(withheld.off_group_chat, 2);
        assert_eq!(
            withheld.off_interface, 0,
            "a group chat is no external transport"
        );
        assert!(
            !group_turn.members_only(),
            "bug: nothing kept to members was served"
        );
        let untagged = serve_over(
            Transport::Messaging,
            for_group_chat(true, async {
                first_party_only_read(&registry, "garmin", None)
            }),
        )
        .await;
        assert_eq!(
            untagged
                .expect_err("an unstamped Garmin read is refused")
                .code,
            ErrorCode::UnavailableOverTransport
        );

        let direct = Provenance::new();
        let (in_dm, withheld) = tracking(
            direct.clone(),
            serve_over(
                Transport::Messaging,
                for_group_chat(
                    posts_to_group_chat(Some(Transport::Messaging), true),
                    with_ai_consent(
                        BTreeSet::new(),
                        ai_read(async {
                            filter_activities(&registry, garmin_and_strava_sessions())
                        }),
                    ),
                ),
            ),
        )
        .await;
        assert_eq!(ids(&in_dm), vec!["garmin", "relayed", "strava"]);
        assert!(withheld.is_empty());
        assert!(
            direct.members_only(),
            "bug: a reply drawn from Garmin is marked"
        );
        assert!(
            !posts_to_group_chat(Some(Transport::WebApp), false),
            "an in-app group thread is read by members alone"
        );
    }
}
