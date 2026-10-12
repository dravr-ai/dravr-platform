// ABOUTME: The athlete's gear ledger — the shoes they declare, the activities attributed to each, and the pair a new activity accrues to
// ABOUTME: Pure domain types, input bounds and the default-pair rule; storage lives in pierre-database, the routes in pierre-server
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The gear ledger (carnet#711).
//!
//! An athlete declares the shoes they train in. Each activity is attributed
//! to at most one pair, and a pair's distance is the distance it was declared
//! with plus the distance of every activity attributed to it. Gear is declared
//! in Dravr, never read from a provider's own gear id, and only the athlete
//! reads it.
//!
//! An activity the athlete has not attributed is given the default pair of its
//! sport ([`default_pair`]): the pair they pinned, otherwise the one they last
//! used, otherwise the one they declared last. A pair takes only activities
//! that started after it was declared, and a retired pair takes none: retiring
//! keeps its distance and history and takes it out of every choice.
//!
//! The warning distance is a fact about the pair, never a claim about the
//! athlete's body: a pair reaches it when its distance is at or past the
//! threshold the athlete set, [`DEFAULT_SHOE_WARN_DISTANCE_METERS`] unless they
//! set another.

use std::cmp::Ordering;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::sport_type_alias::sport_matches_family;
use super::SportType;
use crate::errors::{AppError, AppResult};

/// The warning distance a pair takes unless the athlete sets another: 500 km,
/// the threshold the athlete who asked for the ledger named (carnet#711).
pub const DEFAULT_SHOE_WARN_DISTANCE_METERS: f64 = 500_000.0;

/// The largest distance a pair can be declared with, or warned at: 10 000 km,
/// well past what any pair of shoes is run in.
const MAX_GEAR_DISTANCE_METERS: f64 = 10_000_000.0;

/// The longest name a pair carries, in characters.
const MAX_GEAR_NAME_CHARS: usize = 80;

/// The most pairs one athlete can declare, retired pairs included. It bounds
/// the ledger every read returns whole.
pub const MAX_GEAR_PER_ATHLETE: i64 = 100;

/// What a piece of gear is. Stored as [`GearKind::as_str`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GearKind {
    /// A pair of shoes.
    Shoe,
}

impl GearKind {
    /// The stored and wire spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Shoe => "shoe",
        }
    }

    /// The kind a stored spelling names, `None` for any other text.
    #[must_use]
    pub fn parse(stored: &str) -> Option<Self> {
        match stored {
            "shoe" => Some(Self::Shoe),
            _ => None,
        }
    }
}

/// How an activity came to be attributed to a pair. Stored as
/// [`AttributionSource::as_str`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttributionSource {
    /// The athlete chose the pair, or chose none, for this activity.
    Declared,
    /// The ledger gave the activity its sport's default pair.
    Default,
}

impl AttributionSource {
    /// The stored and wire spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Declared => "declared",
            Self::Default => "default",
        }
    }

    /// The source a stored spelling names, `None` for any other text.
    #[must_use]
    pub fn parse(stored: &str) -> Option<Self> {
        match stored {
            "declared" => Some(Self::Declared),
            "default" => Some(Self::Default),
            _ => None,
        }
    }
}

/// The spelling a shoe sport is stored and sent under — the sport's own serde
/// name — or `None` for a sport no pair of shoes is declared for.
///
/// A road pair is declared for `run` and also takes treadmill and trail runs
/// through [`sport_matches_family`]; a trail pair is declared for
/// `trail_running` and takes trail runs only.
#[must_use]
pub const fn shoe_sport_slug(sport: &SportType) -> Option<&'static str> {
    match sport {
        SportType::Run => Some("run"),
        SportType::TrailRunning => Some("trail_running"),
        SportType::Walk => Some("walk"),
        SportType::Hike => Some("hike"),
        _ => None,
    }
}

/// The shoe sport a stored or submitted spelling names, `None` for any other
/// text.
#[must_use]
pub fn parse_shoe_sport(slug: &str) -> Option<SportType> {
    match slug {
        "run" => Some(SportType::Run),
        "trail_running" => Some(SportType::TrailRunning),
        "walk" => Some(SportType::Walk),
        "hike" => Some(SportType::Hike),
        _ => None,
    }
}

/// What the athlete declares about a pair: everything but its history.
#[derive(Debug, Clone, PartialEq)]
pub struct GearDeclaration {
    /// What the gear is.
    pub kind: GearKind,
    /// The athlete's name for it.
    pub name: String,
    /// The sport it is worn for; one of the sports [`shoe_sport_slug`] names.
    pub sport: SportType,
    /// The distance it already had when declared, in metres.
    pub initial_distance_meters: f64,
    /// The distance at which the pair says it has reached its warning, in
    /// metres.
    pub warn_distance_meters: f64,
    /// The athlete pinned it as their sport's default: new activities of that
    /// sport accrue to it whatever they used last.
    pub pinned: bool,
}

impl GearDeclaration {
    /// The declaration with every field inside its bounds: a name of 1 to
    /// `MAX_GEAR_NAME_CHARS` characters once trimmed, with no control
    /// character; a shoe sport; a declared distance from 0 to
    /// `MAX_GEAR_DISTANCE_METERS`; a warning distance above 0 and at most
    /// that bound.
    ///
    /// # Errors
    ///
    /// An invalid-input error naming the first field out of bounds.
    pub fn validated(mut self) -> AppResult<Self> {
        let name = self.name.trim();
        if name.is_empty() {
            return Err(AppError::invalid_field("name", "must not be blank"));
        }
        if name.chars().count() > MAX_GEAR_NAME_CHARS {
            return Err(AppError::invalid_field(
                "name",
                &format!("must be at most {MAX_GEAR_NAME_CHARS} characters"),
            ));
        }
        if name.chars().any(char::is_control) {
            return Err(AppError::invalid_field(
                "name",
                "must not contain control characters",
            ));
        }
        self.name = name.to_owned();
        if shoe_sport_slug(&self.sport).is_none() {
            return Err(AppError::invalid_field(
                "sport",
                "must be one of run, trail_running, walk, hike",
            ));
        }
        if !(0.0..=MAX_GEAR_DISTANCE_METERS).contains(&self.initial_distance_meters) {
            return Err(AppError::invalid_field(
                "initial_distance_meters",
                &format!("must be between 0 and {MAX_GEAR_DISTANCE_METERS}"),
            ));
        }
        let warn_in_bounds = self.warn_distance_meters > 0.0
            && self.warn_distance_meters <= MAX_GEAR_DISTANCE_METERS;
        if !warn_in_bounds {
            return Err(AppError::invalid_field(
                "warn_distance_meters",
                &format!("must be above 0 and at most {MAX_GEAR_DISTANCE_METERS}"),
            ));
        }
        Ok(self)
    }
}

/// One pair in the athlete's ledger, with its history.
#[derive(Debug, Clone, PartialEq)]
pub struct AthleteGear {
    /// The pair's id.
    pub id: Uuid,
    /// What the athlete declared about it.
    pub declaration: GearDeclaration,
    /// When the athlete declared it; it takes no activity that started before.
    pub added_at: DateTime<Utc>,
    /// When the athlete retired it, `None` while it is in use.
    pub retired_at: Option<DateTime<Utc>>,
    /// Distance of the activities attributed to it, in metres.
    pub attributed_distance_meters: f64,
    /// How many activities are attributed to it.
    pub activity_count: u64,
    /// When the latest activity attributed to it started.
    pub last_used_at: Option<DateTime<Utc>>,
}

impl AthleteGear {
    /// The pair's distance: what it was declared with plus every activity
    /// attributed to it, in metres.
    #[must_use]
    pub fn distance_meters(&self) -> f64 {
        self.declaration.initial_distance_meters + self.attributed_distance_meters
    }

    /// Whether the pair's distance is at or past its warning distance.
    #[must_use]
    pub fn warn_reached(&self) -> bool {
        self.distance_meters() >= self.declaration.warn_distance_meters
    }

    /// Whether the athlete retired the pair.
    #[must_use]
    pub const fn is_retired(&self) -> bool {
        self.retired_at.is_some()
    }

    /// Whether an activity of `sport` that started at `start` can take this
    /// pair as its default: the pair is in use, was declared by then, and is
    /// worn for that sport or the family it heads.
    fn takes(&self, sport: &SportType, start: DateTime<Utc>) -> bool {
        !self.is_retired()
            && self.added_at <= start
            && sport_matches_family(sport, &self.declaration.sport)
    }
}

/// The pair an activity of `sport` that started at `start` accrues to when the
/// athlete has not chosen one, or `None` when no pair takes it.
///
/// A pair takes it when the pair is in use, was declared by `start`, and is
/// worn for `sport` or the family `sport` belongs to. Among those, a pair
/// declared for that very sport comes before one declared for the family it
/// heads — a trail run goes to the trail pair when there is one, and to the
/// road pair otherwise. Then the pinned pair, then the one used last, then the
/// one declared last.
#[must_use]
pub fn default_pair<'a>(
    ledger: &'a [AthleteGear],
    sport: &SportType,
    start: DateTime<Utc>,
) -> Option<&'a AthleteGear> {
    ledger
        .iter()
        .filter(|pair| pair.takes(sport, start))
        .max_by(|a, b| default_order(a, b, sport))
}

/// The order [`default_pair`] ranks two candidate pairs in, best last.
fn default_order(a: &AthleteGear, b: &AthleteGear, sport: &SportType) -> Ordering {
    let exact = |pair: &AthleteGear| pair.declaration.sport == *sport;
    exact(a)
        .cmp(&exact(b))
        .then_with(|| a.declaration.pinned.cmp(&b.declaration.pinned))
        .then_with(|| a.last_used_at.cmp(&b.last_used_at))
        .then_with(|| a.added_at.cmp(&b.added_at))
        .then_with(|| b.id.cmp(&a.id))
}

/// One activity copy, by the provider key the activity cache stores it under
/// and that provider's id for it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ActivityGearKey {
    /// The provider key the cached activity is stored under.
    pub provider: String,
    /// The provider's id for the activity.
    pub activity_id: String,
}

/// Which pair one activity is attributed to.
#[derive(Debug, Clone, PartialEq)]
pub struct GearAttribution {
    /// The activity copy the attribution is recorded on.
    pub key: ActivityGearKey,
    /// The pair, or `None` when the athlete declared the activity was run in
    /// no pair of theirs.
    pub gear_id: Option<Uuid>,
    /// Whether the athlete chose it or the ledger gave the default.
    pub source: AttributionSource,
    /// When the activity started.
    pub start_date: DateTime<Utc>,
    /// The activity's distance, in metres, as it stood when attributed, so a
    /// pair keeps its distance after the activity cache lets the activity go.
    pub distance_meters: f64,
}

#[cfg(test)]
mod tests {
    use std::slice;

    use chrono::TimeZone;
    use serde_json::Value;

    use super::*;

    fn at(day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, day, 7, 0, 0).unwrap()
    }

    fn pair(sport: SportType, added: u32) -> AthleteGear {
        AthleteGear {
            id: Uuid::new_v4(),
            declaration: GearDeclaration {
                kind: GearKind::Shoe,
                name: "Pair".to_owned(),
                sport,
                initial_distance_meters: 0.0,
                warn_distance_meters: DEFAULT_SHOE_WARN_DISTANCE_METERS,
                pinned: false,
            },
            added_at: at(added),
            retired_at: None,
            attributed_distance_meters: 0.0,
            activity_count: 0,
            last_used_at: None,
        }
    }

    fn chosen(ledger: &[AthleteGear], sport: &SportType, day: u32) -> Option<Uuid> {
        default_pair(ledger, sport, at(day)).map(|p| p.id)
    }

    #[test]
    fn the_last_used_pair_is_the_default_unless_another_is_pinned() {
        let mut used = pair(SportType::Run, 1);
        used.last_used_at = Some(at(10));
        let mut earlier = pair(SportType::Run, 2);
        earlier.last_used_at = Some(at(5));
        let mut ledger = vec![earlier, used];
        assert_eq!(chosen(&ledger, &SportType::Run, 20), Some(ledger[1].id));

        ledger[0].declaration.pinned = true;
        assert_eq!(chosen(&ledger, &SportType::Run, 20), Some(ledger[0].id));
    }

    #[test]
    fn with_nothing_used_the_pair_declared_last_is_the_default() {
        let ledger = vec![pair(SportType::Run, 1), pair(SportType::Run, 3)];
        assert_eq!(chosen(&ledger, &SportType::Run, 20), Some(ledger[1].id));

        let mut ledger = ledger;
        ledger[0].last_used_at = Some(at(2));
        assert_eq!(
            chosen(&ledger, &SportType::Run, 20),
            Some(ledger[0].id),
            "a pair used once outranks one never used"
        );
    }

    #[test]
    fn a_trail_run_takes_the_trail_pair_and_the_road_pair_when_there_is_none() {
        let mut road = pair(SportType::Run, 1);
        road.declaration.pinned = true;
        road.last_used_at = Some(at(15));
        let trail = pair(SportType::TrailRunning, 1);
        let ledger = vec![road.clone(), trail.clone()];
        assert_eq!(
            chosen(&ledger, &SportType::TrailRunning, 20),
            Some(trail.id)
        );
        assert_eq!(chosen(&ledger, &SportType::Run, 20), Some(road.id));
        assert_eq!(
            chosen(&[road.clone()], &SportType::TrailRunning, 20),
            Some(road.id)
        );
        assert_eq!(
            chosen(&[road.clone()], &SportType::VirtualRun, 20),
            Some(road.id),
            "a treadmill run goes to the road pair"
        );
        assert_eq!(
            chosen(&[trail], &SportType::Run, 20),
            None,
            "a trail pair takes no road run"
        );
        assert_eq!(chosen(&[road], &SportType::Ride, 20), None);
    }

    #[test]
    fn a_pair_takes_nothing_from_before_it_was_declared_and_nothing_once_retired() {
        let declared = pair(SportType::Run, 10);
        assert_eq!(chosen(slice::from_ref(&declared), &SportType::Run, 9), None);
        assert_eq!(
            chosen(slice::from_ref(&declared), &SportType::Run, 10),
            Some(declared.id)
        );

        let mut retired = declared;
        retired.retired_at = Some(at(12));
        assert_eq!(chosen(&[retired], &SportType::Run, 20), None);
    }

    #[test]
    fn the_warning_is_reached_at_the_threshold_on_declared_plus_attributed_distance() {
        let mut shoe = pair(SportType::Run, 1);
        shoe.declaration.initial_distance_meters = 120_000.0;
        shoe.attributed_distance_meters = 379_999.0;
        assert!((shoe.distance_meters() - 499_999.0).abs() < f64::EPSILON);
        assert!(!shoe.warn_reached());
        shoe.attributed_distance_meters = 380_000.0;
        assert!(shoe.warn_reached(), "500 km on a 500 km threshold");
    }

    #[test]
    fn a_shoe_sport_is_spelled_as_the_sport_serialises() {
        for (sport, slug) in [
            (SportType::Run, "run"),
            (SportType::TrailRunning, "trail_running"),
            (SportType::Walk, "walk"),
            (SportType::Hike, "hike"),
        ] {
            assert_eq!(shoe_sport_slug(&sport), Some(slug));
            assert_eq!(
                serde_json::to_value(&sport).ok(),
                Some(Value::String(slug.to_owned()))
            );
            assert_eq!(parse_shoe_sport(slug), Some(sport));
        }
        assert_eq!(shoe_sport_slug(&SportType::VirtualRun), None);
        assert_eq!(shoe_sport_slug(&SportType::Ride), None);
        assert_eq!(parse_shoe_sport("ride"), None);
    }

    #[test]
    fn a_declaration_is_bounded() {
        let base = pair(SportType::Run, 1).declaration;
        let named = |name: &str| GearDeclaration {
            name: name.to_owned(),
            ..base.clone()
        };
        assert_eq!(
            named("  Pegasus 41  ")
                .validated()
                .ok()
                .map(|declaration| declaration.name)
                .as_deref(),
            Some("Pegasus 41")
        );
        assert!(named(&"x".repeat(MAX_GEAR_NAME_CHARS)).validated().is_ok());
        assert!(named(&"x".repeat(MAX_GEAR_NAME_CHARS + 1))
            .validated()
            .is_err());
        assert!(named("   ").validated().is_err());
        assert!(named("Peg\u{0}asus").validated().is_err());

        let ride = GearDeclaration {
            sport: SportType::Ride,
            ..base.clone()
        };
        assert!(ride.validated().is_err());

        let distances = |initial: f64, warn: f64| GearDeclaration {
            initial_distance_meters: initial,
            warn_distance_meters: warn,
            ..base.clone()
        };
        assert!(distances(0.0, 1.0).validated().is_ok());
        assert!(
            distances(MAX_GEAR_DISTANCE_METERS, MAX_GEAR_DISTANCE_METERS)
                .validated()
                .is_ok()
        );
        assert!(distances(-1.0, 500_000.0).validated().is_err());
        assert!(distances(MAX_GEAR_DISTANCE_METERS + 1.0, 500_000.0)
            .validated()
            .is_err());
        assert!(distances(0.0, 0.0).validated().is_err());
        assert!(distances(0.0, MAX_GEAR_DISTANCE_METERS + 1.0)
            .validated()
            .is_err());
        assert!(distances(f64::NAN, 500_000.0).validated().is_err());
    }

    #[test]
    fn the_stored_spellings_round_trip() {
        assert_eq!(
            GearKind::parse(GearKind::Shoe.as_str()),
            Some(GearKind::Shoe)
        );
        assert_eq!(GearKind::parse("bike"), None);
        for source in [AttributionSource::Declared, AttributionSource::Default] {
            assert_eq!(AttributionSource::parse(source.as_str()), Some(source));
        }
        assert_eq!(AttributionSource::parse("guess"), None);
        // The serde spelling is the stored one, so the wire and the column agree.
        assert_eq!(
            serde_json::to_value(AttributionSource::Default).ok(),
            Some(Value::String("default".to_owned()))
        );
    }
}
