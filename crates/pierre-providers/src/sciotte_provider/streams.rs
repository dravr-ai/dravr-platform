// ABOUTME: Folds a scraped detail's recorded streams, or failing them its GPS track, into the activity's time series
// ABOUTME: Strava's streams cross channel for channel on their time axis; a bare track keeps its points and elevations
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use dravr_sciotte::models::{
    Activity as SciotteActivity, ActivityStreams as SciotteStreams, RouteTrack as SciotteRouteTrack,
};
use tracing::warn;

use crate::core::no_recorded_samples;
use crate::models::TimeSeriesData;

/// The activity's time series: the streams a detail read answered on their
/// time axis, else its route as a track ([`route_to_time_series`]), else
/// none, which proves nothing.
///
/// A scrape of Strava's detail reads the route and the sensor streams in one
/// request (`/activities/{id}/streams`), so when its streams answered, their
/// `latlng` and `altitude` channels hold every fix and elevation the route
/// holds, now on the time axis the route lacks. A stream set with no `latlng`
/// channel is the activity recording no GPS, as Strava's API streams say it,
/// unless the route beside it has points: the scraper then dropped a `latlng`
/// stream it could not place on the axis, and the route keeps the map.
pub(super) fn time_series(sciotte: &SciotteActivity) -> Option<TimeSeriesData> {
    let route = sciotte.route.as_ref();
    let mapped = route.is_some_and(|route| !route.coordinates.is_empty());
    match sciotte.streams.as_ref() {
        Some(streams) if streams.gps_coordinates.is_some() || !mapped => {
            Some(streams_to_time_series(streams))
        }
        _ => route.map(route_to_time_series),
    }
}

/// Sciotte's [`SciotteStreams`] as cageux's [`TimeSeriesData`]: the two
/// mirror each other field for field, every channel as long as the time axis
/// and a `None` sample one the channel did not record.
fn streams_to_time_series(streams: &SciotteStreams) -> TimeSeriesData {
    TimeSeriesData {
        timestamps: streams.timestamps.clone(),
        heart_rate: streams.heart_rate.clone(),
        power: streams.power.clone(),
        cadence: streams.cadence.clone(),
        speed: streams.speed.clone(),
        altitude: streams.altitude.clone(),
        temperature: streams.temperature.clone(),
        gps_coordinates: streams.gps_coordinates.clone(),
        distance: streams.distance.clone(),
    }
}

/// Fold sciotte's GPS [`SciotteRouteTrack`] into cageux's [`TimeSeriesData`],
/// the channel every route consumer reads — `export_routes` and the endurance
/// route endpoint hydrate their geometry from `gps_coordinates` + `altitude`.
///
/// The track of a detail that answered no time stream (Garmin's, COROS', or
/// a Strava read refused its sensor streams), so the numeric channels the
/// API providers fill from device streams stay absent here. `timestamps` is
/// empty: sciotte's `RouteTrack` carries coordinates, elevations, distances
/// and bounds but no time, and its coordinate list may be downsampled, so a
/// sample index is not a second and no time axis can be read from it. The
/// route consumers read `gps_coordinates` and `altitude` alone, and a stream
/// set without a time axis gives every time-based consumer nothing to time.
///
/// The two ways a detail answer can lack a route mean different things, and
/// the conversion keeps them apart. No `route` at all is the scraper not
/// having read one — a page read before its map rendered, a detail read past
/// the navigation budget — and the activity carries no time series, which
/// proves nothing. A `route` whose coordinate list is empty is the scraper
/// saying it read the page and the activity recorded no GPS (a trainer ride):
/// it becomes a stream set of zero samples, the same answer Strava's streams
/// give for a ride without a `latlng` channel, and the route read settles
/// `no_gps` on it.
fn route_to_time_series(route: &SciotteRouteTrack) -> TimeSeriesData {
    if route.coordinates.is_empty() {
        return no_recorded_samples();
    }

    // cageux's altitude channel is f32. Elevation in metres spans roughly
    // -430 (Dead Sea) to 8849 (Everest), well inside f32's exactly-representable
    // integer range, so narrowing costs sub-millimetre precision that no
    // barometric or GPS altimeter resolves.
    let altitude = match route.altitudes_meters.as_ref() {
        Some(meters) if meters.len() == route.coordinates.len() => {
            Some(meters.iter().map(|m| Some(*m as f32)).collect())
        }
        // An elevation series of a different length is not index-aligned with
        // the track, and a caller reading the pair position by position would
        // attribute each reading to the wrong point. Dropping it reports the
        // track's elevation as unknown, which is what a misaligned series means.
        Some(meters) => {
            warn!(
                coordinates = route.coordinates.len(),
                altitudes = meters.len(),
                "sciotte route elevation series is not index-aligned with its track; dropping it"
            );
            None
        }
        None => None,
    };

    TimeSeriesData {
        timestamps: Vec::new(),
        heart_rate: None,
        power: None,
        cadence: None,
        speed: None,
        altitude,
        temperature: None,
        gps_coordinates: Some(route.coordinates.iter().copied().map(Some).collect()),
        distance: None,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{time_series, SciotteActivity};

    /// A scraped Strava ride carrying `fields`.
    fn scraped(fields: &serde_json::Value) -> SciotteActivity {
        let mut row = json!({
            "id": "16001",
            "name": "Session",
            "sport_type": "ride",
            "start_date": "2026-11-07T13:00:00Z",
            "duration_seconds": 3_600,
            "provider": "strava",
        });
        if let (Some(row), Some(fields)) = (row.as_object_mut(), fields.as_object()) {
            row.extend(fields.clone());
        }
        serde_json::from_value(row).unwrap_or_else(|e| panic!("fixture: {e}"))
    }

    #[test]
    fn streams_cross_on_their_time_axis_ahead_of_the_route() {
        let activity = scraped(&json!({
            "route": { "coordinates": [[45.5, -73.6]], "altitudes_meters": [44.0] },
            "streams": {
                "timestamps": [0, 1],
                "heart_rate": [120, null],
                "gps_coordinates": [[45.5, -73.6], null]
            }
        }));
        let series = time_series(&activity).unwrap_or_else(|| panic!("bug: no series"));
        assert_eq!(series.timestamps, [0, 1], "bug: the time axis was dropped");
        assert_eq!(series.heart_rate, Some(vec![Some(120), None]));
        assert_eq!(
            series.gps_coordinates,
            Some(vec![Some((45.5, -73.6)), None]),
            "bug: a sample without a fix moved or vanished"
        );
        assert_eq!(
            series.altitude, None,
            "nothing the streams lack is invented"
        );
    }

    #[test]
    fn an_indoor_ride_keeps_its_sensor_streams_with_no_fix() {
        let activity = scraped(&json!({
            "route": { "coordinates": [] },
            "streams": { "timestamps": [0, 1], "heart_rate": [120, 122] }
        }));
        let series = time_series(&activity).unwrap_or_else(|| panic!("bug: no series"));
        assert_eq!(series.heart_rate, Some(vec![Some(120), Some(122)]));
        assert!(
            series.gps_coordinates.is_none(),
            "a stream set without a fix is the provider's word that none was recorded"
        );
    }

    #[test]
    fn streams_that_lost_their_fixes_leave_the_map_to_the_route() {
        let activity = scraped(&json!({
            "route": { "coordinates": [[45.5, -73.6]] },
            "streams": { "timestamps": [0, 1], "heart_rate": [120, 122] }
        }));
        let series = time_series(&activity).unwrap_or_else(|| panic!("bug: no series"));
        assert_eq!(
            series.gps_coordinates,
            Some(vec![Some((45.5, -73.6))]),
            "bug: the route's map was dropped for streams without a fix"
        );
    }

    #[test]
    fn a_detail_without_streams_keeps_its_route() {
        let activity = scraped(&json!({
            "route": { "coordinates": [[45.5, -73.6]], "altitudes_meters": [44.0] }
        }));
        let series = time_series(&activity).unwrap_or_else(|| panic!("bug: no series"));
        assert!(series.timestamps.is_empty(), "a track has no time axis");
        assert_eq!(series.gps_coordinates, Some(vec![Some((45.5, -73.6))]));
        assert!(
            time_series(&scraped(&json!({}))).is_none(),
            "no read, no series"
        );
    }
}
