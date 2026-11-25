// ABOUTME: A cached activity row's key, its place as served (id, name, who named it), and what the athlete said of home
// ABOUTME: A served place carries no point; the declared home's point is read by the places recompute alone, never served
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A row's place, as served.
//!
//! A place is a cluster of an athlete's trimmed activity starts. What a row
//! carries is only the place's id and name: its centroid stays in
//! `athlete_places` and leaves only through the athlete's own data export.
//!
//! `PlaceRef` is deliberately not `Serialize`. Whether `name` may be shown
//! depends on the transport: a name the athlete gave is served everywhere, an
//! automatic one (a city or a segment) only first-party. A serializer here
//! would bypass that check, so the response builder renders it explicitly.
//!
//! [`HomeDeclaration`] is what the athlete said about home (carnet#1593). Its
//! point is a trimmed start or a place centroid, never a raw start, and only
//! the places recompute reads it (`check-activity-writers.sh`). Its `Debug`
//! names the state alone, so a `{:?}` in a log never prints the point.

use std::fmt;

use dravr_tronc::dates::Timestamp;

/// A `cached_activities` row's key: the provider key it is filed under and
/// the activity id. Ordered by provider, then activity id.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CachedRowKey {
    /// The provider key the cached row is stored under.
    pub provider: String,
    /// The activity id within that provider.
    pub activity_id: String,
}

/// A row's place: its id, its name if it has one, and whether the athlete
/// gave that name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaceRef {
    /// The place id (a uuid, kept across re-clustering).
    pub id: String,
    /// The place name: the athlete's own, else an automatic city or segment
    /// name, else none.
    pub name: Option<String>,
    /// Whether `name` is one the athlete gave (`name_source = athlete`).
    pub athlete_named: bool,
}

/// What the athlete said about home. No statement reads as `Inferred`.
#[derive(Clone, Copy, PartialEq)]
pub enum HomeDeclaration {
    /// Nothing said, or "forget": the clustering infers home.
    Inferred,
    /// Home is around this seed: a trimmed start or a place centroid, in
    /// degrees. Never served.
    At {
        /// Latitude in degrees.
        lat: f64,
        /// Longitude in degrees.
        lon: f64,
    },
    /// Home is not where the athlete trains: nothing is excluded.
    Elsewhere,
}

impl fmt::Debug for HomeDeclaration {
    /// The state alone; the seed is never formatted.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Inferred => "Inferred",
            Self::At { .. } => "At",
            Self::Elsewhere => "Elsewhere",
        })
    }
}

/// The stored declaration and when it was made.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StoredHome {
    /// What the athlete said; `Inferred` when they never declared.
    pub home: HomeDeclaration,
    /// When they said it; `None` when they never declared.
    pub declared_at: Option<Timestamp>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_declaration_debug_never_prints_the_point() {
        let home = HomeDeclaration::At {
            lat: 46.812_345,
            lon: -71.208_765,
        };
        let stored = StoredHome {
            home,
            declared_at: None,
        };
        for printed in [
            format!("{home:?}"),
            format!("{stored:?}"),
            format!("{home:#?}"),
        ] {
            assert!(
                !printed.chars().any(|c| c.is_ascii_digit()),
                "bug: a home declaration's Debug printed its point: {printed}"
            );
            assert!(
                printed.contains("At"),
                "bug: a home declaration's Debug lost its state: {printed}"
            );
        }
    }

    #[test]
    fn row_keys_order_by_provider_then_activity_id() {
        let key = |provider: &str, id: &str| CachedRowKey {
            provider: provider.to_owned(),
            activity_id: id.to_owned(),
        };
        let mut keys = vec![key("strava", "1"), key("garmin", "9"), key("garmin", "10")];
        keys.sort();
        assert_eq!(
            keys,
            vec![key("garmin", "10"), key("garmin", "9"), key("strava", "1")],
            "bug: clustering ties break on the lowest (provider, activity_id)"
        );
    }
}
