// ABOUTME: Stitches mapped trail stretches into one drawable itinerary, joined only where they meet on the ground
// ABOUTME: Also derives the route_ref a drawable public line is stored and named by

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Composed routes
//!
//! Route discovery answers with stretches: one named trail each, drawn along
//! the longest chain of its mapped ways. A session often wants more than one —
//! the riverside path out, the rail trail on, the park loop to finish — and
//! [`compose`] stitches them into one line, in the order given.
//!
//! Stretches are joined only where they meet: where the line so far ends
//! within [`JOIN_TOLERANCE_METERS`] of the next stretch. That join is drawn as
//! the straight few metres it is. A wider gap is refused rather than bridged,
//! because a straight line across it may cross a canal, a fence or a road that
//! the map does not show as passable, and a coach must never send an athlete
//! across one.
//!
//! A stretch may be met in its middle — a trail ending at a T junction with
//! another — and is then followed from the junction toward the stretch that
//! comes next. [`meets`] answers the same question for two lines on their own,
//! which is what lets route discovery say which stretches can follow which.
//!
//! Every drawable public line, a single stretch or a composition, is named by
//! a [`route_ref`] derived from its coordinates: the model carries a short
//! handle, and the platform draws the line the handle names.

use std::cmp::Ordering;
use std::error::Error;
use std::fmt;

use sha2::{Digest, Sha256};

use crate::route_track::{nearest_on_chord, project_about};
use crate::routes::haversine_meters_between;

/// How close two stretches must come for the route to pass from one to the
/// other.
///
/// Two mapped ways that meet on the ground share a node, or end a few metres
/// apart where a mapper stopped one short of the other. A wider gap is not
/// bridged with a straight line, because on the ground it may be a canal, a
/// fence or a road.
pub const JOIN_TOLERANCE_METERS: f64 = 30.0;

/// Most stretches one composed route stitches together.
///
/// Eight named trails is already an itinerary nobody follows without a map in
/// hand; past that the composition is better split into two sessions, and the
/// bound keeps the pairwise distance search small.
pub const MAX_STRETCHES: usize = 8;

/// Most mapped points one composition takes in, over all its stretches.
///
/// Finding where two stretches meet measures every point of each against
/// every segment of the other, so the work grows with the product of their
/// sizes and runs inside the tool call. A discovered stretch is the chain of
/// one trail's ways inside the search, a few thousand points at most, and
/// eight of those fit. What does not fit is a composition fed back in as a
/// stretch of another, and that one again: each round multiplies the points,
/// and the joins of the third would hold the call for minutes.
const MAX_COMPOSED_POINTS: usize = 20_000;

/// What every route reference starts with, so one reads as a reference in a
/// tool argument and is never mistaken for an activity id.
const ROUTE_REF_PREFIX: &str = "rt_";

/// Bytes of the digest a route reference keeps: 64 bits, written as 16 hex
/// digits. Refs are only compared within one athlete's own store, where a
/// collision among a day's routes is not a practical concern.
const ROUTE_REF_DIGEST_BYTES: usize = 8;

/// Scale a coordinate is quantized at before it is hashed: a millionth of a
/// degree, about 11 cm. Finer than any mapped node is placed, coarse enough
/// that a line read back through JSON hashes to the same reference.
const ROUTE_REF_SCALE: f64 = 1e6;

/// One mapped stretch to compose, in walking order with the others.
#[derive(Debug, Clone, Copy)]
pub struct Stretch<'a> {
    /// The stretch's name, as an error names it.
    pub name: &'a str,
    /// Its line as `(latitude, longitude)` in degrees, in either direction:
    /// [`compose`] decides which way it is walked.
    pub coordinates: &'a [(f64, f64)],
}

/// Stretches stitched into one line.
#[derive(Debug, Clone, PartialEq)]
pub struct ComposedRoute {
    /// `(latitude, longitude)` from the start of the route to its finish,
    /// out and back included when it was asked for.
    pub coordinates: Vec<(f64, f64)>,
    /// Length of `coordinates`, summed great-circle leg by leg — twice the
    /// one-way length for an out-and-back.
    pub length_meters: f64,
    /// Metres the line crosses at each join, one per pair of consecutive
    /// stretches; each is within [`JOIN_TOLERANCE_METERS`].
    pub joins_meters: Vec<f64>,
}

/// Why stretches cannot be composed. Each message is a sentence the model
/// reading the tool result can act on.
#[derive(Debug, Clone, PartialEq)]
pub enum ComposeError {
    /// No stretch was given.
    NoStretches,
    /// More stretches than [`MAX_STRETCHES`] were given.
    TooManyStretches {
        /// The most one route takes.
        max: usize,
    },
    /// The stretches carry more mapped points between them than one
    /// composition takes in.
    TooManyPoints {
        /// The most points one composition takes in.
        max: usize,
    },
    /// A stretch would add no distance: it has fewer than two mapped points or
    /// no length, or the route would enter and leave it at the same point.
    StretchTooShort {
        /// The stretch's name.
        name: String,
    },
    /// Consecutive stretches do not come within [`JOIN_TOLERANCE_METERS`] of
    /// each other.
    Gap {
        /// The stretch the route would leave.
        from: String,
        /// The stretch the route would join.
        to: String,
        /// How far the line would have to cross unmapped ground.
        gap_meters: f64,
    },
}

impl fmt::Display for ComposeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoStretches => f.write_str(
                "no stretches were given; a route is composed from at least one discovered stretch",
            ),
            Self::TooManyStretches { max } => write!(
                f,
                "a composed route takes at most {max} stretches; drop the ones that add least, \
                 or split the session into two routes"
            ),
            Self::TooManyPoints { max } => write!(
                f,
                "these stretches carry more than {max} mapped points between them, more than one \
                 route is composed from; compose the discovered stretches themselves rather than \
                 routes already composed from them, or split the session into two routes"
            ),
            Self::StretchTooShort { name } => write!(
                f,
                "\"{name}\" adds no distance to the route: it has no mapped length, or the route \
                 would enter and leave it at the same point; leave it out"
            ),
            Self::Gap {
                from,
                to,
                gap_meters,
            } => write!(
                f,
                "\"{from}\" and \"{to}\" do not meet: the route would cross {gap_meters:.0} m of \
                 unmapped ground between them, more than the {JOIN_TOLERANCE_METERS:.0} m a join \
                 may bridge; put a stretch that connects them in between, or reorder the \
                 stretches so each one meets the next"
            ),
        }
    }
}

impl Error for ComposeError {}

/// Stitch `stretches` into one line, followed in the order given.
///
/// - The first stretch is walked toward the next: it ends at its end nearest
///   stretch two. A single stretch is walked as it is mapped.
/// - Each later stretch is entered at its point nearest the end of the line so
///   far, which must lie within [`JOIN_TOLERANCE_METERS`]; entering in its
///   middle (a T junction) cuts it there.
/// - It is then followed toward its point nearest the stretch after it, to
///   the end on that side — or, when that end does not meet the next stretch
///   but the next stretch meets this one in its middle, to that junction. The
///   last stretch runs on to the end that leaves more of it.
/// - `out_and_back` returns along the whole line, the turn point drawn once.
///
/// The junction exit is what keeps this in step with [`meets`]: two stretches
/// that meet, by either one's end touching the other, can follow each other.
/// So does the frame: every decision between two consecutive stretches is
/// measured on the projection `meets` measures that pair on, so a pair it
/// finds a millimetre inside the tolerance is not refused here as a gap.
///
/// # Errors
///
/// [`ComposeError::NoStretches`] and [`ComposeError::TooManyStretches`] for
/// a count outside 1 to [`MAX_STRETCHES`]; [`ComposeError::TooManyPoints`]
/// for stretches past [`MAX_COMPOSED_POINTS`] between them;
/// [`ComposeError::StretchTooShort`] for a stretch that would add no
/// distance; [`ComposeError::Gap`] for consecutive stretches that do not meet.
pub fn compose(
    stretches: &[Stretch<'_>],
    out_and_back: bool,
) -> Result<ComposedRoute, ComposeError> {
    if stretches.is_empty() {
        return Err(ComposeError::NoStretches);
    }
    if stretches.len() > MAX_STRETCHES {
        return Err(ComposeError::TooManyStretches { max: MAX_STRETCHES });
    }
    let points: usize = stretches
        .iter()
        .map(|stretch| stretch.coordinates.len())
        .sum();
    if points > MAX_COMPOSED_POINTS {
        return Err(ComposeError::TooManyPoints {
            max: MAX_COMPOSED_POINTS,
        });
    }
    if let Some(short) = stretches
        .iter()
        .find(|stretch| stretch.coordinates.len() < 2 || !has_length(stretch.coordinates))
    {
        return Err(too_short(short));
    }
    // The frame each consecutive pair is measured on: the pair's own mean
    // latitude, exactly as `meets` projects it. One frame for the whole route
    // would scale every pair's east-west metres by a slightly different
    // factor, enough to refuse at the edge of the tolerance a pair `meets`
    // accepted.
    let frames: Vec<f64> = stretches
        .windows(2)
        .map(|pair| mean_latitude(pair[0].coordinates.iter().chain(pair[1].coordinates)))
        .collect();

    let mut line: Vec<(f64, f64)> = Vec::new();
    let mut joins_meters = Vec::with_capacity(stretches.len() - 1);
    for (index, stretch) in stretches.iter().enumerate() {
        // This stretch and the next, on their pair's frame.
        let ahead = frames.get(index).copied();
        let next =
            ahead.map(|reference| project_about(stretches[index + 1].coordinates, reference));
        let (from, to) = match line.last() {
            None => {
                let reference = ahead.unwrap_or_else(|| mean_latitude(stretch.coordinates.iter()));
                let here = project_about(stretch.coordinates, reference);
                (
                    opening_start(&here, next.as_deref()),
                    opening_finish(&here, next.as_deref()),
                )
            }
            Some(&end) => {
                // Entered on the frame of the pair it joins from.
                let behind = frames[index - 1];
                let entered = project_about(stretch.coordinates, behind);
                let (entry, gap) = nearest_station(&entered, project_point(end, behind));
                if gap > JOIN_TOLERANCE_METERS {
                    return Err(ComposeError::Gap {
                        from: stretches[index - 1].name.to_owned(),
                        to: stretch.name.to_owned(),
                        gap_meters: gap,
                    });
                }
                joins_meters.push(gap);
                let here = ahead.map_or(entered, |reference| {
                    project_about(stretch.coordinates, reference)
                });
                (entry, onward_finish(&here, entry, next.as_deref()))
            }
        };
        let part = sub_line(stretch.coordinates, from, to);
        if !has_length(&part) {
            return Err(too_short(stretch));
        }
        // A join that lands on the node the line ends on adds no point.
        let shared = usize::from(line.last() == part.first());
        line.extend(part.into_iter().skip(shared));
    }
    if out_and_back {
        let back: Vec<(f64, f64)> = line.iter().rev().skip(1).copied().collect();
        line.extend(back);
    }
    Ok(ComposedRoute {
        length_meters: line_meters(&line),
        coordinates: line,
        joins_meters,
    })
}

/// Whether one line can follow the other: an end of either comes within
/// [`JOIN_TOLERANCE_METERS`] of any point of the other.
///
/// Measured to the nearest point of each segment, not only to the mapped
/// nodes, so a long straight stretch whose nodes sit hundreds of metres apart
/// is still met where a trail ends against its middle. An empty line meets
/// nothing.
#[must_use]
pub fn meets(a: &[(f64, f64)], b: &[(f64, f64)]) -> bool {
    if a.is_empty() || b.is_empty() {
        return false;
    }
    let reference = mean_latitude(a.iter().chain(b));
    let (planar_a, planar_b) = (project_about(a, reference), project_about(b, reference));
    end_meets(&planar_a, &planar_b) || end_meets(&planar_b, &planar_a)
}

/// The reference a drawable public line is stored and named by: `rt_` and
/// 16 hex digits, the first 8 bytes of a SHA-256 over the line.
///
/// Each point is quantized to a millionth of a degree and hashed as two
/// little-endian `i64`s, latitude then longitude, in order; one trailing byte
/// says whether the line is an out-and-back (a discovered stretch is never
/// one). The same line always gets the same reference, so a route found
/// twice is stored once, and a reference cannot be forged into a line the
/// platform never computed — it is only ever looked up.
#[must_use]
pub fn route_ref(coordinates: &[(f64, f64)], out_and_back: bool) -> String {
    let mut hasher = Sha256::new();
    for &(latitude, longitude) in coordinates {
        hasher.update(quantized(latitude).to_le_bytes());
        hasher.update(quantized(longitude).to_le_bytes());
    }
    hasher.update([u8::from(out_and_back)]);
    let digest = hasher.finalize();
    format!(
        "{ROUTE_REF_PREFIX}{}",
        hex::encode(&digest[..ROUTE_REF_DIGEST_BYTES])
    )
}

/// A coordinate in millionths of a degree. A degree is at most 180 in
/// magnitude, so the product fits an `i64` with room to spare.
fn quantized(degrees: f64) -> i64 {
    (degrees * ROUTE_REF_SCALE).round() as i64
}

fn too_short(stretch: &Stretch<'_>) -> ComposeError {
    ComposeError::StretchTooShort {
        name: stretch.name.to_owned(),
    }
}

// ============================================================================
// Choosing the part of each stretch the route follows
//
// A place on a line is a station: the index of the segment it lies on plus
// the share of the way along it, so 0 is the first point, `len - 1` the last,
// and 2.5 halfway between the third and fourth. Stations compare in walking
// order, which is what every choice below is made in. They are found on the
// planar projection and read back on the degrees, which a linear projection
// keeps in step.
// ============================================================================

/// Where the first stretch starts: the end away from the stretch after it.
///
/// When an end meets the next stretch, the nearer end is where the route
/// hands over and the other is the start. When neither does, the route hands
/// over at the junction in the middle (see [`opening_finish`]) and starts from
/// the end that leaves more of the stretch before it.
fn opening_start(here: &[(f64, f64)], next: Option<&[(f64, f64)]>) -> f64 {
    let last = last_station(here);
    let Some(next) = next else {
        return 0.0;
    };
    if let Some(handover) = nearer_meeting_end(here, next) {
        return last - handover;
    }
    let (junction, _) = closest_approach(here, next);
    if planar_meters(here, junction, 0.0) >= planar_meters(here, junction, last) {
        0.0
    } else {
        last
    }
}

/// Where the first stretch hands over to the next: its end nearer the next
/// stretch when either end meets it, otherwise where the two come closest.
/// A single stretch runs to its last mapped point.
fn opening_finish(here: &[(f64, f64)], next: Option<&[(f64, f64)]>) -> f64 {
    let Some(next) = next else {
        return last_station(here);
    };
    nearer_meeting_end(here, next).unwrap_or_else(|| closest_approach(here, next).0)
}

/// The end of `here` nearer `next`, as a station, when either end meets it.
fn nearer_meeting_end(here: &[(f64, f64)], next: &[(f64, f64)]) -> Option<f64> {
    let last = last_station(here);
    let first_gap = nearest_station(next, here[0]).1;
    let last_gap = nearest_station(next, here[here.len() - 1]).1;
    if first_gap.min(last_gap) > JOIN_TOLERANCE_METERS {
        return None;
    }
    Some(if first_gap < last_gap { 0.0 } else { last })
}

/// Where a stretch entered at `entry` is left.
///
/// Toward its point nearest the next stretch, to the end on that side when
/// that end meets the next stretch, and to the point itself when it does not
/// — the next stretch then leaves from this one's middle. When the route
/// enters right where the next stretch is nearest, it runs to whichever end
/// meeting the next stretch leaves more of this one, or adds nothing. The
/// last stretch runs to the end that leaves more of it.
fn onward_finish(here: &[(f64, f64)], entry: f64, next: Option<&[(f64, f64)]>) -> f64 {
    let last = last_station(here);
    let Some(next) = next else {
        return if planar_meters(here, entry, 0.0) > planar_meters(here, entry, last) {
            0.0
        } else {
            last
        };
    };
    let (exit, _) = closest_approach(here, next);
    let toward = match exit.total_cmp(&entry) {
        Ordering::Less => 0.0,
        Ordering::Greater => last,
        Ordering::Equal => {
            return [0.0, last]
                .into_iter()
                .filter(|&end| station_meets(here, end, next))
                .max_by(|&a, &b| {
                    planar_meters(here, entry, a).total_cmp(&planar_meters(here, entry, b))
                })
                .unwrap_or(entry);
        }
    };
    if station_meets(here, toward, next) {
        toward
    } else {
        exit
    }
}

/// Whether the point at `station` of `here` lies within the join tolerance
/// of `next`.
fn station_meets(here: &[(f64, f64)], station: f64, next: &[(f64, f64)]) -> bool {
    nearest_station(next, point_at(here, station)).1 <= JOIN_TOLERANCE_METERS
}

/// Whether an end of `line` lies within the join tolerance of `other`.
fn end_meets(line: &[(f64, f64)], other: &[(f64, f64)]) -> bool {
    [line[0], line[line.len() - 1]]
        .into_iter()
        .any(|end| nearest_station(other, end).1 <= JOIN_TOLERANCE_METERS)
}

/// The station of `here` where it comes closest to `other`, and how close.
///
/// Every node of each line is measured against the other's segments. Two
/// mapped ways that cross on the ground share a node there, so a crossing
/// with no node near it is a bridge or a tunnel, which is no junction.
fn closest_approach(here: &[(f64, f64)], other: &[(f64, f64)]) -> (f64, f64) {
    let mut closest = (0.0, f64::INFINITY);
    for (index, &node) in here.iter().enumerate() {
        let meters = nearest_station(other, node).1;
        if meters < closest.1 {
            closest = (index as f64, meters);
        }
    }
    for &node in other {
        let found = nearest_station(here, node);
        if found.1 < closest.1 {
            closest = found;
        }
    }
    closest
}

/// The station of `line` nearest `point`, and how far that is, in planar
/// metres. Of equally near places the first along the line wins.
fn nearest_station(line: &[(f64, f64)], point: (f64, f64)) -> (f64, f64) {
    let mut nearest = (0.0, planar_distance(point, line[0]));
    for (index, pair) in line.windows(2).enumerate() {
        let (along, meters) = nearest_on_chord(point, pair[0], pair[1]);
        if meters < nearest.1 {
            nearest = (index as f64 + along, meters);
        }
    }
    nearest
}

/// The station of a line's last point.
fn last_station(line: &[(f64, f64)]) -> f64 {
    (line.len() - 1) as f64
}

/// The point at `station`, interpolated within its segment.
fn point_at(line: &[(f64, f64)], station: f64) -> (f64, f64) {
    let segment = (station.max(0.0).floor() as usize).min(line.len() - 2);
    let along = station - segment as f64;
    let (a, b) = (line[segment], line[segment + 1]);
    if along <= 0.0 {
        a
    } else if along >= 1.0 {
        b
    } else {
        (along.mul_add(b.0 - a.0, a.0), along.mul_add(b.1 - a.1, a.1))
    }
}

/// The part of `line` from station `from` to station `to`, in that direction:
/// the two cut points and every node strictly between them.
fn sub_line(line: &[(f64, f64)], from: f64, to: f64) -> Vec<(f64, f64)> {
    let (low, high) = (from.min(to), from.max(to));
    let between = (0..line.len())
        .filter(|&index| {
            let station = index as f64;
            station > low && station < high
        })
        .map(|index| line[index]);
    let mut part = vec![point_at(line, from)];
    if from <= to {
        part.extend(between);
    } else {
        part.extend(between.rev());
    }
    part.push(point_at(line, to));
    // A cut on a node yields that node twice.
    part.dedup();
    part
}

/// Planar length of the part of `line` between two stations.
fn planar_meters(line: &[(f64, f64)], from: f64, to: f64) -> f64 {
    sub_line(line, from, to)
        .windows(2)
        .map(|pair| planar_distance(pair[0], pair[1]))
        .sum()
}

fn planar_distance(a: (f64, f64), b: (f64, f64)) -> f64 {
    (a.0 - b.0).hypot(a.1 - b.1)
}

/// One `(latitude, longitude)` point on the planar frame `reference` sets.
fn project_point(point: (f64, f64), reference: f64) -> (f64, f64) {
    project_about(&[point], reference)[0]
}

/// The latitude a set of lines is projected about: their mean, so the frame
/// is true in the middle of the area the route covers.
fn mean_latitude<'a>(points: impl Iterator<Item = &'a (f64, f64)>) -> f64 {
    let (sum, count) = points.fold((0.0, 0_usize), |(sum, count), &(latitude, _)| {
        (sum + latitude, count + 1)
    });
    sum / count.max(1) as f64
}

/// Great-circle metres between two `(latitude, longitude)` points.
fn point_meters(a: (f64, f64), b: (f64, f64)) -> f64 {
    haversine_meters_between(a.0, a.1, b.0, b.1)
}

/// Length of a `(latitude, longitude)` line, summed great-circle leg by leg.
fn line_meters(line: &[(f64, f64)]) -> f64 {
    line.windows(2)
        .map(|pair| point_meters(pair[0], pair[1]))
        .sum()
}

/// Whether a line covers any ground. False for a non-finite coordinate too,
/// whose length is not a number.
fn has_length(line: &[(f64, f64)]) -> bool {
    line_meters(line) > 0.0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Metres in a degree of latitude on the haversine's sphere.
    const METERS_PER_DEGREE: f64 = 111_194.93;

    /// Degrees of latitude in `meters`.
    fn north(meters: f64) -> f64 {
        meters / METERS_PER_DEGREE
    }

    /// A straight line due north along `longitude`, from `south` for
    /// `meters`, with a node every 100 m.
    fn northward(longitude: f64, south: f64, meters: f64) -> Vec<(f64, f64)> {
        let nodes = (meters / 100.0).round() as usize;
        (0..=nodes)
            .map(|index| (south + north(index as f64 * 100.0), longitude))
            .collect()
    }

    /// Degrees of longitude in `meters` east at latitude 46.
    fn east(meters: f64) -> f64 {
        meters / (METERS_PER_DEGREE * 46.0_f64.to_radians().cos())
    }

    fn stretch<'a>(name: &'a str, coordinates: &'a [(f64, f64)]) -> Stretch<'a> {
        Stretch { name, coordinates }
    }

    fn assert_near(actual: f64, expected: f64, what: &str) {
        assert!(
            (actual - expected).abs() < 1.0,
            "{what}: expected {expected:.1}, got {actual:.1}"
        );
    }

    /// Two trails end to end, both mapped in walking order: one line through
    /// both, the shared node drawn once and the join zero metres.
    #[test]
    fn stretches_that_share_an_end_are_followed_end_to_end() {
        let first = northward(-72.0, 46.0, 1_000.0);
        let second = northward(-72.0, 46.0 + north(1_000.0), 500.0);
        let route = compose(&[stretch("A", &first), stretch("B", &second)], false).unwrap();
        assert_eq!(route.coordinates.first(), first.first());
        assert_eq!(route.coordinates.last(), second.last());
        assert_eq!(route.coordinates.len(), first.len() + second.len() - 1);
        assert_near(route.length_meters, 1_500.0, "length");
        assert_eq!(route.joins_meters, vec![0.0]);
    }

    /// Stretches mapped against the walking direction are turned round: the
    /// first so it ends where the second begins, the second so it leaves from
    /// that junction.
    #[test]
    fn reversed_stretches_are_walked_the_way_the_route_goes() {
        let mut first = northward(-72.0, 46.0, 1_000.0);
        first.reverse();
        let mut second = northward(-72.0, 46.0 + north(1_000.0), 500.0);
        second.reverse();
        let route = compose(&[stretch("A", &first), stretch("B", &second)], false).unwrap();
        assert_eq!(route.coordinates.first(), first.last());
        assert_eq!(route.coordinates.last(), second.first());
        assert!(route
            .coordinates
            .windows(2)
            .all(|pair| pair[0].0 < pair[1].0));
        assert_near(route.length_meters, 1_500.0, "length");
    }

    /// A trail ending against the middle of another (a T junction) enters it
    /// there; the second stretch is cut at the junction and followed toward
    /// its farther end, so its near half is not walked.
    #[test]
    fn a_t_junction_cuts_the_stretch_it_enters() {
        // An east-west street 2 km long, and a path from the south ending
        // against it 600 m from its west end.
        let street: Vec<(f64, f64)> = vec![(46.01, -72.0), (46.01, -72.0 + east(2_000.0))];
        let path = vec![(46.0, -72.0 + east(600.0)), (46.01, -72.0 + east(600.0))];
        let route = compose(&[stretch("Path", &path), stretch("Street", &street)], false).unwrap();
        assert_eq!(route.coordinates.last(), street.last());
        assert_near(
            route.length_meters,
            METERS_PER_DEGREE.mul_add(0.01, 1_400.0),
            "the path, then the 1.4 km of street beyond the junction",
        );
        assert!(route.joins_meters[0] < 1.0);
    }

    /// The direction a T-junction stretch is followed in is toward the stretch
    /// after it, even when that is the shorter way.
    #[test]
    fn a_t_junction_is_followed_toward_the_next_stretch() {
        let street: Vec<(f64, f64)> = vec![(46.01, -72.0), (46.01, -72.0 + east(2_000.0))];
        let path = vec![(46.0, -72.0 + east(600.0)), (46.01, -72.0 + east(600.0))];
        let onward = northward(-72.0, 46.01, 800.0);
        let route = compose(
            &[
                stretch("Path", &path),
                stretch("Street", &street),
                stretch("Onward", &onward),
            ],
            false,
        )
        .unwrap();
        assert_eq!(route.coordinates.last(), onward.last());
        assert_near(
            route.length_meters,
            METERS_PER_DEGREE.mul_add(0.01, 600.0 + 800.0),
            "path, 600 m of street west, then onward",
        );
        assert_eq!(route.joins_meters.len(), 2);
    }

    /// A stretch whose end butts into the middle of the one before it is
    /// reached by leaving that one at the junction, not at its far end.
    #[test]
    fn a_stretch_leaving_from_the_middle_of_another_is_reached_at_the_junction() {
        let trail = northward(-72.0, 46.0, 3_000.0);
        let spur = vec![
            (46.0 + north(1_000.0), -72.0),
            (46.0 + north(1_000.0), -72.0 + east(700.0)),
        ];
        let route = compose(&[stretch("Trail", &trail), stretch("Spur", &spur)], false).unwrap();
        assert_eq!(route.coordinates.first(), trail.last());
        assert_eq!(route.coordinates.last(), spur.last());
        assert_near(
            route.length_meters,
            2_000.0 + 700.0,
            "trail to the junction, then spur",
        );
    }

    /// Joins up to the tolerance are drawn as the straight metres they are;
    /// past it the compose fails, naming both stretches and the gap.
    #[test]
    fn a_gap_wider_than_the_tolerance_is_refused_with_its_width() {
        let first = northward(-72.0, 46.0, 1_000.0);
        let near = northward(-72.0, 46.0 + north(1_020.0), 500.0);
        let joined = compose(&[stretch("A", &first), stretch("B", &near)], false).unwrap();
        assert_near(joined.joins_meters[0], 20.0, "join");
        assert_near(joined.length_meters, 1_520.0, "length with the join");

        let far = northward(-72.0, 46.0 + north(1_412.0), 500.0);
        let error =
            compose(&[stretch("Rivière", &first), stretch("Canal", &far)], false).unwrap_err();
        match &error {
            ComposeError::Gap {
                from,
                to,
                gap_meters,
            } => {
                assert_eq!((from.as_str(), to.as_str()), ("Rivière", "Canal"));
                assert_near(*gap_meters, 412.0, "gap");
            }
            other => panic!("expected a gap, got {other:?}"),
        }
        let message = error.to_string();
        assert!(
            message.contains("\"Rivière\" and \"Canal\" do not meet"),
            "{message}"
        );
        assert!(message.contains("412 m"), "{message}");
        assert!(
            message.contains(&format!("{JOIN_TOLERANCE_METERS:.0} m a join may bridge")),
            "{message}"
        );
    }

    /// A pair `meets` accepts at the edge of the tolerance composes: both
    /// measure the pair on one projection. Here the 55 km trail sets the
    /// pair's frame near latitude 46.25 while the join sits at 46.0, so the
    /// 30.1 great-circle metres to the spur measure under 30 on it. A compose
    /// that judged the join by the great circle would refuse a pair discovery
    /// lists as connected.
    #[test]
    fn a_pair_that_meets_at_the_edge_of_the_tolerance_composes() {
        let mut trail = northward(-72.0, 46.0, 55_000.0);
        trail.reverse();
        let spur = vec![(46.0, -72.0 + east(30.1)), (46.0, -72.0 + east(530.0))];
        assert!(
            meets(&trail, &spur),
            "the spur's end is within the tolerance"
        );
        let route = compose(&[stretch("Trail", &trail), stretch("Spur", &spur)], false)
            .expect("a pair that meets composes");
        assert!(route.joins_meters[0] <= JOIN_TOLERANCE_METERS);
        assert_eq!(route.coordinates.first(), trail.first());
        assert_eq!(route.coordinates.last(), spur.last());
    }

    /// Out and back returns along the line: the turn point once, the length
    /// doubled, the finish on the start.
    #[test]
    fn out_and_back_returns_along_the_line_and_doubles_its_length() {
        let first = northward(-72.0, 46.0, 1_000.0);
        let second = northward(-72.0, 46.0 + north(1_000.0), 500.0);
        let one_way = compose(&[stretch("A", &first), stretch("B", &second)], false).unwrap();
        let both = compose(&[stretch("A", &first), stretch("B", &second)], true).unwrap();
        assert_eq!(both.coordinates.len(), one_way.coordinates.len() * 2 - 1);
        assert_eq!(both.coordinates.first(), both.coordinates.last());
        assert_near(both.length_meters, one_way.length_meters * 2.0, "length");
        let turn = one_way.coordinates.len() - 1;
        assert_ne!(both.coordinates[turn - 1], both.coordinates[turn]);
        assert_eq!(both.coordinates[turn - 1], both.coordinates[turn + 1]);
    }

    /// One stretch is the stretch, as mapped.
    #[test]
    fn a_single_stretch_is_drawn_as_mapped() {
        let mut only = northward(-72.0, 46.0, 1_000.0);
        only.reverse();
        let route = compose(&[stretch("Solo", &only)], false).unwrap();
        assert_eq!(route.coordinates, only);
        assert!(route.joins_meters.is_empty());
        assert_near(route.length_meters, 1_000.0, "length");
    }

    #[test]
    fn the_stretch_count_is_bounded() {
        assert_eq!(compose(&[], false).unwrap_err(), ComposeError::NoStretches);
        let line = northward(-72.0, 46.0, 200.0);
        let many: Vec<Stretch<'_>> = (0..=MAX_STRETCHES).map(|_| stretch("S", &line)).collect();
        let error = compose(&many, false).unwrap_err();
        assert_eq!(error, ComposeError::TooManyStretches { max: MAX_STRETCHES });
        assert!(error
            .to_string()
            .contains(&format!("at most {MAX_STRETCHES} stretches")));
    }

    /// The points a composition takes in are bounded over all its stretches:
    /// two stretches of half the budget each compose, one point more is
    /// refused before any join is searched for.
    #[test]
    fn the_points_a_composition_takes_in_are_bounded() {
        // A node every metre, so the budget is a few kilometres of line.
        let metre_by_metre = |from: usize, nodes: usize| -> Vec<(f64, f64)> {
            (from..from + nodes)
                .map(|metre| (46.0 + north(metre as f64), -72.0))
                .collect()
        };
        let half = MAX_COMPOSED_POINTS / 2;
        let first = metre_by_metre(0, half);
        let second = metre_by_metre(half - 1, half);
        let route = compose(&[stretch("A", &first), stretch("B", &second)], false)
            .expect("a composition within the budget composes");
        assert_eq!(route.coordinates.len(), MAX_COMPOSED_POINTS - 1);

        let longer = metre_by_metre(half - 1, half + 1);
        let error = compose(&[stretch("A", &first), stretch("B", &longer)], false).unwrap_err();
        assert_eq!(
            error,
            ComposeError::TooManyPoints {
                max: MAX_COMPOSED_POINTS
            }
        );
        assert!(
            error
                .to_string()
                .contains("rather than routes already composed from them"),
            "{error}"
        );
    }

    /// A stretch with no length, or one entered and left at the same point,
    /// adds nothing and is named so it can be dropped.
    #[test]
    fn a_stretch_that_adds_nothing_is_named() {
        let pin = [(46.0, -72.0)];
        assert_eq!(
            compose(&[stretch("Pin", &pin)], false).unwrap_err(),
            ComposeError::StretchTooShort {
                name: "Pin".to_owned()
            }
        );
        // Three trails meeting at one junction: the middle one is entered
        // and left at that junction, and none of it is walked.
        let junction = (46.01, -72.0);
        let west = vec![(46.0, -72.0), junction];
        let middle = vec![junction, (46.01, -72.0 + east(500.0))];
        let onward = vec![junction, (46.02, -72.0)];
        let error = compose(
            &[
                stretch("West", &west),
                stretch("Middle", &middle),
                stretch("Onward", &onward),
            ],
            false,
        )
        .unwrap_err();
        assert_eq!(
            error,
            ComposeError::StretchTooShort {
                name: "Middle".to_owned()
            }
        );
        assert!(error.to_string().contains("leave it out"));
    }

    /// Lines meet when an end of either comes within the tolerance of the
    /// other — anywhere along it, between nodes included.
    #[test]
    fn meets_reads_ends_against_the_whole_other_line() {
        let street: Vec<(f64, f64)> = vec![(46.01, -72.0), (46.01, -72.0 + east(2_000.0))];
        let touching = vec![
            (46.0, -72.0 + east(600.0)),
            (46.01 - north(25.0), -72.0 + east(600.0)),
        ];
        let short = vec![
            (46.0, -72.0 + east(600.0)),
            (46.01 - north(40.0), -72.0 + east(600.0)),
        ];
        assert!(
            meets(&touching, &street),
            "an end 25 m from the middle of a long segment"
        );
        assert!(meets(&street, &touching), "either order");
        assert!(!meets(&short, &street), "40 m short");
        // Crossing in the middle with no end near is no junction.
        let crossing = vec![(46.0, -72.0 + east(900.0)), (46.02, -72.0 + east(900.0))];
        assert!(!meets(&crossing, &street));
        assert!(!meets(&[], &street));
    }

    /// The same line always gets the same reference; any change to a point,
    /// the order or the out-and-back flag gives another.
    #[test]
    fn a_route_ref_is_a_deterministic_digest_of_the_line() {
        let line = northward(-72.0, 46.0, 500.0);
        let reference = route_ref(&line, false);
        assert_eq!(reference.len(), 19);
        assert!(reference.starts_with("rt_"));
        assert!(reference[3..]
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        assert_eq!(route_ref(&line, false), reference);
        // Below the quantization step: the same line read back through JSON.
        let jittered: Vec<(f64, f64)> = line.iter().map(|&(lat, lon)| (lat + 1e-9, lon)).collect();
        assert_eq!(route_ref(&jittered, false), reference);
        assert_ne!(route_ref(&line, true), reference);
        let mut reversed = line.clone();
        reversed.reverse();
        assert_ne!(route_ref(&reversed, false), reference);
        let moved: Vec<(f64, f64)> = line.iter().map(|&(lat, lon)| (lat + 2e-6, lon)).collect();
        assert_ne!(route_ref(&moved, false), reference);
    }

    /// The digest is over the documented byte layout, so any other component
    /// can recompute a reference from the same line.
    #[test]
    fn a_route_ref_hashes_quantized_little_endian_points_and_the_flag() {
        let line = [(46.123_456_7, -72.5), (46.2, -72.4)];
        let mut bytes = Vec::new();
        for (lat, lon) in [(46_123_457_i64, -72_500_000_i64), (46_200_000, -72_400_000)] {
            bytes.extend(lat.to_le_bytes());
            bytes.extend(lon.to_le_bytes());
        }
        bytes.push(1);
        let digest = Sha256::digest(&bytes);
        assert_eq!(
            route_ref(&line, true),
            format!("rt_{}", hex::encode(&digest[..8]))
        );
    }
}
