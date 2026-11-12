// ABOUTME: The field dictionary: which activity fields and stream channels each backend supplies, list and detail
// ABOUTME: One function joins a provider's two modes (mirror, OAuth) and the one serving, for the model to read
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! What each backend supplies of an [`Activity`](crate::models::Activity).
//!
//! A provider reached two ways supplies two different sets: Strava's API
//! carries the description, the workout tag and the trainer and commute flags
//! that a scrape of Strava's pages does not, and the scrape carries heart rate
//! on a detail page only. A field a backend never supplies reads, on every row,
//! exactly like a field the athlete never recorded: absent. The model that
//! reads the row cannot tell the two apart, and an absent heart rate read as
//! "no effort" is a wrong answer.
//!
//! So every backend's [`ProviderDescriptor`](crate::spi::ProviderDescriptor)
//! declares what its mapper fills ([`ActivityFieldSupport`]), and
//! [`field_support`] joins a provider's two modes and the one serving the
//! athlete into one answer ([`FieldSupport`]) for the model to read. A
//! declared field can still be absent on a row (no heart-rate strap, no
//! description written); an undeclared one is never there.
//!
//! The names are the keys the [`Activity`](crate::models::Activity) and its
//! [`TimeSeriesData`](crate::models::TimeSeriesData) serialize under, so the
//! model reads the dictionary in the words the rows use.
//!
//! A declaration follows the payloads a backend's mapper is measured against,
//! never what its provider's app shows. Two fields the athlete can write are
//! therefore declared absent, because no measured payload carries them:
//!
//! - Garmin's push (`garmin`) declares no description: the pushed activity
//!   summaries the ingest is measured against (the `tests/fixtures/garmin`
//!   payloads of `pierre-server`) carry none, so its mapper has nothing to
//!   fill. Garmin's mirror (`sciotte_garmin`) is where an athlete's Garmin
//!   description would come from.
//! - Wahoo (`wahoo`) declares neither a description nor notes: the Cloud API
//!   reference's workout sample its field guard is measured against carries
//!   neither, in the workout or its `workout_summary`.
//!
//! When a measured payload carries one, the mapper fills it and the
//! declaration gains it in the same change.

use crate::backend_resolver::{
    activity_origin, hosted_login_target, is_mirror_backend, mirror_backend_for, user_facing_name,
};
use crate::registry::ProviderRegistry;
use crate::spi::ProviderDescriptor;

bitflags::bitflags! {
    /// Activity fields a backend can supply, beyond the identity, time and
    /// distance every activity carries.
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
    pub struct ActivityFields: u32 {
        /// The athlete's own description of the activity.
        const DESCRIPTION = 1 << 0;
        /// The note only the athlete reads (Strava's private note).
        const PRIVATE_NOTE = 1 << 1;
        /// Comments under the activity: the notes a provider keeps in one
        /// field (a TrainingPeaks athlete or coach comment, a COROS note) and
        /// its timestamped thread.
        const COMMENTS = 1 << 2;
        /// The athlete's tag for the session: race, long run, workout.
        const WORKOUT_TYPE = 1 << 3;
        /// Recorded on an indoor trainer or treadmill.
        const TRAINER = 1 << 4;
        /// Flagged as a commute.
        const COMMUTE = 1 << 5;
        /// Average heart rate.
        const AVERAGE_HEART_RATE = 1 << 6;
        /// Maximum heart rate.
        const MAX_HEART_RATE = 1 << 7;
        /// Average power.
        const AVERAGE_POWER = 1 << 8;
        /// Normalized (weighted) power.
        const NORMALIZED_POWER = 1 << 9;
        /// Moving time beside the elapsed duration.
        const MOVING_TIME = 1 << 10;
        /// Energy, in kilocalories.
        const CALORIES = 1 << 11;
        /// Average cadence.
        const AVERAGE_CADENCE = 1 << 12;
        /// The gear (shoes, bike) the activity was recorded with.
        const GEAR = 1 << 13;
        /// The device that recorded it.
        const DEVICE = 1 << 14;
        /// The laps the athlete or the workout marked.
        const LAPS = 1 << 15;
        /// Uniform distance splits.
        const SPLITS = 1 << 16;
        /// Segment efforts.
        const SEGMENT_EFFORTS = 1 << 17;
        /// The route as an encoded summary polyline.
        const SUMMARY_POLYLINE = 1 << 18;
        /// The provider's own sport label, finer than the sport type (a
        /// gravel ride, a trail run).
        const SPORT_TYPE_DETAIL = 1 << 19;
        /// The athlete's perceived exertion.
        const PERCEIVED_EXERTION = 1 << 20;
    }
}

/// Each field's name, the key its [`Activity`](crate::models::Activity)
/// serializes it under, in flag order.
const FIELD_NAMES: [(ActivityFields, &str); 21] = [
    (ActivityFields::DESCRIPTION, "description"),
    (ActivityFields::PRIVATE_NOTE, "private_note"),
    (ActivityFields::COMMENTS, "comments"),
    (ActivityFields::WORKOUT_TYPE, "workout_type"),
    (ActivityFields::TRAINER, "trainer"),
    (ActivityFields::COMMUTE, "commute"),
    (ActivityFields::AVERAGE_HEART_RATE, "average_heart_rate"),
    (ActivityFields::MAX_HEART_RATE, "max_heart_rate"),
    (ActivityFields::AVERAGE_POWER, "average_power"),
    (ActivityFields::NORMALIZED_POWER, "normalized_power"),
    (ActivityFields::MOVING_TIME, "moving_time_seconds"),
    (ActivityFields::CALORIES, "calories"),
    (ActivityFields::AVERAGE_CADENCE, "average_cadence"),
    (ActivityFields::GEAR, "gear"),
    (ActivityFields::DEVICE, "device_name"),
    (ActivityFields::LAPS, "laps"),
    (ActivityFields::SPLITS, "splits"),
    (ActivityFields::SEGMENT_EFFORTS, "segment_efforts"),
    (ActivityFields::SUMMARY_POLYLINE, "summary_polyline"),
    (ActivityFields::SPORT_TYPE_DETAIL, "sport_type_detail"),
    (ActivityFields::PERCEIVED_EXERTION, "perceived_exertion"),
];

impl ActivityFields {
    /// The names of the fields set, in flag order.
    #[must_use]
    pub fn names(self) -> Vec<&'static str> {
        FIELD_NAMES
            .iter()
            .filter(|(flag, _)| self.contains(*flag))
            .map(|(_, name)| *name)
            .collect()
    }
}

bitflags::bitflags! {
    /// The sample channels of an activity's streams a backend can supply.
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
    pub struct StreamChannels: u16 {
        /// A time axis: seconds from the start, one per sample.
        const TIME = 1 << 0;
        /// Heart rate per sample.
        const HEART_RATE = 1 << 1;
        /// Power per sample.
        const POWER = 1 << 2;
        /// Cadence per sample.
        const CADENCE = 1 << 3;
        /// Speed per sample.
        const SPEED = 1 << 4;
        /// Altitude per sample.
        const ALTITUDE = 1 << 5;
        /// A GPS fix per sample.
        const LATLNG = 1 << 6;
        /// Cumulative distance per sample.
        const DISTANCE = 1 << 7;
        /// Temperature per sample.
        const TEMPERATURE = 1 << 8;
    }
}

/// Each channel's name, the key its [`TimeSeriesData`](crate::models::TimeSeriesData)
/// serializes it under, in flag order.
const CHANNEL_NAMES: [(StreamChannels, &str); 9] = [
    (StreamChannels::TIME, "timestamps"),
    (StreamChannels::HEART_RATE, "heart_rate"),
    (StreamChannels::POWER, "power"),
    (StreamChannels::CADENCE, "cadence"),
    (StreamChannels::SPEED, "speed"),
    (StreamChannels::ALTITUDE, "altitude"),
    (StreamChannels::LATLNG, "gps_coordinates"),
    (StreamChannels::DISTANCE, "distance"),
    (StreamChannels::TEMPERATURE, "temperature"),
];

impl StreamChannels {
    /// The names of the channels set, in flag order.
    #[must_use]
    pub fn names(self) -> Vec<&'static str> {
        CHANNEL_NAMES
            .iter()
            .filter(|(flag, _)| self.contains(*flag))
            .map(|(_, name)| *name)
            .collect()
    }
}

/// What one backend's mapper fills on the activities it returns.
///
/// `list` is what a list read carries on every row; `detail_only` what only a
/// read of one activity adds; `streams` the channels of its sample streams,
/// which are read per activity. A field in neither set is one the backend
/// never supplies.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ActivityFieldSupport {
    /// Fields every list row carries when the activity recorded them.
    pub list: ActivityFields,
    /// Fields only a detail read adds.
    pub detail_only: ActivityFields,
    /// Channels of the activity's sample streams.
    pub streams: StreamChannels,
}

impl ActivityFieldSupport {
    /// A backend that promises nothing: the default, until its descriptor
    /// declares what its mapper fills.
    pub const NONE: Self = Self {
        list: ActivityFields::empty(),
        detail_only: ActivityFields::empty(),
        streams: StreamChannels::empty(),
    };

    /// Whether the backend declares nothing at all.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.list.is_empty() && self.detail_only.is_empty() && self.streams.is_empty()
    }

    /// Every field the backend supplies, on a list row or a detail read.
    #[must_use]
    pub const fn supplied(&self) -> ActivityFields {
        self.list.union(self.detail_only)
    }

    /// The fields the backend never supplies, list or detail.
    #[must_use]
    pub const fn not_supplied(&self) -> ActivityFields {
        self.supplied().complement()
    }
}

/// Which of a provider's two backends answers a read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FieldMode {
    /// The scrape that signs in as the athlete (a sciotte backend).
    Mirror,
    /// The provider's own API, behind its OAuth grant or API key.
    Oauth,
}

impl FieldMode {
    /// The word the connection surfaces use for it (`mirror`, `oauth`).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Mirror => "mirror",
            Self::Oauth => "oauth",
        }
    }

    /// The user-facing provider an activity row came from and the mode that
    /// served it, read off the row's stored `provider` and `source`: a mirror
    /// row names its mirror backend and carries the service it scraped as its
    /// source.
    #[must_use]
    pub fn of_row<'a>(provider: &'a str, source: Option<&'a str>) -> (&'a str, Self) {
        let mode = if is_mirror_backend(provider) {
            Self::Mirror
        } else {
            Self::Oauth
        };
        (activity_origin(provider, source), mode)
    }
}

/// A provider's field dictionary: what each of its modes supplies and which
/// one serves the athlete.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldSupport {
    /// The user-facing provider (`strava`, `garmin`).
    pub provider: &'static str,
    /// What the provider's mirror supplies; `None` when it has no mirror.
    pub mirror: Option<ActivityFieldSupport>,
    /// What the provider's own API supplies; `None` when it has none Dravr
    /// reads activities from.
    pub oauth: Option<ActivityFieldSupport>,
    /// The mode serving the athlete, when one does.
    pub serving: Option<FieldMode>,
}

impl FieldSupport {
    /// What the serving mode supplies, when a mode serves.
    #[must_use]
    pub const fn served(&self) -> Option<ActivityFieldSupport> {
        match self.serving {
            Some(FieldMode::Mirror) => self.mirror,
            Some(FieldMode::Oauth) => self.oauth,
            None => None,
        }
    }
}

/// The field dictionary of a user-facing `provider`, with `serving` the mode
/// that answers the athlete (`None` when nothing does).
///
/// `provider` may also name a mirror backend (`sciotte_garmin`); it reads as
/// the provider it mirrors. A mode is present only when a registered
/// descriptor serves activities in it and declares what it supplies (an
/// imported archive declares nothing, so it has no dictionary): Strava and Garmin have both, an API
/// provider (Intervals.icu, Wahoo) has its API alone, and TrainingPeaks its
/// mirror alone. `None` when neither mode is registered for activities — an
/// unknown name, or a provider that serves none (sleep only).
#[must_use]
pub fn field_support(
    registry: &ProviderRegistry,
    provider: &str,
    serving: Option<FieldMode>,
) -> Option<FieldSupport> {
    // A mirror-backed provider is named by its pair (TrainingPeaks has no
    // descriptor of its own); any other by its registered descriptor.
    let user_facing =
        match mirror_backend_for(user_facing_name(provider)).and_then(hosted_login_target) {
            Some(paired) => paired,
            None => registry.get_descriptor(provider)?.name(),
        };
    let declared = |backend: &str| {
        registry
            .get_descriptor(backend)
            .filter(|descriptor| descriptor.capabilities().supports_activities())
            .map(ProviderDescriptor::activity_fields)
            .filter(|fields| !fields.is_empty())
    };
    let mirror = mirror_backend_for(user_facing).and_then(declared);
    let oauth = declared(user_facing);
    if mirror.is_none() && oauth.is_none() {
        return None;
    }
    let serving = serving.filter(|mode| match mode {
        FieldMode::Mirror => mirror.is_some(),
        FieldMode::Oauth => oauth.is_some(),
    });
    Some(FieldSupport {
        provider: user_facing,
        mirror,
        oauth,
        serving,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::{ActivityFieldSupport, ActivityFields, StreamChannels, CHANNEL_NAMES, FIELD_NAMES};
    use crate::models::Activity;

    /// The guard every mapper's unit test runs: `list_row`, mapped from a list
    /// fixture, carries every field `declared` promises on a list row, and
    /// `detailed`, mapped from a detail fixture with its streams, carries every
    /// field and channel it promises at all.
    ///
    /// # Panics
    ///
    /// Names `backend` and each field its mapper left out.
    pub(crate) fn assert_mapper_fills(
        backend: &str,
        declared: ActivityFieldSupport,
        list_row: &Activity,
        detailed: &Activity,
    ) {
        use pierre_test_support::activity_fields::missing;

        assert!(
            !declared.is_empty(),
            "bug: {backend} serves activities and declares no field"
        );
        let on_list = missing(list_row, &declared.list.names(), &[]);
        assert!(
            on_list.is_empty(),
            "bug: {backend} declares {on_list:?} on a list row, and its mapper left them out"
        );
        let on_detail = missing(
            detailed,
            &declared.supplied().names(),
            &declared.streams.names(),
        );
        assert!(
            on_detail.is_empty(),
            "bug: {backend} declares {on_detail:?}, and its mapper left them out of a detail read"
        );
    }

    #[test]
    fn every_flag_has_one_name() {
        let named = FIELD_NAMES
            .iter()
            .fold(ActivityFields::empty(), |all, (flag, _)| all | *flag);
        assert_eq!(named, ActivityFields::all(), "bug: a field has no name");
        let channels = CHANNEL_NAMES
            .iter()
            .fold(StreamChannels::empty(), |all, (flag, _)| all | *flag);
        assert_eq!(
            channels,
            StreamChannels::all(),
            "bug: a channel has no name"
        );
    }

    #[test]
    fn names_come_in_flag_order() {
        assert_eq!(
            (ActivityFields::TRAINER | ActivityFields::DESCRIPTION).names(),
            ["description", "trainer"],
            "bug: field names are not listed in flag order"
        );
        assert_eq!(
            (StreamChannels::LATLNG | StreamChannels::TIME).names(),
            ["timestamps", "gps_coordinates"],
            "bug: stream channel names are not listed in flag order"
        );
    }
}
