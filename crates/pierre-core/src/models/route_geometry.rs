// ABOUTME: Where an activity's route geometry came from: the summary polyline or the recorded streams
// ABOUTME: One enum and one slug set for the source Home's stored route reads record
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The source of an activity's route geometry.
//!
//! The slugs are the ones `activity_route_tracks.source` stores, and the
//! table's CHECK constraint names exactly these two.

/// Where an activity's route geometry came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteGeometrySource {
    /// The route overview on the cached activity (Strava's `summary_polyline`).
    SummaryPolyline,
    /// The activity's recorded streams.
    Streams,
}

impl RouteGeometrySource {
    /// The slug the stored row carries.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SummaryPolyline => "summary_polyline",
            Self::Streams => "streams",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [RouteGeometrySource; 2] = [
        RouteGeometrySource::SummaryPolyline,
        RouteGeometrySource::Streams,
    ];

    /// A persisted value: `activity_route_tracks.source` rows already hold
    /// these slugs, and the table's CHECK constraint names exactly them.
    #[test]
    fn the_slugs_are_the_ones_the_stored_check_constraint_names() {
        let slugs: Vec<&str> = ALL.iter().map(|source| source.as_str()).collect();
        assert_eq!(
            slugs,
            ["summary_polyline", "streams"],
            "bug: activity_route_tracks.source CHECKs exactly these two slugs"
        );
    }
}
