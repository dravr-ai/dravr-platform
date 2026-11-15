// ABOUTME: What makes a stored row one activity with its other copies, for callers outside the merger
// ABOUTME: The service's own activity id when its copies share one, else the row's own provider and id

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::models::Activity;

use super::native_activity_key;

/// The one activity `activity` is a copy of.
///
/// The service and the service's own activity id its copies share
/// ([`native_activity_key`]), the same under the session's key, the grant's
/// and the athlete's export; or, for a row naming no such id, its own
/// provider and id, so it counts as itself.
///
/// The one key a caller counting activities rather than rows uses — a walk
/// picking which activities to read, a read capping how many it takes — so
/// the two never disagree about which rows are one activity.
#[must_use]
pub fn one_activity_key(activity: &Activity) -> (String, String) {
    native_activity_key(activity).map_or_else(
        || (activity.provider().to_owned(), activity.id().to_owned()),
        |(service, id)| (service.to_owned(), id.to_owned()),
    )
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use pierre_core::constants::oauth::providers as oauth_providers;
    use pierre_core::models::{Activity, ActivityBuilder, SportType};

    use super::one_activity_key;

    fn row(id: &str, provider: &str, source: Option<&str>) -> Activity {
        let start = Utc.with_ymd_and_hms(2026, 11, 7, 13, 0, 0).unwrap();
        let builder = ActivityBuilder::new(id, "Ride", SportType::Ride, start, 3_600, provider);
        match source {
            Some(source) => builder.source(source).build(),
            None => builder.build(),
        }
    }

    #[test]
    fn the_copies_of_one_strava_activity_are_one_activity() {
        let api = row("16101", oauth_providers::STRAVA, None);
        let session = row(
            "16101",
            oauth_providers::SCIOTTE,
            Some(oauth_providers::STRAVA),
        );
        let export = row("16101", oauth_providers::STRAVA_ARCHIVE, None);
        let strava = (oauth_providers::STRAVA.to_owned(), "16101".to_owned());
        assert_eq!(
            one_activity_key(&api),
            strava,
            "bug: Strava's API copy is not keyed by Strava's id"
        );
        assert_eq!(
            one_activity_key(&session),
            strava,
            "bug: the session's copy of a Strava ride is a second activity"
        );
        assert_eq!(
            one_activity_key(&export),
            strava,
            "bug: the export's copy of a Strava ride is a second activity"
        );
    }

    #[test]
    fn a_row_with_no_shared_id_counts_as_itself() {
        let first = row("77", oauth_providers::WHOOP, None);
        let other = row("78", oauth_providers::WHOOP, None);
        assert_eq!(
            one_activity_key(&first),
            (oauth_providers::WHOOP.to_owned(), "77".to_owned()),
            "bug: a row naming no shared id is not keyed by its own provider and id"
        );
        assert_ne!(
            one_activity_key(&first),
            one_activity_key(&other),
            "bug: two rows with no shared id read as one activity"
        );
    }
}
