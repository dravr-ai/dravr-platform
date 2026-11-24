// ABOUTME: Clusters an athlete's privacy-trimmed activity starts into places with ids that survive re-clustering
// ABOUTME: DBSCAN over trimmed starts, named anchors, stable ids; rides from a never-served home placed by their area

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Places
//!
//! Where an athlete trains is a handful of spots: a trailhead, a club's
//! meeting point, a forest they drive to. [`cluster`] finds them in the
//! trimmed first points of their tracks, pure and without I/O; the caller
//! reads the starts and writes the places.
//!
//! **Privacy.** A centroid is built only from [`StartKind::Trimmed`] points,
//! the first point of a track after the privacy trim. A raw recorded start
//! ([`StartKind::RawAssignOnly`]) may join a place whose radius it falls
//! inside, but never creates one or moves its centroid. And the densest
//! cluster of trimmed track endpoints is taken to be home, unless the
//! athlete said where home is ([`HomeDeclaration`]): no start within the
//! exclusion joins or shapes a place, and the home is returned nowhere.
//!
//! **Rides from home** (carnet#1723). A ride that starts inside the
//! exclusion is placed by where it went, never by where it left from. Its
//! *area point* is the mean of its trimmed track's points farther than the
//! exclusion's radius plus the trim radius from the exclusion's centre (700 m
//! for the inferred home, 900 m for a declared one). Area points cluster on
//! their own at `AREA_REACH_METERS`, one ride already making an area, so the
//! athlete can name it; an athlete-named place takes every area point within
//! its radius or that reach. No area point, and no centroid of a group
//! holding one, lies within that distance of the centre: an area point that
//! would (a ride circling the door) is never made, and a group whose
//! centroid would gives its area points back to the exclusion. A ride that
//! never leaves that distance, a walk around the block, stays excluded.
//! An area is a place like any other, flagged [`ClusteredPlace::area`]; the
//! home itself is still returned nowhere.
//!
//! **Home.** The trim leaves a track's first and last points on a ring
//! about [`DEFAULT_PRIVACY_RADIUS_METERS`] around the spot it left from, so
//! a loop that leaves one way and comes back another has its two trimmed
//! ends up to a ring's diameter apart. The home candidate is therefore
//! found by clustering endpoints at `ONE_SPOT_REACH_METERS`, a diameter plus
//! a polyline sample's slack, not at `eps`: every loop from one door counts
//! toward one cluster whichever way it left and came back, so a door ridden
//! in loops is not split per direction and outranked by a trailhead ridden
//! out and back. Three rules keep an athlete's name above the guess:
//! - a start inside an athlete-named place's radius is never excluded, so
//!   naming a spot always brings its starts back;
//! - an endpoint cluster whose centroid lies within `ONE_SPOT_REACH_METERS`
//!   of an athlete-named place is that place's rides, both ends of them,
//!   and is never the home candidate;
//! - of the clusters left, the one with the most endpoints is home.
//!
//! The reach is also the rule's limit: a separate spot whose endpoints sit
//! within it of the door's joins the door's cluster, and a door within it
//! of a named place is not taken for home, so its starts may form a place
//! whose centroid is stored and never served.
//!
//! **Declared home** (carnet#1593). The athlete's word replaces the guess:
//! - `Inferred` is the guess above, unchanged;
//! - `At` excludes every start within [`PlaceParams::home_exclusion_m`]
//!   plus the trim radius of the declared seed. The seed is itself a
//!   trimmed start or a mean of them, so it may sit a trim ring and a whole
//!   end's slack off the door, and another start from that door as far on
//!   the far side: the extra trim radius covers that pair;
//! - `Elsewhere` excludes nothing: the athlete does not train from home.
//!
//! Under `At` home wins near the seed: a start inside an athlete-named place
//! is still excluded when that place's centroid lies within
//! `HOME_WINS_METERS` of the seed, so naming a home start never brings the
//! door's starts back. The place keeps its name and survives empty. A named
//! place farther out keeps its starts, as every named place does inside the
//! inferred exclusion.
//!
//! **Stable ids.** A place keeps its id across re-clustering: each previous
//! place passes it to at most one new cluster, the nearest within `eps`. A
//! place the athlete named is an anchor: every trimmed start inside its
//! radius joins it before clustering runs, even a single visit, and it
//! survives with no member at all rather than lose the athlete's name.

use std::collections::HashMap;

use pierre_core::models::{CachedRowKey, HomeDeclaration};
use uuid::Uuid;

use crate::route_privacy::DEFAULT_PRIVACY_RADIUS_METERS;
use crate::routes::haversine_meters_between;

mod home;

/// The version of the clustering rule. A change to a parameter or the rule
/// bumps it, and every athlete's places are recomputed.
pub const PLACES_VERSION: u32 = 2;

/// The smallest radius a place is given, in metres: the clustering distance,
/// so a tight cluster still catches a start a few hundred metres off.
const MIN_RADIUS_METERS: f64 = 300.0;

/// The share of members a place's radius reaches.
const RADIUS_PERCENTILE: f64 = 0.9;

/// How far apart, in metres, two trimmed endpoints of rides from one spot
/// can lie.
///
/// The trim ring's diameter, plus the spacing a polyline's first sample past
/// the trim may add on each side. Home's endpoints are clustered at it, and
/// a ride whose own two trimmed ends lie within it started and ended at one
/// spot: a loop.
pub const ONE_SPOT_REACH_METERS: f64 =
    2.0f64.mul_add(DEFAULT_PRIVACY_RADIUS_METERS, POLYLINE_SLACK_METERS);

/// How far past the trim radius, in metres, the first kept point of a
/// sparsely sampled polyline may sit, summed over a track's two ends.
const POLYLINE_SLACK_METERS: f64 = 100.0;

/// How far apart, in metres, the area points of rides from home into one
/// area can lie and still be one place.
///
/// Measured on one athlete's trimmed tracks (carnet#1723): rides from his
/// door into one trail network have area points within 380 m of each other
/// for one network and chained within about 600 m for the other, and the two
/// networks lie at least 1.27 km apart. 600 m keeps each network one place
/// and the two apart. An athlete-named area takes every area point within
/// its radius or this reach, whichever is wider, so a name given after one
/// ride holds the next ride there.
const AREA_REACH_METERS: f64 = 600.0;

/// Metres per degree of latitude on the crate's sphere, to bound neighbour
/// searches by latitude before measuring.
const METERS_PER_DEGREE: f64 = 111_194.926_644_558_7;

/// Which kind of start a point is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartKind {
    /// The first point of a privacy-trimmed track: may form and shape a place.
    Trimmed,
    /// A raw recorded start of a row with no track: may only join a place
    /// whose radius it falls inside.
    RawAssignOnly,
}

/// One activity's start, keyed by its cached row.
#[derive(Debug, Clone, PartialEq)]
pub struct StartPoint {
    /// The row the start belongs to.
    pub key: CachedRowKey,
    /// Latitude in degrees.
    pub lat: f64,
    /// Longitude in degrees.
    pub lon: f64,
    /// Whether it may shape a place.
    pub kind: StartKind,
    /// The row's privacy-trimmed track, downsampled by the caller; empty when
    /// it has none. Read only for a start the home exclusion removes, to find
    /// the area the ride went to.
    pub track: Vec<(f64, f64)>,
}

/// A place from the previous clustering.
#[derive(Debug, Clone, PartialEq)]
pub struct PlaceAnchor {
    /// Its id, inherited by its heir.
    pub id: String,
    /// Centroid latitude in degrees.
    pub lat: f64,
    /// Centroid longitude in degrees.
    pub lon: f64,
    /// Its radius in metres: an athlete-named place takes every trimmed start
    /// inside it.
    pub radius_m: f64,
    /// Whether the athlete named it.
    pub athlete_named: bool,
    /// Members it had.
    pub activity_count: u32,
}

/// The clustering's parameters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlaceParams {
    /// DBSCAN neighbourhood, in metres.
    pub eps_m: f64,
    /// Starts within `eps_m` a core point needs, itself included.
    pub min_pts: usize,
    /// Distance from the home candidate inside which no start joins a place,
    /// unless it lies inside an athlete-named place's radius.
    pub home_exclusion_m: f64,
}

impl Default for PlaceParams {
    fn default() -> Self {
        Self {
            eps_m: 300.0,
            min_pts: 3,
            home_exclusion_m: 500.0,
        }
    }
}

/// A place this clustering produced.
#[derive(Debug, Clone, PartialEq)]
pub struct ClusteredPlace {
    /// Inherited from a previous place, or fresh.
    pub id: String,
    /// Centroid latitude: the mean of its trimmed members.
    pub lat: f64,
    /// Centroid longitude.
    pub lon: f64,
    /// The 90th-percentile member distance from the centroid, at least 300 m.
    pub radius_m: f64,
    /// Every row placed here, sorted. Empty for an athlete-named place that
    /// kept its name with no start left.
    pub members: Vec<CachedRowKey>,
    /// Whether rides from home are among its members, placed by the area
    /// their tracks went to rather than by their start.
    pub area: bool,
}

/// The places of one athlete, and the starts the home exclusion removed.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Clustering {
    /// Every place, ordered by its first member then its id.
    pub places: Vec<ClusteredPlace>,
    /// Rows whose start lies within the home exclusion and whose track
    /// reached no area: placed nowhere.
    pub excluded: Vec<CachedRowKey>,
}

/// A group of trimmed starts on its way to becoming a place.
struct Group {
    id: Option<String>,
    members: Vec<usize>,
    lat: f64,
    lon: f64,
}

/// Cluster an athlete's starts into places.
///
/// `points` are the starts, one per workout (the caller collapses copies).
/// `endpoints` are the trimmed first and last points of every track, from
/// which the home candidate is found. `previous` is the last clustering's
/// places. `home` is what the athlete said of home, which replaces the
/// candidate unless it is `Inferred`. The result is deterministic in
/// membership and inherited ids whatever order `points` and `endpoints`
/// arrive in; a place with no previous heir gets a fresh random id.
#[must_use]
pub fn cluster(
    points: &[StartPoint],
    endpoints: &[(f64, f64)],
    previous: &[PlaceAnchor],
    home: HomeDeclaration,
    params: &PlaceParams,
) -> Clustering {
    let mut points: Vec<&StartPoint> = points.iter().collect();
    points.sort_by(|a, b| a.key.cmp(&b.key));
    let named: Vec<&PlaceAnchor> = previous
        .iter()
        .filter(|place| place.athlete_named)
        .collect();
    let home = home::exclusion(home, endpoints, &named, params);

    // The home exclusion first: nothing near the door joins anything by its
    // start. A ride from home that went somewhere is placed by its area
    // point instead; a start the athlete placed by naming a spot it spares,
    // and that lies nearer that spot than the door, is never the door's.
    let mut excluded: Vec<&StartPoint> = Vec::new();
    let mut kept: Vec<&StartPoint> = Vec::with_capacity(points.len());
    let mut areas: Vec<StartPoint> = Vec::new();
    for point in points {
        let Some(home) = home.as_ref() else {
            kept.push(point);
            continue;
        };
        if !home.holds(point) {
            kept.push(point);
        } else if let Some(area) = home.area_point(point) {
            areas.push(area);
        } else {
            excluded.push(point);
        }
    }
    let first_area = kept.len();
    kept.extend(areas.iter());

    // Athlete-named places take every trimmed start inside their radius,
    // and every area point within their radius or an area's reach.
    let mut anchored: Vec<Vec<usize>> = vec![Vec::new(); named.len()];
    let mut free: Vec<usize> = Vec::new();
    let mut free_areas: Vec<usize> = Vec::new();
    let mut assign_only: Vec<usize> = Vec::new();
    for (index, point) in kept.iter().enumerate() {
        if point.kind == StartKind::RawAssignOnly {
            assign_only.push(index);
            continue;
        }
        let is_area = index >= first_area;
        let reach = |radius: f64| {
            if is_area {
                radius.max(AREA_REACH_METERS)
            } else {
                radius
            }
        };
        match nearest_within(
            point.lat,
            point.lon,
            named
                .iter()
                .map(|place| (place.lat, place.lon, reach(place.radius_m))),
        ) {
            Some(anchor) => anchored[anchor].push(index),
            None if is_area => free_areas.push(index),
            None => free.push(index),
        }
    }

    let mut groups: Vec<Group> = Vec::new();
    let mut taken: Vec<&str> = Vec::new();
    for (anchor, members) in named.iter().zip(anchored) {
        if members.is_empty() {
            continue;
        }
        let (lat, lon) = centroid(&kept, &members);
        taken.push(anchor.id.as_str());
        groups.push(Group {
            id: Some(anchor.id.clone()),
            members,
            lat,
            lon,
        });
    }

    let located: Vec<(f64, f64)> = free.iter().map(|&i| (kept[i].lat, kept[i].lon)).collect();
    let (clusters, noise) = dbscan(&located, params);
    let mut found: Vec<Group> = Vec::new();
    for cluster in clusters {
        let members: Vec<usize> = cluster.into_iter().map(|i| free[i]).collect();
        let (lat, lon) = centroid(&kept, &members);
        found.push(Group {
            id: None,
            members,
            lat,
            lon,
        });
    }
    assign_only.extend(noise.into_iter().map(|i| free[i]));

    // Areas cluster on their own, at an area's reach; one ride from home is
    // already an area, so the athlete can name it.
    let area_params = PlaceParams {
        eps_m: AREA_REACH_METERS,
        min_pts: 1,
        ..*params
    };
    let located: Vec<(f64, f64)> = free_areas
        .iter()
        .map(|&i| (kept[i].lat, kept[i].lon))
        .collect();
    let (area_clusters, _) = dbscan(&located, &area_params);
    for cluster in area_clusters {
        let members: Vec<usize> = cluster.into_iter().map(|i| free_areas[i]).collect();
        let (lat, lon) = centroid(&kept, &members);
        found.push(Group {
            id: None,
            members,
            lat,
            lon,
        });
    }

    // Nothing a ride from home places lies near the door: a group whose
    // centroid does gives its area points back to the exclusion.
    if let Some(home) = home.as_ref() {
        let every_group = groups.iter_mut().chain(found.iter_mut());
        for group in every_group {
            let returned = home.keep_area_away(group, &kept, first_area);
            excluded.extend(returned.into_iter().map(|i| kept[i]));
        }
        groups.retain(|group| !group.members.is_empty());
        found.retain(|group| !group.members.is_empty());
    }
    let taken: Vec<&str> = taken
        .into_iter()
        .filter(|id| groups.iter().any(|group| group.id.as_deref() == Some(*id)))
        .collect();

    inherit_ids(&mut found, previous, &taken, &kept, params);
    groups.append(&mut found);

    let mut places: Vec<ClusteredPlace> = groups
        .into_iter()
        .map(|group| ClusteredPlace {
            id: group.id.unwrap_or_else(|| Uuid::new_v4().to_string()),
            radius_m: radius(&kept, &group.members, group.lat, group.lon),
            lat: group.lat,
            lon: group.lon,
            members: group.members.iter().map(|&i| kept[i].key.clone()).collect(),
            area: group.members.iter().any(|&i| i >= first_area),
        })
        .collect();

    // A named place nothing inherited survives empty, keeping its name.
    for place in &named {
        let present = places.iter().any(|kept_place| kept_place.id == place.id);
        if !present {
            places.push(ClusteredPlace {
                id: place.id.clone(),
                lat: place.lat,
                lon: place.lon,
                radius_m: place.radius_m,
                members: Vec::new(),
                area: false,
            });
        }
    }

    // Starts that may only join: raw starts, and trimmed starts no cluster
    // took. They never move a centroid.
    for &index in &assign_only {
        let point = kept[index];
        let target = nearest_within(
            point.lat,
            point.lon,
            places
                .iter()
                .map(|place| (place.lat, place.lon, place.radius_m)),
        );
        if let Some(target) = target {
            places[target].members.push(point.key.clone());
        }
    }

    for place in &mut places {
        place.members.sort();
    }
    places.sort_by(|a, b| {
        a.members
            .first()
            .cmp(&b.members.first())
            .then(a.id.cmp(&b.id))
    });
    let mut excluded: Vec<CachedRowKey> = excluded.into_iter().map(|p| p.key.clone()).collect();
    excluded.sort();
    Clustering { places, excluded }
}

/// Give each new cluster the id of the previous place it inherits from.
///
/// Every previous place not already an anchor's claims the nearest cluster
/// within `eps` (ties: more members, then the lower first key). A cluster
/// claimed once takes the claimant's id. Claimed by several, it takes the
/// athlete-named one's, else the one with more activities; another named
/// claimant is kept as its own empty place, so a name is never lost, and an
/// unnamed one is merged away.
fn inherit_ids(
    clusters: &mut [Group],
    previous: &[PlaceAnchor],
    taken: &[&str],
    kept: &[&StartPoint],
    params: &PlaceParams,
) {
    let mut claimants: Vec<&PlaceAnchor> = previous
        .iter()
        .filter(|place| !taken.contains(&place.id.as_str()))
        .collect();
    claimants.sort_by(|a, b| a.id.cmp(&b.id));
    let mut claims: HashMap<usize, Vec<&PlaceAnchor>> = HashMap::new();
    for place in claimants {
        let best = clusters
            .iter()
            .enumerate()
            .map(|(index, cluster)| {
                let meters =
                    haversine_meters_between(place.lat, place.lon, cluster.lat, cluster.lon);
                (index, meters, cluster)
            })
            .filter(|(_, meters, _)| *meters <= params.eps_m)
            .min_by(|a, b| {
                a.1.total_cmp(&b.1)
                    .then_with(|| b.2.members.len().cmp(&a.2.members.len()))
                    .then_with(|| first_key(kept, a.2).cmp(&first_key(kept, b.2)))
            });
        if let Some((index, _, _)) = best {
            claims.entry(index).or_default().push(place);
        }
    }
    for (index, mut rivals) in claims {
        rivals.sort_by(|a, b| {
            b.athlete_named
                .cmp(&a.athlete_named)
                .then_with(|| b.activity_count.cmp(&a.activity_count))
                .then_with(|| a.id.cmp(&b.id))
        });
        clusters[index].id = Some(rivals[0].id.clone());
    }
}

fn first_key<'a>(kept: &[&'a StartPoint], group: &Group) -> Option<&'a CachedRowKey> {
    group.members.iter().map(|&i| &kept[i].key).min()
}

/// DBSCAN over `points` with haversine distance: the clusters, each a list
/// of indices in discovery order, and the indices left as noise. Points are
/// visited in the order given, so equal input gives equal output.
fn dbscan(points: &[(f64, f64)], params: &PlaceParams) -> (Vec<Vec<usize>>, Vec<usize>) {
    const UNVISITED: usize = usize::MAX;
    const NOISE: usize = usize::MAX - 1;
    let neighbours = Neighbours::new(points, params.eps_m);
    let mut label = vec![UNVISITED; points.len()];
    let mut clusters: Vec<Vec<usize>> = Vec::new();
    for start in 0..points.len() {
        if label[start] != UNVISITED {
            continue;
        }
        let around = neighbours.of(start);
        if around.len() < params.min_pts {
            label[start] = NOISE;
            continue;
        }
        let id = clusters.len();
        let mut members = vec![start];
        label[start] = id;
        let mut queue = around;
        let mut next = 0;
        while next < queue.len() {
            let point = queue[next];
            next += 1;
            if label[point] == NOISE {
                label[point] = id;
                members.push(point);
            }
            if label[point] != UNVISITED {
                continue;
            }
            label[point] = id;
            members.push(point);
            let reach = neighbours.of(point);
            if reach.len() >= params.min_pts {
                queue.extend(reach);
            }
        }
        clusters.push(members);
    }
    let noise = (0..points.len()).filter(|&i| label[i] == NOISE).collect();
    (clusters, noise)
}

/// Neighbour search bounded by latitude: points sorted by latitude, so only
/// those within `eps` degrees of latitude are measured.
struct Neighbours<'points> {
    points: &'points [(f64, f64)],
    by_latitude: Vec<usize>,
    eps_m: f64,
}

impl<'points> Neighbours<'points> {
    fn new(points: &'points [(f64, f64)], eps_m: f64) -> Self {
        let mut by_latitude: Vec<usize> = (0..points.len()).collect();
        by_latitude.sort_by(|&a, &b| points[a].0.total_cmp(&points[b].0).then(a.cmp(&b)));
        Self {
            points,
            by_latitude,
            eps_m,
        }
    }

    /// Every point within `eps` of point `index`, itself included, ascending.
    fn of(&self, index: usize) -> Vec<usize> {
        let (lat, lon) = self.points[index];
        let band = self.eps_m / METERS_PER_DEGREE;
        let from = self
            .by_latitude
            .partition_point(|&i| self.points[i].0 < lat - band);
        let mut found: Vec<usize> = self.by_latitude[from..]
            .iter()
            .take_while(|&&i| self.points[i].0 <= lat + band)
            .copied()
            .filter(|&i| {
                let (other_lat, other_lon) = self.points[i];
                haversine_meters_between(lat, lon, other_lat, other_lon) <= self.eps_m
            })
            .collect();
        found.sort_unstable();
        found
    }
}

/// The index of the nearest `(lat, lon, radius)` whose radius holds the
/// point; of equally near ones, the first.
fn nearest_within(
    lat: f64,
    lon: f64,
    areas: impl Iterator<Item = (f64, f64, f64)>,
) -> Option<usize> {
    areas
        .enumerate()
        .map(|(index, (area_lat, area_lon, radius))| {
            (
                index,
                haversine_meters_between(lat, lon, area_lat, area_lon),
                radius,
            )
        })
        .filter(|&(_, meters, radius)| meters <= radius)
        .min_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)))
        .map(|(index, _, _)| index)
}

/// The mean of the members' points.
fn centroid(kept: &[&StartPoint], members: &[usize]) -> (f64, f64) {
    let count = members.len().max(1) as f64;
    let lat = members.iter().map(|&i| kept[i].lat).sum::<f64>() / count;
    let lon = members.iter().map(|&i| kept[i].lon).sum::<f64>() / count;
    (lat, lon)
}

/// The 90th-percentile member distance from the centroid, at least
/// [`MIN_RADIUS_METERS`].
fn radius(kept: &[&StartPoint], members: &[usize], lat: f64, lon: f64) -> f64 {
    let mut distances: Vec<f64> = members
        .iter()
        .map(|&i| haversine_meters_between(lat, lon, kept[i].lat, kept[i].lon))
        .collect();
    distances.sort_by(f64::total_cmp);
    let rank = (RADIUS_PERCENTILE * distances.len() as f64).ceil() as usize;
    distances
        .get(rank.saturating_sub(1))
        .copied()
        .unwrap_or(0.0)
        .max(MIN_RADIUS_METERS)
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) const ORIGIN: (f64, f64) = (46.0, -72.0);

    /// The point `east` and `north` metres from the origin.
    pub(super) fn at(east: f64, north: f64) -> (f64, f64) {
        (
            ORIGIN.0 + north / METERS_PER_DEGREE,
            ORIGIN.1 + east / (METERS_PER_DEGREE * ORIGIN.0.to_radians().cos()),
        )
    }

    pub(super) fn key(id: &str) -> CachedRowKey {
        CachedRowKey {
            provider: "strava".to_owned(),
            activity_id: id.to_owned(),
        }
    }

    fn start(id: &str, east: f64, north: f64, kind: StartKind) -> StartPoint {
        let (lat, lon) = at(east, north);
        StartPoint {
            key: key(id),
            lat,
            lon,
            kind,
            track: Vec::new(),
        }
    }

    pub(super) fn trimmed(id: &str, east: f64, north: f64) -> StartPoint {
        start(id, east, north, StartKind::Trimmed)
    }

    pub(super) fn anchor(id: &str, east: f64, north: f64, named: bool, count: u32) -> PlaceAnchor {
        let (lat, lon) = at(east, north);
        PlaceAnchor {
            id: id.to_owned(),
            lat,
            lon,
            radius_m: 300.0,
            athlete_named: named,
            activity_count: count,
        }
    }

    /// `count` trimmed starts within 60 m of `(east, north)`, ids `prefix0..`.
    pub(super) fn visits(prefix: &str, count: usize, east: f64, north: f64) -> Vec<StartPoint> {
        (0..count)
            .map(|i| {
                let angle = i as f64;
                trimmed(
                    &format!("{prefix}{i}"),
                    angle.cos().mul_add(60.0, east),
                    angle.sin().mul_add(60.0, north),
                )
            })
            .collect()
    }

    fn run(points: &[StartPoint], previous: &[PlaceAnchor]) -> Clustering {
        run_with(points, &[], previous, HomeDeclaration::Inferred)
    }

    pub(super) fn run_with(
        points: &[StartPoint],
        endpoints: &[(f64, f64)],
        previous: &[PlaceAnchor],
        home: HomeDeclaration,
    ) -> Clustering {
        cluster(points, endpoints, previous, home, &PlaceParams::default())
    }

    pub(super) fn memberships(clustering: &Clustering) -> Vec<Vec<CachedRowKey>> {
        let mut sets: Vec<Vec<CachedRowKey>> = clustering
            .places
            .iter()
            .map(|place| place.members.clone())
            .collect();
        sets.sort();
        sets
    }

    #[test]
    fn three_starts_close_together_make_a_place_and_two_do_not() {
        let three = run(&visits("a", 3, 0.0, 0.0), &[]);
        assert_eq!(
            three.places.len(),
            1,
            "bug: three close starts made no place"
        );
        assert_eq!(three.places[0].members.len(), 3, "bug: a member is missing");
        assert!(
            three.places[0].radius_m >= MIN_RADIUS_METERS,
            "bug: the radius is below its floor"
        );
        let two = run(&visits("a", 2, 0.0, 0.0), &[]);
        assert!(two.places.is_empty(), "bug: two starts made a place");
    }

    #[test]
    fn clustering_is_deterministic_under_permutation() {
        let mut points = visits("a", 4, 0.0, 0.0);
        points.extend(visits("b", 5, 3_000.0, 0.0));
        points.push(trimmed("lone", 9_000.0, 9_000.0));
        let forward = run(&points, &[]);
        points.reverse();
        let backward = run(&points, &[]);
        assert_eq!(
            memberships(&forward),
            memberships(&backward),
            "bug: input order changed the places"
        );
        assert_eq!(forward.places.len(), 2, "bug: wrong place count");
    }

    #[test]
    fn a_place_keeps_its_id_when_new_activities_arrive() {
        let first = run(&visits("a", 3, 0.0, 0.0), &[]);
        let id = first.places[0].id.clone();
        let previous = [PlaceAnchor {
            id: id.clone(),
            lat: first.places[0].lat,
            lon: first.places[0].lon,
            radius_m: first.places[0].radius_m,
            athlete_named: false,
            activity_count: 3,
        }];
        let mut grown = visits("a", 3, 0.0, 0.0);
        grown.extend(visits("b", 50, 40.0, 10.0));
        let second = run(&grown, &previous);
        assert_eq!(second.places.len(), 1, "bug: the place split");
        assert_eq!(second.places[0].id, id, "bug: the id was not inherited");
    }

    #[test]
    fn a_named_place_keeps_its_name_after_fifty_more_activities() {
        let previous = [anchor("gai-luron", 0.0, 0.0, true, 3)];
        let mut points = visits("a", 3, 0.0, 0.0);
        points.extend(visits("b", 50, 100.0, 50.0));
        let after = run(&points, &previous);
        let place = after
            .places
            .iter()
            .find(|place| place.id == "gai-luron")
            .expect("bug: the named place lost its id");
        assert_eq!(place.members.len(), 53, "bug: the named place lost members");
    }

    /// A split: the previous place sits between two new clusters, both
    /// within reach. Only one inherits its id.
    #[test]
    fn a_split_gives_one_heir_and_one_fresh_id() {
        let previous = [anchor("old", 0.0, 0.0, false, 6)];
        let mut points = visits("a", 3, -250.0, 0.0);
        points.extend(visits("b", 4, 280.0, 0.0));
        let after = run(&points, &previous);
        assert_eq!(
            after.places.len(),
            2,
            "bug: the split did not give two places"
        );
        let heirs = after
            .places
            .iter()
            .filter(|place| place.id == "old")
            .count();
        assert_eq!(heirs, 1, "bug: {heirs} clusters inherited one id");
        let heir = after.places.iter().find(|place| place.id == "old").unwrap();
        assert_eq!(
            heir.members.len(),
            3,
            "bug: the farther cluster inherited the id"
        );
    }

    /// Two named places whose starts now form one cluster keep both names.
    #[test]
    fn two_named_places_merging_keep_both_names() {
        let previous = [
            PlaceAnchor {
                radius_m: 50.0,
                ..anchor("north", 0.0, 1_000.0, true, 5)
            },
            PlaceAnchor {
                radius_m: 50.0,
                ..anchor("south", 0.0, 1_200.0, true, 3)
            },
        ];
        // The starts sit between the two, outside both radii, so DBSCAN
        // clusters them and both names claim the one cluster.
        let after = run(&visits("a", 4, 0.0, 1_100.0), &previous);
        let ids: Vec<&str> = after.places.iter().map(|place| place.id.as_str()).collect();
        assert!(
            ids.contains(&"north") && ids.contains(&"south"),
            "bug: a name was lost in a merge: {ids:?}"
        );
        let north = after
            .places
            .iter()
            .find(|place| place.id == "north")
            .unwrap();
        assert_eq!(
            north.members.len(),
            4,
            "bug: the busier name did not survive"
        );
        let south = after
            .places
            .iter()
            .find(|place| place.id == "south")
            .unwrap();
        assert!(south.members.is_empty(), "bug: the loser kept members");
    }

    #[test]
    fn an_unnamed_place_merged_away_disappears() {
        let previous = [
            anchor("big", 0.0, 0.0, false, 9),
            anchor("small", 150.0, 0.0, false, 2),
        ];
        let after = run(&visits("a", 4, 80.0, 0.0), &previous);
        let ids: Vec<&str> = after.places.iter().map(|place| place.id.as_str()).collect();
        assert_eq!(ids, vec!["big"], "bug: the merge kept {ids:?}");
    }

    /// The athlete named a spot on one visit: it stays a place with that
    /// visit as its member, below `min_pts`.
    #[test]
    fn a_single_visit_athlete_place_keeps_its_row() {
        let previous = [anchor("trailhead", 0.0, 0.0, true, 1)];
        let after = run(&[trimmed("only", 20.0, 0.0)], &previous);
        assert_eq!(
            after.places.len(),
            1,
            "bug: the named single visit vanished"
        );
        assert_eq!(after.places[0].id, "trailhead", "bug: wrong id");
        assert_eq!(
            after.places[0].members,
            vec![key("only")],
            "bug: the visit lost its place"
        );
    }

    #[test]
    fn erasure_removes_an_unnamed_place_and_keeps_a_named_one_empty() {
        let previous = [
            anchor("unnamed", 0.0, 0.0, false, 4),
            anchor("named", 5_000.0, 0.0, true, 4),
        ];
        let after = run(&[], &previous);
        assert_eq!(after.places.len(), 1, "bug: wrong survivors: {after:?}");
        assert_eq!(
            after.places[0].id, "named",
            "bug: the named place was deleted"
        );
        assert!(after.places[0].members.is_empty(), "bug: phantom members");
    }

    #[test]
    fn raw_starts_never_create_or_move_a_place() {
        let raw = |id: &str, east: f64| start(id, east, 0.0, StartKind::RawAssignOnly);
        let alone = run(&[raw("r1", 0.0), raw("r2", 10.0), raw("r3", 20.0)], &[]);
        assert!(alone.places.is_empty(), "bug: raw starts created a place");

        let mut points = visits("a", 3, 0.0, 0.0);
        let without = run(&points, &[]);
        points.push(raw("r1", 250.0));
        let with = run(&points, &[]);
        assert_eq!(with.places.len(), 1, "bug: wrong place count");
        assert!(
            with.places[0].members.contains(&key("r1")),
            "bug: a raw start inside the radius was not assigned"
        );
        assert!(
            (with.places[0].lat - without.places[0].lat).abs() < 1e-12
                && (with.places[0].lon - without.places[0].lon).abs() < 1e-12,
            "bug: a raw start moved the centroid"
        );
    }
}
