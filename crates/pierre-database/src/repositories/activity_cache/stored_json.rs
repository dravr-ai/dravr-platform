// ABOUTME: The data_json a cached activity row stores: the activity as served, minus its per-sample streams
// ABOUTME: Samples live in one store only, so a provider row never carries them into cached_activities
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::Activity;

use super::rows::fill_missing;
use serde_json::Value;

/// The `Activity` key that holds its per-sample streams.
const TIME_SERIES_KEY: &str = "time_series_data";

/// The `data_json` a `cached_activities` row stores for `activity`.
///
/// It is the activity as its provider served it, without
/// `time_series_data`. A provider that reads samples on its list path
/// (sciotte with `PIERRE_SCIOTTE_ENRICH_DETAILS=true` attaches the route
/// track) would otherwise persist them in every summary row; samples belong
/// to the one per-activity sample store, and the live providers read them on
/// demand. `Activity` has no setter for the field, so the strip happens on
/// the serialized object: one extra `Value` pass per upserted row.
///
/// # Errors
/// Returns a database error when the activity does not serialize, or does
/// not serialize to a JSON object.
pub(crate) fn stored_activity_json(activity: &Activity) -> AppResult<String> {
    let mut value = serde_json::to_value(activity)
        .map_err(|e| AppError::database(format!("Failed to serialize activity: {e}")))?;
    let Value::Object(object) = &mut value else {
        return Err(AppError::database(
            "Failed to serialize activity: not a JSON object",
        ));
    };
    object.remove(TIME_SERIES_KEY);
    serde_json::to_string(&value)
        .map_err(|e| AppError::database(format!("Failed to serialize activity: {e}")))
}

/// The keys only some reads of an activity carry: Strava's tags come from
/// the first training-list page alone, and the gear from a read that
/// resolved it. A later write of the same activity that lacks one must not
/// erase what an earlier read stored.
const KEPT_READ_KEYS: [&str; 2] = ["tags", "gear"];

/// [`stored_activity_json`] over a row already stored: a tag or gear read the
/// incoming copy lacks is filled in from `stored` (a read of its own
/// stands: [`fill_missing`]), every other field is the incoming copy's.
///
/// # Errors
/// As [`stored_activity_json`]; a stored copy that is not a JSON object is
/// ignored, as it holds nothing to keep.
pub(crate) fn stored_activity_json_over(
    activity: &Activity,
    stored: Option<&str>,
) -> AppResult<String> {
    let json = stored_activity_json(activity)?;
    let Some(stored) = stored else {
        return Ok(json);
    };
    let (Ok(Value::Object(mut incoming)), Ok(Value::Object(mut earlier))) = (
        serde_json::from_str::<Value>(&json),
        serde_json::from_str::<Value>(stored),
    ) else {
        return Ok(json);
    };
    earlier.retain(|key, _| KEPT_READ_KEYS.contains(&key.as_str()));
    fill_missing(&mut incoming, earlier);
    serde_json::to_string(&Value::Object(incoming))
        .map_err(|e| AppError::database(format!("Failed to serialize activity: {e}")))
}

/// `stored` with the tags and gear a provider's own list row `listed` carries,
/// and nothing else of it touched; `None` when that changes nothing, or when
/// `stored` is not a JSON object, which holds nothing to amend.
///
/// A tag read the row carries replaces the stored one: it is the fresher
/// read. A row without one leaves the stored tags alone, so a read that
/// found none never erases tags a better read stored. Gear fills a row that
/// holds none, or holds a name without the provider's id.
///
/// # Errors
/// Returns a database error when the row does not serialize.
pub(crate) fn stored_json_with_listed_reads(
    stored: &str,
    listed: &Activity,
) -> AppResult<Option<String>> {
    let Ok(Value::Object(mut row)) = serde_json::from_str::<Value>(stored) else {
        return Ok(None);
    };
    let Value::Object(read) = serde_json::to_value(listed)
        .map_err(|e| AppError::database(format!("Failed to serialize activity: {e}")))?
    else {
        return Ok(None);
    };
    let before = row.clone();
    if let Some(tags) = read.get("tags").filter(|tags| !tags.is_null()) {
        row.insert("tags".to_owned(), tags.clone());
    }
    if let Some(gear) = read.get("gear").filter(|gear| !gear.is_null()) {
        let listed_id = gear.get("id").filter(|id| !id.is_null());
        match row.get_mut("gear") {
            None | Some(Value::Null) => {
                row.insert("gear".to_owned(), gear.clone());
            }
            Some(Value::Object(held)) if listed_id.is_some() => {
                if held.get("id").is_none_or(Value::is_null) {
                    held.insert("id".to_owned(), listed_id.cloned().unwrap_or(Value::Null));
                }
            }
            Some(_) => {}
        }
    }
    if row == before {
        return Ok(None);
    }
    serde_json::to_string(&Value::Object(row))
        .map(Some)
        .map_err(|e| AppError::database(format!("Failed to serialize activity: {e}")))
}

/// The `Activity` key that holds the laps a read fills in from beside the row.
const LAPS_KEY: &str = "laps";

/// The `data_json` of an amended summary: [`stored_activity_json`]
/// without the laps a read filled in from beside the row, which stand where
/// they are stored and are not the summary's to carry.
///
/// # Errors
/// As [`stored_activity_json`].
fn amended_activity_json(activity: &Activity) -> AppResult<String> {
    let mut value: Value = serde_json::from_str(&stored_activity_json(activity)?)
        .map_err(|e| AppError::database(format!("Failed to serialize activity: {e}")))?;
    if let Value::Object(object) = &mut value {
        object.remove(LAPS_KEY);
    }
    serde_json::to_string(&value)
        .map_err(|e| AppError::database(format!("Failed to serialize activity: {e}")))
}

/// `activity` as an amend stores it: its summary alone, read back.
///
/// It is what [`amended_activity_json`] writes into the row. A read of a
/// cached row folds in the laps stored beside it, and an amendment of that
/// read hands its result to the activity store, whose record is the summary
/// too; the laps stay where they are stored.
///
/// # Errors
/// As [`stored_activity_json`], or when the summary does not read back.
pub fn amended_summary(activity: &Activity) -> AppResult<Activity> {
    serde_json::from_str(&amended_activity_json(activity)?)
        .map_err(|e| AppError::database(format!("Failed to read the amended summary: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use pierre_core::models::{
        ActivityBuilder, ActivityTag, GearRef, SportType, TagSource, TaggedBy, TimeSeriesData,
    };

    fn ride(streams: Option<TimeSeriesData>) -> Activity {
        ActivityBuilder::new(
            "ride-1",
            "Morning ride",
            SportType::Ride,
            Utc.with_ymd_and_hms(2026, 10, 27, 7, 0, 0)
                .single()
                .expect("a valid instant"),
            3_600,
            "strava",
        )
        .distance_meters(30_000.0)
        .time_series_data_opt(streams)
        .build()
    }

    fn streams() -> TimeSeriesData {
        TimeSeriesData {
            timestamps: vec![0, 1, 2],
            heart_rate: Some(vec![Some(120), Some(121), Some(122)]),
            power: None,
            cadence: None,
            speed: None,
            altitude: None,
            temperature: None,
            gps_coordinates: Some(vec![
                Some((45.5, -73.5)),
                Some((45.6, -73.6)),
                Some((45.7, -73.7)),
            ]),
            distance: None,
            grade_smooth: None,
            moving: None,
        }
    }

    #[test]
    fn a_streams_bearing_activity_is_stored_without_its_samples() {
        let json = stored_activity_json(&ride(Some(streams()))).expect("serializes");

        assert!(
            !json.contains(TIME_SERIES_KEY),
            "bug: the stored row still carries samples: {json}"
        );
        let stored: Activity = serde_json::from_str(&json).expect("an Activity");
        assert!(
            stored.time_series_data().is_none(),
            "bug: samples read back from the stored row"
        );
        assert_eq!(stored.id(), "ride-1", "bug: the summary lost its id");
        assert_eq!(
            stored.distance_meters(),
            Some(30_000.0),
            "bug: the summary lost its distance"
        );
    }

    #[test]
    fn an_activity_without_samples_is_stored_as_served() {
        let activity = ride(None);

        let json = stored_activity_json(&activity).expect("serializes");

        let stored: Value = serde_json::from_str(&json).expect("JSON");
        assert_eq!(
            stored,
            serde_json::to_value(&activity).expect("serializes"),
            "bug: an activity without samples must be stored unchanged"
        );
    }

    fn tagged(tags: Option<Vec<ActivityTag>>) -> Activity {
        ActivityBuilder::new(
            "run-1",
            "Easy run",
            SportType::Run,
            Utc.with_ymd_and_hms(2026, 11, 7, 7, 0, 0)
                .single()
                .expect("a valid instant"),
            1_800,
            "strava",
        )
        .tags_opt(tags.map(|tags| {
            tags.into_iter()
                .map(|tag| TaggedBy::new(tag, TagSource::ProviderTag))
                .collect()
        }))
        .build()
    }

    fn read_back(json: &str) -> Activity {
        serde_json::from_str(json).expect("the stored copy reads back")
    }

    fn tags_of(activity: &Activity) -> Option<Vec<ActivityTag>> {
        activity
            .tags()
            .map(|tags| tags.iter().map(|t| t.tag).collect())
    }

    #[test]
    fn a_write_without_tags_keeps_the_stored_tags() {
        let stored = stored_activity_json(&tagged(Some(vec![ActivityTag::Recovery]))).unwrap();
        let kept = stored_activity_json_over(&tagged(None), Some(&stored)).unwrap();
        assert_eq!(
            tags_of(&read_back(&kept)),
            Some(vec![ActivityTag::Recovery]),
            "bug: a deeper page without tags erased the stored tags"
        );
    }

    #[test]
    fn a_write_with_tags_replaces_the_stored_tags() {
        let stored = stored_activity_json(&tagged(Some(vec![ActivityTag::Recovery]))).unwrap();
        let replaced =
            stored_activity_json_over(&tagged(Some(vec![ActivityTag::Race])), Some(&stored))
                .unwrap();
        assert_eq!(
            tags_of(&read_back(&replaced)),
            Some(vec![ActivityTag::Race]),
            "bug: a fresh tag read did not replace the stored one"
        );
        let cleared = stored_activity_json_over(&tagged(Some(Vec::new())), Some(&stored)).unwrap();
        assert_eq!(
            tags_of(&read_back(&cleared)),
            Some(Vec::new()),
            "bug: a read that found no tag did not replace the stored tags"
        );
    }

    #[test]
    fn a_write_without_gear_keeps_the_stored_gear() {
        let with_gear = ActivityBuilder::new(
            "run-1",
            "Easy run",
            SportType::Run,
            Utc.with_ymd_and_hms(2026, 11, 7, 7, 0, 0)
                .single()
                .expect("a valid instant"),
            1_800,
            "strava",
        )
        .gear_opt(Some(GearRef {
            id: Some("g1".to_owned()),
            name: "Shoes".to_owned(),
        }))
        .build();
        let stored = stored_activity_json(&with_gear).unwrap();
        let kept = stored_activity_json_over(&tagged(None), Some(&stored)).unwrap();
        assert_eq!(
            read_back(&kept).gear().map(|g| g.name.clone()),
            Some("Shoes".to_owned()),
            "bug: a write without gear erased the stored gear"
        );
    }
}
