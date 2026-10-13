// ABOUTME: Chains of same-named OSM ways joined end to end — the longest one measures a trail and draws it
// ABOUTME: Ways meet where they share an end node; a chain never passes an end twice, so it exists on the ground
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashMap;

use super::OverpassPoint;
use crate::routes::haversine_meters_between;

/// OSM stores a coordinate to seven decimal places; multiplying by this
/// turns one into the whole number two ways meeting at a node share.
const OSM_COORDINATE_SCALE: f64 = 1e7;

/// Walk steps one name's chain search may take before it settles for the
/// longest chain found. A name rarely has more than a few dozen fetched
/// segments, which an exhaustive search covers in hundreds of steps; the cap
/// is for a densely braided network, where the search grows exponentially
/// and a tool call cannot wait for it.
const MAX_CHAIN_SEARCH_STEPS: usize = 20_000;

/// Length in metres along a line of points, summed great-circle leg by leg.
///
/// A gap in the line (Overpass writes `null` for a node it clipped away)
/// ends one run of legs and starts the next, so no leg is drawn across it.
fn polyline_length_meters(points: &[Option<OverpassPoint>]) -> f64 {
    points
        .windows(2)
        .filter_map(|pair| match (&pair[0], &pair[1]) {
            (Some(from), Some(to)) => {
                Some(haversine_meters_between(from.lat, from.lon, to.lat, to.lon))
            }
            _ => None,
        })
        .sum()
}

/// Where a way's line starts and ends, as a comparable key.
///
/// Two ways that meet share a node, and a node has one pair of coordinates,
/// so equal keys mean the ways connect end to end.
type EndPoint = (i64, i64);

/// One fetched way as a link in a chain: its two ends, its length, and the
/// line between them.
#[derive(Debug, Clone)]
pub(super) struct WaySegment {
    start: EndPoint,
    end: EndPoint,
    meters: f64,
    /// The way's nodes as `(latitude, longitude)`, in the order OSM lists
    /// them. A node the response clipped away is left out, so a gap would be
    /// drawn straight across; the queries here ask for whole ways (`geom`
    /// with no bounding box), so Overpass writes none.
    points: Vec<(f64, f64)>,
}

/// A point as an [`EndPoint`], at the precision OSM stores coordinates in.
fn end_point(point: &OverpassPoint) -> EndPoint {
    // A coordinate times 1e7 is at most 1.8e9 in magnitude: it fits.
    #[allow(clippy::cast_possible_truncation)]
    let key = |degrees: f64| (degrees * OSM_COORDINATE_SCALE).round() as i64;
    (key(point.lat), key(point.lon))
}

/// A way's geometry as a chain link, or `None` when the response carried no
/// line for it or the line has no length.
pub(super) fn way_segment(geometry: Option<&[Option<OverpassPoint>]>) -> Option<WaySegment> {
    let line = geometry?;
    let start = line.iter().flatten().next()?;
    let end = line.iter().flatten().next_back()?;
    let meters = polyline_length_meters(line);
    (meters > 0.0).then(|| WaySegment {
        start: end_point(start),
        end: end_point(end),
        meters,
        points: line
            .iter()
            .flatten()
            .map(|point| (point.lat, point.lon))
            .collect(),
    })
}

/// The longest chain among ways that share a name: how long it is, and the
/// line it draws.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct MappedChain {
    /// Summed way by way, exactly as [`polyline_length_meters`] measures one.
    pub(super) meters: f64,
    /// `(latitude, longitude)` from one end of the chain to the other, each
    /// way oriented to start where the one before it ended and the node two
    /// ways share kept once.
    pub(super) coordinates: Vec<(f64, f64)>,
}

/// One way taken by a chain walk, and which way round it was walked.
#[derive(Debug, Clone, Copy)]
struct ChainLink {
    /// Index into the searched segments.
    segment: usize,
    /// Walked from its OSM end to its OSM start.
    reversed: bool,
}

/// The longest chain among ways that share a name.
///
/// A chain is a run of ways joined end to end that passes through no end
/// point twice. Ways that merely share a name — a second trail of the same
/// name across town, the opposite carriageway of a divided path — are not
/// joined to it, so they are never added in; of two ways between the same
/// pair of ends only one is taken. Every chain is a stretch that exists on
/// the ground, which is what makes its length a lower bound on the trail and
/// its line something a map can draw without inventing a connection.
///
/// The search is exhaustive up to [`MAX_CHAIN_SEARCH_STEPS`]; past that it
/// returns the longest chain it has found, which is still a real one.
pub(super) fn longest_chain(segments: &[WaySegment]) -> Option<MappedChain> {
    let mut search = ChainSearch {
        segments,
        steps_left: MAX_CHAIN_SEARCH_STEPS,
        best: 0.0,
        walk: Vec::new(),
        best_walk: Vec::new(),
    };
    // Start from ends that only one way touches first: on an unbranched trail
    // those are its two extremities, and the first walk is already the answer.
    let mut touches: HashMap<EndPoint, usize> = HashMap::new();
    for segment in segments {
        *touches.entry(segment.start).or_insert(0) += 1;
        *touches.entry(segment.end).or_insert(0) += 1;
    }
    let mut starts: Vec<EndPoint> = touches.keys().copied().collect();
    starts.sort_by_key(|point| (touches.get(point).copied().unwrap_or(0), *point));
    for start in starts {
        let mut visited = vec![start];
        search.extend(start, 0.0, &mut visited);
    }
    (search.best > 0.0).then(|| MappedChain {
        meters: search.best,
        coordinates: chain_coordinates(segments, &search.best_walk),
    })
}

/// The line a chain walk draws: its ways in walk order, each oriented the
/// way it was walked, with the node consecutive ways share kept once.
fn chain_coordinates(segments: &[WaySegment], walk: &[ChainLink]) -> Vec<(f64, f64)> {
    let mut line: Vec<(f64, f64)> = Vec::new();
    for link in walk {
        let points = &segments[link.segment].points;
        // Every way after the first starts on the node the line ends on.
        let shared = usize::from(!line.is_empty());
        if link.reversed {
            line.extend(points.iter().rev().skip(shared).copied());
        } else {
            line.extend(points.iter().skip(shared).copied());
        }
    }
    line
}

/// The state of one [`longest_chain`] search.
struct ChainSearch<'a> {
    segments: &'a [WaySegment],
    steps_left: usize,
    best: f64,
    /// The ways the current walk has taken, in order.
    walk: Vec<ChainLink>,
    /// The walk that measured `best`, kept so the winner can be drawn.
    best_walk: Vec<ChainLink>,
}

impl ChainSearch<'_> {
    /// Walk on from `at`, having covered `meters` through `visited`.
    fn extend(&mut self, at: EndPoint, meters: f64, visited: &mut Vec<EndPoint>) {
        self.record(meters);
        let segments = self.segments;
        for (index, segment) in segments.iter().enumerate() {
            let (next, reversed) = if segment.start == at {
                (segment.end, false)
            } else if segment.end == at {
                (segment.start, true)
            } else {
                continue;
            };
            if next == at {
                // A way that closes on itself is a chain of its own.
                self.walk.push(ChainLink {
                    segment: index,
                    reversed: false,
                });
                self.record(meters + segment.meters);
                self.walk.pop();
                continue;
            }
            if visited.contains(&next) {
                continue;
            }
            if self.steps_left == 0 {
                return;
            }
            self.steps_left -= 1;
            visited.push(next);
            self.walk.push(ChainLink {
                segment: index,
                reversed,
            });
            self.extend(next, meters + segment.meters, visited);
            self.walk.pop();
            visited.pop();
        }
    }

    /// Keep the current walk when it is the longest yet. Strictly longer, so
    /// of two chains of equal length the first found is the one drawn.
    fn record(&mut self, meters: f64) {
        if meters > self.best {
            self.best = meters;
            self.best_walk.clone_from(&self.walk);
        }
    }
}
