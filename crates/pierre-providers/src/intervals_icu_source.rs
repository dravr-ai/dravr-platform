// ABOUTME: Intervals.icu activity provenance — the service that recorded a relayed activity, named as Dravr names providers
// ABOUTME: Pure mapping of intervals.icu's `source` field; provider AI policies attach to that origin (carnet#723)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Intervals.icu activity provenance
//!
//! intervals.icu relays activities other services recorded (Garmin Connect,
//! Strava, COROS, …) beside the ones the athlete uploads or enters. A
//! provider's terms bind its data whichever service relays it, so the
//! activity keeps its origin under the name the provider registry uses.

/// The service that recorded a relayed activity, named as Dravr names
/// providers, or `None` when the athlete put it into intervals.icu directly.
///
/// The name must match the provider registry's: `GARMIN_CONNECT` is `garmin`.
/// An origin Dravr does not know is kept, lowercased, for the relay's policy
/// to judge.
pub fn upstream_source(raw: Option<&str>) -> Option<String> {
    let raw = raw?.trim();
    match raw.to_ascii_uppercase().as_str() {
        "" | "UPLOAD" | "MANUAL" | "OAUTH_CLIENT" | "DROPBOX" => None,
        "GARMIN_CONNECT" | "GARMIN" => Some("garmin".to_owned()),
        _ => Some(raw.to_ascii_lowercase()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_relayed_activity_keeps_the_service_that_recorded_it() {
        assert_eq!(
            upstream_source(Some("GARMIN_CONNECT")).as_deref(),
            Some("garmin")
        );
        assert_eq!(upstream_source(Some("STRAVA")).as_deref(), Some("strava"));
        assert_eq!(upstream_source(Some("COROS")).as_deref(), Some("coros"));
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
            assert_eq!(upstream_source(direct), None, "{direct:?}");
        }
    }
}
