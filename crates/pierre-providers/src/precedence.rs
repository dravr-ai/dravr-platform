// ABOUTME: The one classifier from a stored row's provider key, data source or device to a precedence Source
// ABOUTME: dravr-equilibre's rank decides between Sources; this decides which Source a row is, once
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Which [`Source`] a stored row is (Athlete Data Consolidation §3.1).
//!
//! dravr-equilibre's [`rank`](dravr_equilibre::precedence::rank) is the one
//! precedence rule; it ranks a `Source` — the service whose device recorded
//! the reading and the ingress it reached Dravr by — and never sees a storage
//! key. This module turns the keys Dravr stores rows under into sources, so
//! the knowledge of what each key means lives in one place: the mirror
//! backends ([`backend_resolver`]), the archive keys, the relays and the
//! recording devices they name.
//!
//! | Stored key | Service | Ingress |
//! |---|---|---|
//! | `garmin` (push, and the usr/pwd capture it shares a row key with) | garmin | api |
//! | `garmin_archive`, `strava_archive` | the export's service | export |
//! | a mirror backend (`sciotte`, `sciotte_coros`...) | the service it signed in to | capture |
//! | `intervals_icu`, `trainingpeaks`, `terra`, naming another service's device | that service | relay |
//! | `upload` | the device's service, else upload | upload |
//! | `athlete` | athlete | manual |
//! | any other connection | its own service | api |
//!
//! A workout's copy on Strava names the device that recorded it (the API's
//! detail, the export's file): a Garmin watch's run is Garmin's recording
//! relayed by Strava, so the one rank puts Garmin's own copy first.
//!
//! The `Source` is used for precedence only. Terms, attribution and the
//! retention sweeps key on the stored provider key itself: a Strava export is
//! the athlete's own copy, never data fetched under Strava's terms.

pub use dravr_equilibre::precedence::{rank, Ingress, Kind, Service, Source};
use pierre_core::constants::oauth_providers::{
    garmin_recorded, ATHLETE, COROS, GARMIN, GARMIN_ARCHIVE, INTERVALS_ICU, STRAVA, STRAVA_ARCHIVE,
    SYNTHETIC, SYNTHETIC_SLEEP, TERRA, TRAININGPEAKS, UPLOAD, WAHOO, WHOOP,
};
use pierre_core::models::{Activity, DataSource};

use crate::backend_resolver;

/// The service a name (a provider key, a relayed `source`, a device or an
/// app) names, `None` when it names none Dravr ranks.
///
/// Matched as a case-insensitive substring, as device names carry the brand
/// among other words (`Garmin Forerunner 965`, `Apple Watch Ultra 2`).
#[must_use]
pub fn service_named(name: &str) -> Option<Service> {
    let lower = name.to_ascii_lowercase();
    if lower == INTERVALS_ICU || lower.contains("intervals.icu") {
        return Some(Service::IntervalsIcu);
    }
    [
        (GARMIN, Service::Garmin),
        (WHOOP, Service::Whoop),
        ("oura", Service::Oura),
        ("polar", Service::Polar),
        ("apple", Service::Apple),
        ("fitbit", Service::Fitbit),
        (COROS, Service::Coros),
        ("suunto", Service::Suunto),
        (WAHOO, Service::Wahoo),
        ("withings", Service::Withings),
        (TRAININGPEAKS, Service::TrainingPeaks),
        (STRAVA, Service::Strava),
    ]
    .into_iter()
    .find(|(brand, _)| lower.contains(brand))
    .map(|(_, service)| service)
}

/// The source of a row filed under `key` before anything the row names is
/// read, `None` for a key no row is filed under.
///
/// Every key a connection, an import, an upload or a manual entry writes is
/// listed here, so a new key cannot be ranked by accident: guard G5 holds
/// every registered key, every user-provided key and [`ATHLETE`] to it.
#[must_use]
pub fn key_source(key: &str) -> Option<Source> {
    let source = |service, ingress| Some(Source::new(service, ingress));
    match key {
        ATHLETE => source(Service::Athlete, Ingress::Manual),
        GARMIN => source(Service::Garmin, Ingress::Api),
        GARMIN_ARCHIVE => source(Service::Garmin, Ingress::Export),
        STRAVA => source(Service::Strava, Ingress::Api),
        STRAVA_ARCHIVE => source(Service::Strava, Ingress::Export),
        WHOOP => source(Service::Whoop, Ingress::Api),
        COROS => source(Service::Coros, Ingress::Api),
        WAHOO => source(Service::Wahoo, Ingress::Api),
        INTERVALS_ICU => source(Service::IntervalsIcu, Ingress::Api),
        TRAININGPEAKS => source(Service::TrainingPeaks, Ingress::Api),
        UPLOAD => source(Service::Upload, Ingress::Upload),
        // An aggregator: what it holds is its devices' (a relay), never its own.
        TERRA => source(Service::Other, Ingress::Relay),
        // Fixture data, generated on the spot.
        SYNTHETIC | SYNTHETIC_SLEEP => source(Service::Other, Ingress::Api),
        mirror if backend_resolver::is_mirror_backend(mirror) => {
            let service = service_named(backend_resolver::user_facing_name(mirror))?;
            source(service, Ingress::Capture)
        }
        _ => None,
    }
}

/// [`key_source`], an unknown key ranking as another service's own data.
fn base_source(key: &str) -> Source {
    key_source(key).unwrap_or(Source::new(Service::Other, Ingress::Api))
}

/// Whether `key` is a service that relays other services' recordings: its
/// rows name the service or device that recorded them.
fn is_relay(key: &str) -> bool {
    [INTERVALS_ICU, TRAININGPEAKS, TERRA].contains(&key)
}

/// Whether a wellness row filed under `key` needs its data source to name
/// its recorder ([`wellness_source`]): a relay's or an upload's row does.
#[must_use]
pub fn needs_data_source(key: &str) -> bool {
    key == UPLOAD || is_relay(key)
}

/// The source of a stored wellness row (a night, a day's recovery, body or
/// activity totals) filed under `key`, with its `data_source` when the reader
/// holds it.
///
/// A relay's or an upload's row names its recorder only through its data
/// source: an intervals.icu day whose data source names a Garmin device, or
/// Garmin as its source, is Garmin's, relayed. Without the data source the
/// row is the key's own ([`key_source`]).
#[must_use]
pub fn wellness_source(key: &str, data_source: Option<&DataSource>) -> Source {
    let base = base_source(key);
    if !needs_data_source(key) {
        return base;
    }
    let recorder = data_source.and_then(|ds| {
        [
            ds.source.as_deref(),
            ds.device_model.as_deref(),
            ds.original_source_name.as_deref(),
        ]
        .into_iter()
        .flatten()
        .find_map(service_named)
    });
    match recorder {
        Some(service) if key == UPLOAD => Source::new(service, Ingress::Upload),
        Some(service) if service != base.service => Source::new(service, Ingress::Relay),
        _ => base,
    }
}

/// The source of one cached copy of a workout.
///
/// - The service's own key: its API (`garmin`, `coros`, `whoop`...); its
///   export (`garmin_archive`, `strava_archive`); a usr/pwd capture (a mirror
///   backend, whose row names the service it signed in to as its `source`).
/// - A copy a relay holds — Strava, intervals.icu, TrainingPeaks, Terra — of
///   a recording another service's device made ([`garmin_recorded`], or a
///   device or `source` naming that service) is that service's, relayed.
///   Strava's copy of a recording its own app made, or naming no device, is
///   Strava's own.
/// - An upload is its device's service, uploaded.
#[must_use]
pub fn activity_source(activity: &Activity) -> Source {
    let key = activity.provider();
    let device = activity.device_name();
    let relayed = activity.source();
    let own = if backend_resolver::is_mirror_backend(key) {
        service_named(backend_resolver::activity_origin(key, relayed)).map_or_else(
            || base_source(key),
            |service| Source::new(service, Ingress::Capture),
        )
    } else {
        base_source(key)
    };
    let recorder = if garmin_recorded(relayed, device) {
        Some(Service::Garmin)
    } else {
        device.or(relayed).and_then(service_named)
    };
    match recorder {
        Some(service) if key == UPLOAD => Source::new(service, Ingress::Upload),
        Some(service) if service != own.service && relays_recordings(own.service) => {
            Source::new(service, Ingress::Relay)
        }
        _ => own,
    }
}

/// Whether a service's copies of a workout can be another service's
/// recording relayed: Strava, intervals.icu, TrainingPeaks and an aggregator
/// hold the recordings of the devices their athletes sync to them.
const fn relays_recordings(service: Service) -> bool {
    matches!(
        service,
        Service::Strava | Service::IntervalsIcu | Service::TrainingPeaks | Service::Other
    )
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use pierre_core::constants::oauth_providers::USER_PROVIDED_PROVIDERS;
    use pierre_core::constants::oauth_providers::{
        ATHLETE, GARMIN, GARMIN_ARCHIVE, INTERVALS_ICU, SCIOTTE, SCIOTTE_COROS, STRAVA,
        STRAVA_ARCHIVE, UPLOAD, WHOOP,
    };
    use pierre_core::models::{ActivityBuilder, DataSource, DeviceType, SportType};

    use super::*;
    use crate::registry::ProviderRegistry;

    fn data_source(provider: &str, device: Option<&str>, source: Option<&str>) -> DataSource {
        DataSource {
            id: "ds".to_owned(),
            user_id: "u".to_owned(),
            provider: provider.to_owned(),
            device_model: device.map(str::to_owned),
            software_version: None,
            source: source.map(str::to_owned),
            device_type: DeviceType::Watch,
            original_source_name: None,
        }
    }

    fn run(key: &str, device: Option<&str>, source: Option<&str>) -> Activity {
        let start = Utc.with_ymd_and_hms(2026, 10, 1, 7, 0, 0).unwrap();
        ActivityBuilder::new("1", "Run", SportType::Run, start, 3_600, key)
            .device_name_opt(device.map(str::to_owned))
            .source_opt(source.map(str::to_owned))
            .build()
    }

    #[test]
    fn each_stored_wellness_key_is_its_service_and_ingress() {
        let cases = [
            (GARMIN, Source::new(Service::Garmin, Ingress::Api)),
            (
                GARMIN_ARCHIVE,
                Source::new(Service::Garmin, Ingress::Export),
            ),
            (
                STRAVA_ARCHIVE,
                Source::new(Service::Strava, Ingress::Export),
            ),
            (WHOOP, Source::new(Service::Whoop, Ingress::Api)),
            (SCIOTTE_COROS, Source::new(Service::Coros, Ingress::Capture)),
            (
                INTERVALS_ICU,
                Source::new(Service::IntervalsIcu, Ingress::Api),
            ),
            (ATHLETE, Source::new(Service::Athlete, Ingress::Manual)),
        ];
        for (key, source) in cases {
            assert_eq!(wellness_source(key, None), source, "{key}");
        }
    }

    #[test]
    fn an_intervals_day_naming_a_garmin_device_is_garmins_relayed() {
        let ds = data_source(INTERVALS_ICU, Some("Garmin Forerunner 965"), None);
        assert_eq!(
            wellness_source(INTERVALS_ICU, Some(&ds)),
            Source::new(Service::Garmin, Ingress::Relay)
        );
        let relayed = data_source(INTERVALS_ICU, None, Some("garmin"));
        assert_eq!(
            wellness_source(INTERVALS_ICU, Some(&relayed)),
            Source::new(Service::Garmin, Ingress::Relay)
        );
    }

    #[test]
    fn a_strava_copy_of_a_garmin_recording_is_garmins_relayed() {
        assert_eq!(
            activity_source(&run(STRAVA, Some("Garmin Forerunner 965"), None)),
            Source::new(Service::Garmin, Ingress::Relay)
        );
        assert_eq!(
            activity_source(&run(STRAVA_ARCHIVE, Some("Garmin Edge 840"), None)),
            Source::new(Service::Garmin, Ingress::Relay)
        );
    }

    #[test]
    fn a_strava_recording_or_a_copy_naming_no_device_is_stravas_own() {
        assert_eq!(
            activity_source(&run(STRAVA, Some("Strava iPhone App"), None)),
            Source::new(Service::Strava, Ingress::Api)
        );
        assert_eq!(
            activity_source(&run(STRAVA, None, None)),
            Source::new(Service::Strava, Ingress::Api)
        );
        assert_eq!(
            activity_source(&run(STRAVA_ARCHIVE, None, None)),
            Source::new(Service::Strava, Ingress::Export)
        );
    }

    #[test]
    fn the_services_own_copies_rank_by_ingress() {
        assert_eq!(
            activity_source(&run(GARMIN, Some("Forerunner 965"), None)),
            Source::new(Service::Garmin, Ingress::Api)
        );
        assert_eq!(
            activity_source(&run(SCIOTTE, None, Some(GARMIN))),
            Source::new(Service::Garmin, Ingress::Capture)
        );
        assert_eq!(
            activity_source(&run(GARMIN_ARCHIVE, None, None)),
            Source::new(Service::Garmin, Ingress::Export)
        );
    }

    #[test]
    fn an_upload_is_its_devices_service() {
        assert_eq!(
            activity_source(&run(UPLOAD, Some("COROS PACE 3"), None)),
            Source::new(Service::Coros, Ingress::Upload)
        );
        assert_eq!(
            activity_source(&run(UPLOAD, None, None)),
            Source::new(Service::Upload, Ingress::Upload)
        );
    }

    /// Guard G5 (Athlete Data Consolidation §7): every key a row is filed
    /// under — every registered connection, every key of the athlete's own
    /// data — is one the classifier names and one whose terms are declared,
    /// so no key is ranked or served by accident.
    #[test]
    fn every_written_key_parses_and_has_a_terms_descriptor() {
        let registry = ProviderRegistry::new();
        let mut keys: Vec<&str> = registry.supported_providers();
        keys.extend(USER_PROVIDED_PROVIDERS);
        keys.push(ATHLETE);
        assert!(
            keys.len() > 3,
            "the registry registered no connection: {keys:?}"
        );
        for key in keys {
            assert!(key_source(key).is_some(), "{key}: no precedence source");
            assert!(
                registry.terms_descriptor(key).is_some(),
                "{key}: no terms descriptor"
            );
        }
    }

    /// The keys recovery rows are stored under before this rank existed rank
    /// among themselves as dravr-equilibre 0.8's `recovery_priority` ranked
    /// them, so a merged day an open `pending_advice` evaluation re-reads
    /// keeps the primary — and the baseline — it was opened with
    /// (Athlete Data Consolidation S2).
    #[test]
    fn stored_recovery_keys_keep_the_order_open_evaluations_were_opened_under() {
        use std::cmp::Ordering;
        let before = [WHOOP, GARMIN, "coros", INTERVALS_ICU, STRAVA];
        for (i, a) in before.iter().enumerate() {
            for b in &before[i + 1..] {
                assert_eq!(
                    rank(
                        Kind::Recovery,
                        &wellness_source(a, None),
                        &wellness_source(b, None)
                    ),
                    Ordering::Less,
                    "{a} must still lead {b}"
                );
            }
        }
        assert_eq!(
            rank(
                Kind::Recovery,
                &wellness_source(SCIOTTE_COROS, None),
                &wellness_source(INTERVALS_ICU, None)
            ),
            Ordering::Less
        );
    }

    #[test]
    fn a_whoop_workout_is_whoops_whatever_it_names() {
        assert_eq!(
            activity_source(&run(WHOOP, None, None)),
            Source::new(Service::Whoop, Ingress::Api)
        );
    }
}
