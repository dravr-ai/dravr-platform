// ABOUTME: Intervals.icu activity provenance — the service or device that recorded an activity, named as Dravr names providers
// ABOUTME: Pure mapping of intervals.icu's `source` and `device_name`; AI policies and the Garmin attribution attach to it

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Intervals.icu activity provenance
//!
//! intervals.icu relays activities other services recorded (Garmin Connect,
//! Strava, COROS, …) beside the ones the athlete uploads or enters. A
//! provider's terms bind its data whichever service relays it, so the
//! activity keeps its origin under the name the provider registry uses.
//!
//! intervals.icu's API terms (effective 2025-10-23) also bind data a Garmin
//! device recorded, however it reached intervals.icu, and identify it by the
//! activity's `device_name` containing "garmin": a FIT file the athlete
//! uploaded from a Garmin watch is Garmin-sourced, and is shown with the
//! Garmin attribution (carnet#521).

/// The service or device that recorded an activity, named as Dravr names
/// providers, or `None` when nothing but the athlete stands behind it.
///
/// `raw` is intervals.icu's `source`. A relayed activity keeps the service
/// that relayed it, under the provider registry's name (`GARMIN_CONNECT` is
/// `garmin`); an origin Dravr does not know is kept, lowercased, for the
/// relay's policy to judge. An activity the athlete put in directly (an
/// upload, a manual entry, a Dropbox sync) is `garmin` when its
/// `device_name` names a Garmin device, and has no upstream otherwise.
pub fn upstream_source(raw: Option<&str>, device_name: Option<&str>) -> Option<String> {
    let raw = raw.map(str::trim).unwrap_or_default();
    match raw.to_ascii_uppercase().as_str() {
        "" | "UPLOAD" | "MANUAL" | "OAUTH_CLIENT" | "DROPBOX" => {
            recorded_by_garmin(device_name).then(|| "garmin".to_owned())
        }
        "GARMIN_CONNECT" | "GARMIN" => Some("garmin".to_owned()),
        _ => Some(raw.to_ascii_lowercase()),
    }
}

/// Whether intervals.icu's `device_name` names a Garmin device, as its API
/// terms identify one: the name contains "garmin", in any case.
fn recorded_by_garmin(device_name: Option<&str>) -> bool {
    device_name.is_some_and(|name| name.to_ascii_lowercase().contains("garmin"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_relayed_activity_keeps_the_service_that_recorded_it() {
        assert_eq!(
            upstream_source(Some("GARMIN_CONNECT"), None).as_deref(),
            Some("garmin")
        );
        assert_eq!(
            upstream_source(Some("STRAVA"), None).as_deref(),
            Some("strava")
        );
        assert_eq!(
            upstream_source(Some("COROS"), Some("Garmin Edge 840")).as_deref(),
            Some("coros"),
            "a relaying service keeps its own terms"
        );
    }

    #[test]
    fn an_activity_the_athlete_put_in_directly_has_no_upstream() {
        for direct in [
            Some("UPLOAD"),
            Some("MANUAL"),
            Some("OAUTH_CLIENT"),
            Some(" "),
            None,
        ] {
            assert_eq!(upstream_source(direct, None), None, "{direct:?}");
            assert_eq!(
                upstream_source(direct, Some("Wahoo ELEMNT BOLT")),
                None,
                "{direct:?}"
            );
        }
    }

    #[test]
    fn a_direct_activity_a_garmin_device_recorded_is_garmin_sourced() {
        for direct in [Some("UPLOAD"), Some("DROPBOX"), None] {
            assert_eq!(
                upstream_source(direct, Some("Garmin Forerunner 965")).as_deref(),
                Some("garmin"),
                "{direct:?}"
            );
        }
        assert_eq!(
            upstream_source(Some("MANUAL"), Some("GARMIN EDGE 1050")).as_deref(),
            Some("garmin")
        );
    }
}
