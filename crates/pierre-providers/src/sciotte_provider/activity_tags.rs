// ABOUTME: Maps a scraped Strava activity's raw tag map into cageux activity tags
// ABOUTME: Tag ids are Strava's tag id space, never the workout_type code space
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use dravr_sciotte::models::Activity as SciotteActivity;
use pierre_core::models::{ActivityTag, TagSource, TaggedBy};

/// The athlete's own tags on a scraped Strava activity, each stamped
/// [`TagSource::ProviderTag`]. `None` when the page carried no tag map (an
/// activity past the first training-list page, or one read by its detail
/// page alone), so "never read" stays apart from "read, none set" (`Some`
/// of an empty list).
///
/// Strava's tag ids are their own vocabulary ([`ActivityTag::from_strava_tag`]);
/// the `workout_type` code is read separately by cageux's merged view.
pub(super) fn provider_tags(sciotte: &SciotteActivity) -> Option<Vec<TaggedBy>> {
    sciotte.tags.as_ref().map(|raw| {
        ActivityTag::from_strava_tags(raw)
            .into_iter()
            .map(|tag| TaggedBy::new(tag, TagSource::ProviderTag))
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::super::{convert_activity, SciotteTarget};
    use super::{ActivityTag, SciotteActivity, TagSource};

    fn scraped(tags: Option<serde_json::Value>) -> SciotteActivity {
        let mut payload = serde_json::json!({
            "id": "16002",
            "name": "Easy spin",
            "sport_type": "run",
            "start_date": "2026-11-07T13:00:00Z",
            "duration_seconds": 1800,
            "provider": "strava"
        });
        if let Some(tags) = tags {
            payload["tags"] = tags;
        }
        serde_json::from_value(payload).unwrap_or_else(|e| panic!("fixture: {e}"))
    }

    #[test]
    fn strava_tag_ids_become_provider_tags_in_id_order() {
        let row = scraped(Some(
            serde_json::json!({"12": true, "3": true, "1": false, "7": true}),
        ));
        let activity = convert_activity(&row, SciotteTarget::Strava);
        let tags = activity
            .tags()
            .unwrap_or_else(|| panic!("bug: tags dropped"));
        let got: Vec<_> = tags.iter().map(|t| (t.tag, t.source)).collect();
        assert_eq!(
            got,
            vec![
                (ActivityTag::Commute, TagSource::ProviderTag),
                (ActivityTag::Recovery, TagSource::ProviderTag),
            ],
            "bug: unset ids and sport-family flags must not become tags"
        );
    }

    #[test]
    fn only_strava_reads_the_tag_map() {
        let row = scraped(Some(serde_json::json!({"1": true})));
        let activity = convert_activity(&row, SciotteTarget::Garmin);
        assert!(
            activity.tags().is_none(),
            "bug: a non-Strava target read Strava tag ids"
        );
    }

    #[test]
    fn a_page_without_a_tag_map_claims_nothing() {
        let activity = convert_activity(&scraped(None), SciotteTarget::Strava);
        assert!(activity.tags().is_none(), "bug: unread tags became Some");
    }

    #[test]
    fn a_read_page_with_no_tag_set_is_an_empty_list() {
        let activity = convert_activity(
            &scraped(Some(serde_json::json!({"1": false}))),
            SciotteTarget::Strava,
        );
        assert_eq!(
            activity.tags().map(<[_]>::len),
            Some(0),
            "bug: a read page with no tag set is not an empty list"
        );
    }
}
