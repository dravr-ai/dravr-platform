// ABOUTME: Merges recordings into sessions and remembers which sessions hold Garmin-sourced data on any copy
// ABOUTME: The merge keeps one row per workout; this keeps the attribution Garmin's guidelines owe the rows it folds away

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Garmin provenance across a merge (carnet#1154).
//!
//! [`merge_duplicates`] keeps one canonical row per workout, and the canonical
//! row may be another service's copy: a ride a Garmin watch recorded and
//! Strava relayed without its device name. Garmin's brand guidelines still
//! owe a figure derived from that session its attribution, so the merge is
//! made here with the Garmin-sourced copies noted first ([`garmin_recorded`]),
//! and a session holds Garmin-sourced data when any copy folded into it did.
//!
//! Merges nest: a member's rows are merged per tenant, then once more across
//! tenants. [`GarminSessions::merge_all`] carries what each earlier merge
//! noted into the next, so a mark is never lost to the second fold.

use std::collections::HashSet;

use pierre_core::constants::oauth_providers::{
    garmin_contribution, garmin_recorded, GarminContribution,
};
use pierre_core::models::Activity;

use super::{merge_duplicates, DedupConfig, FragmentReport};

/// Sessions merged from every recording of each workout, with the ones that
/// hold Garmin-sourced data on any of their copies.
#[derive(Debug, Clone, Default)]
pub struct GarminSessions {
    /// One row per workout, as [`merge_duplicates`] keeps it.
    pub sessions: Vec<Activity>,
    /// Ids of the sessions holding Garmin-sourced data.
    garmin: HashSet<String>,
}

impl GarminSessions {
    /// Merge `copies` into sessions, noting which hold Garmin-sourced data,
    /// with the merge's report.
    #[must_use]
    pub fn merge(copies: Vec<Activity>, config: &DedupConfig) -> (Self, FragmentReport) {
        Self::merge_marked(copies, &HashSet::new(), config)
    }

    /// Merge sessions several earlier merges produced once more, keeping what
    /// each of them noted.
    #[must_use]
    pub fn merge_all(parts: impl IntoIterator<Item = Self>, config: &DedupConfig) -> Self {
        let mut copies = Vec::new();
        let mut marked = HashSet::new();
        for part in parts {
            marked.extend(part.garmin);
            copies.extend(part.sessions);
        }
        Self::merge_marked(copies, &marked, config).0
    }

    /// The merge, with each copy in `marked` counted as Garmin-sourced
    /// whatever it names.
    fn merge_marked(
        copies: Vec<Activity>,
        marked: &HashSet<String>,
        config: &DedupConfig,
    ) -> (Self, FragmentReport) {
        let garmin_copies: HashSet<String> = copies
            .iter()
            .filter(|copy| {
                marked.contains(copy.id()) || garmin_recorded(copy.source(), copy.device_name())
            })
            .map(|copy| copy.id().to_owned())
            .collect();
        let (sessions, report) = merge_duplicates(copies, config);
        let garmin = sessions
            .iter()
            .filter(|session| {
                garmin_copies.contains(session.id())
                    || report
                        .groups
                        .iter()
                        .find(|group| group.canonical_id == session.id())
                        .is_some_and(|group| {
                            group
                                .fragment_ids
                                .iter()
                                .any(|id| garmin_copies.contains(id))
                        })
            })
            .map(|session| session.id().to_owned())
            .collect();
        (Self { sessions, garmin }, report)
    }

    /// Whether `session`, one of [`Self::sessions`], holds Garmin-sourced data
    /// on any of its copies.
    #[must_use]
    pub fn garmin_recorded(&self, session: &Activity) -> bool {
        self.garmin.contains(session.id())
    }

    /// What Garmin contributed to a figure derived from every session.
    #[must_use]
    pub fn garmin_contribution(&self) -> Option<GarminContribution> {
        garmin_contribution(
            self.sessions
                .iter()
                .map(|session| self.garmin_recorded(session)),
        )
    }
}

#[cfg(test)]
mod tests {
    use dravr_tronc::dates::{Elapsed, Timestamp};
    use pierre_core::constants::oauth_providers::GarminContribution;
    use pierre_core::models::{Activity, ActivityBuilder, SportType};

    use super::{DedupConfig, GarminSessions};

    fn ride(id: &str, provider: &str, start: Timestamp) -> Activity {
        ActivityBuilder::new(id, "Morning Ride", SportType::Ride, start, 3_600, provider)
            .distance_meters(30_000.0)
            .build()
    }

    /// A Garmin copy folded behind another service's canonical row still
    /// marks the session it was folded into.
    #[test]
    fn a_garmin_copy_behind_another_canonical_marks_its_session() {
        let start = Timestamp::now() - Elapsed::days(1);
        let strava = ride("s-1", "strava", start);
        let garmin = ActivityBuilder::new("g-1", "Ride", SportType::Ride, start, 3_600, "garmin")
            .distance_meters(30_000.0)
            .source("garmin")
            .build();
        let (merged, _) = GarminSessions::merge(vec![strava, garmin], &DedupConfig::default());
        assert_eq!(merged.sessions.len(), 1, "one workout");
        assert!(merged.garmin_recorded(&merged.sessions[0]));
        assert_eq!(merged.garmin_contribution(), Some(GarminContribution::Sole));
    }

    /// A mark survives a second merge, and a session with no Garmin copy
    /// makes the contribution partial.
    #[test]
    fn a_second_merge_keeps_the_marks_of_the_first() {
        let start = Timestamp::now() - Elapsed::days(2);
        let garmin = ActivityBuilder::new("g-2", "Ride", SportType::Ride, start, 3_600, "garmin")
            .distance_meters(30_000.0)
            .source("garmin")
            .build();
        let (first, _) = GarminSessions::merge(vec![garmin], &DedupConfig::default());
        let other = ride("s-2", "strava", start + Elapsed::days(1));
        let (second, _) = GarminSessions::merge(vec![other], &DedupConfig::default());
        let all = GarminSessions::merge_all([first, second], &DedupConfig::default());
        assert_eq!(all.sessions.len(), 2);
        assert_eq!(all.garmin_contribution(), Some(GarminContribution::Part));

        let (none, _) =
            GarminSessions::merge(vec![ride("s-3", "strava", start)], &DedupConfig::default());
        assert_eq!(none.garmin_contribution(), None);
    }
}
