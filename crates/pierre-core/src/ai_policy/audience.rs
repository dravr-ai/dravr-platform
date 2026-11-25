// ABOUTME: Who may read a provider's data among a surface's readers — the members-only audience gate (carnet#1170)
// ABOUTME: A members-only provider's items, wherever relayed, never feed what is posted into a messaging group chat

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The audience gate.
//!
//! A provider whose terms keep its data to Dravr's members declares
//! [`AudiencePolicy::MembersOnly`]; [`members_only`] reads that for an item,
//! and the filters in the parent module drop such an item whenever what the
//! call feeds is posted into a messaging group chat
//! ([`Exposure::group_chat`](super::Exposure::group_chat)).

use serde_json::Value;

use super::{ProviderTerms, Withheld};
use crate::transport::AudiencePolicy;

/// Whether an item from `provider` whose upstream is `source` must stay out of
/// a messaging group chat (carnet#1170).
///
/// It must when the provider that served it, or the service that recorded it
/// upstream, declares [`AudiencePolicy::MembersOnly`]. No relay settles this
/// for the source it relays: whoever serves a Garmin-recorded activity, the
/// people reading a group chat are not Garmin's to choose.
#[must_use]
pub fn members_only(lookup: &dyn ProviderTerms, provider: &str, source: Option<&str>) -> bool {
    let declares = |name: &str| {
        lookup
            .audience_policy(name)
            .is_some_and(AudiencePolicy::is_members_only)
    };
    declares(provider)
        || source
            .filter(|src| !src.eq_ignore_ascii_case(provider))
            .is_some_and(|src| declares(&src.to_ascii_lowercase()))
}

impl Withheld {
    /// The neutral note a model answering into a messaging group chat reads in
    /// place of what was held back from it: that data exists and is not shown
    /// in a group chat, and that the member can ask in a direct message —
    /// never what it is, nor which service it came from.
    #[must_use]
    pub fn group_chat_note(&self) -> Value {
        const NOT_IN_GROUP_CHAT: &str = "Some of this data is not shown in a group chat, because the people in the chat are not all Dravr members and the terms of the service it came from keep it to them. The member can see it in the Dravr app, or ask about it in a direct message; do not guess its contents and do not describe it here.";
        serde_json::json!({
            "items_withheld": self.off_group_chat,
            "reason": "not_shown_in_a_group_chat",
            "note": NOT_IN_GROUP_CHAT,
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::ai_policy::{filter_json, Exposure, SourcePolicy};
    use crate::transport::TransportPolicy;

    /// A read for a model on one of Dravr's own surfaces.
    const MODEL: Exposure = Exposure {
        to_model: true,
        external: false,
        group_chat: false,
    };

    /// Terms that declare no audience for anyone.
    struct NoAudience;

    impl ProviderTerms for NoAudience {
        fn ai_policy(&self, _provider: &str) -> Option<&'static SourcePolicy> {
            None
        }

        fn transport_policy(&self, _provider: &str) -> Option<TransportPolicy> {
            Some(TransportPolicy::AnyTransport)
        }
    }

    /// Terms where `garmin` keeps its data to Dravr's members, and every
    /// other provider declares nothing about its audience.
    struct MembersOnlyGarmin;

    impl ProviderTerms for MembersOnlyGarmin {
        fn ai_policy(&self, _provider: &str) -> Option<&'static SourcePolicy> {
            None
        }

        fn transport_policy(&self, _provider: &str) -> Option<TransportPolicy> {
            Some(TransportPolicy::AnyTransport)
        }

        fn audience_policy(&self, provider: &str) -> Option<AudiencePolicy> {
            (provider == "garmin").then_some(AudiencePolicy::MembersOnly)
        }
    }

    // carnet#1170: the provider or the recorded source decides; no relay
    // settles a group chat's audience for what it relays.
    #[test]
    fn a_members_only_provider_or_source_keeps_its_items_out_of_a_group_chat() {
        assert!(members_only(&MembersOnlyGarmin, "garmin", None));
        assert!(members_only(
            &MembersOnlyGarmin,
            "intervals",
            Some("GARMIN")
        ));
        assert!(members_only(&MembersOnlyGarmin, "sciotte", Some("garmin")));
        assert!(!members_only(&MembersOnlyGarmin, "strava", None));
        assert!(!members_only(
            &MembersOnlyGarmin,
            "intervals",
            Some("wahoo")
        ));
        assert!(
            !members_only(&NoAudience, "garmin", None),
            "terms declaring no audience restrict none"
        );
    }

    #[test]
    fn a_group_chat_read_drops_members_only_items_and_nothing_else() {
        let payload = json!({
            "activities": [
                {"id": "1", "provider": "garmin", "name": "Hills"},
                {"id": "2", "provider": "intervals", "source": "garmin", "name": "Tempo"},
                {"id": "3", "provider": "strava", "name": "Easy"}
            ]
        });
        let group_chat = Exposure {
            to_model: true,
            external: false,
            group_chat: true,
        };
        let mut posted = payload.clone();
        let withheld = filter_json(&MembersOnlyGarmin, &mut posted, group_chat);
        assert_eq!(
            posted,
            json!({"activities": [{"id": "3", "provider": "strava", "name": "Easy"}]})
        );
        assert_eq!(withheld.off_group_chat, 2);
        assert_eq!(withheld.off_interface, 0);
        assert!(!withheld.is_empty());
        let note = withheld.group_chat_note();
        assert_eq!(note["items_withheld"], 2, "{note}");
        assert!(
            !note["note"].as_str().unwrap_or_default().contains("Garmin"),
            "bug: the note never names the service it withholds: {note}"
        );

        let mut direct = payload.clone();
        assert!(
            filter_json(&MembersOnlyGarmin, &mut direct, MODEL).is_empty(),
            "the member's own direct message keeps everything"
        );
        assert_eq!(direct, payload);
    }
}
