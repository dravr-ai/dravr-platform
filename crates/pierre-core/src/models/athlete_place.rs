// ABOUTME: A cached activity row's key, and its place as it is served: the place id, its name and who gave the name
// ABOUTME: Never carries a centroid, radius or point; a place is never located for anyone
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

#[cfg(test)]
mod tests {
    use super::*;

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
