// ABOUTME: Clusters an athlete's privacy-trimmed activity starts into places with ids that survive re-clustering
// ABOUTME: DBSCAN over trimmed starts, athlete-named anchors, one heir per previous place, and a never-stored home exclusion

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
//! cluster of trimmed track endpoints is taken to be home: every start
//! within [`PlaceParams::home_exclusion_m`] of it is excluded from every
//! place, and the home itself is computed afresh on each call and returned
//! nowhere.
//!
//! **Home.** The trim leaves a track's first and last points on a ring
//! about [`DEFAULT_PRIVACY_RADIUS_METERS`] around the spot it left from, so
//! a loop that leaves one way and comes back another has its two trimmed
//! ends up to a ring's diameter apart. The home candidate is therefore
//! found by clustering endpoints at `HOME_REACH_METERS`, a diameter plus
//! a polyline sample's slack, not at `eps`: every loop from one door counts
//! toward one cluster whichever way it left and came back, so a door ridden
//! in loops is not split per direction and outranked by a trailhead ridden
//! out and back. Three rules keep an athlete's name above the guess:
//! - a start inside an athlete-named place's radius is never excluded, so
//!   naming a spot always brings its starts back;
//! - an endpoint cluster whose centroid lies within `HOME_REACH_METERS`
//!   of an athlete-named place is that place's rides, both ends of them,
//!   and is never the home candidate;
//! - of the clusters left, the one with the most endpoints is home.
//!
//! The reach is also the rule's limit: a separate spot whose endpoints sit
//! within it of the door's joins the door's cluster, and a door within it
//! of a named place is not taken for home, so its starts may form a place
//! whose centroid is stored and never served.
//!
//! **Stable ids.** A place keeps its id across re-clustering: each previous
//! place passes it to at most one new cluster, the nearest within `eps`. A
//! place the athlete named is an anchor: every trimmed start inside its
//! radius joins it before clustering runs, even a single visit, and it
//! survives with no member at all rather than lose the athlete's name.

use std::collections::HashMap;

use pierre_core::models::CachedRowKey;
use uuid::Uuid;

use crate::route_privacy::DEFAULT_PRIVACY_RADIUS_METERS;
use crate::routes::haversine_meters_between;

/// The version of the clustering rule. A change to a parameter or the rule
/// bumps it, and every athlete's places are recomputed.
pub const PLACES_VERSION: u32 = 1;

/// The smallest radius a place is given, in metres: the clustering distance,
/// so a tight cluster still catches a start a few hundred metres off.
const MIN_RADIUS_METERS: f64 = 300.0;

/// The share of members a place's radius reaches.
const RADIUS_PERCENTILE: f64 = 0.9;

/// How far apart, in metres, two trimmed endpoints of rides from one spot
/// can lie: the trim ring's diameter, plus the spacing a polyline's first
/// sample past the trim may add on each side.
const HOME_REACH_METERS: f64 = 2.0 * DEFAULT_PRIVACY_RADIUS_METERS + POLYLINE_SLACK_METERS;

/// How far past the trim radius, in metres, the first kept point of a
/// sparsely sampled polyline may sit, summed over a track's two ends.
const POLYLINE_SLACK_METERS: f64 = 100.0;

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
}

/// The places of one athlete, and the starts the home exclusion removed.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Clustering {
    /// Every place, ordered by its first member then its id.
    pub places: Vec<ClusteredPlace>,
    /// Rows whose start lies within the home exclusion: placed nowhere.
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
/// places. The result is deterministic in membership and inherited ids
/// whatever order `points` and `endpoints` arrive in; a place with no
/// previous heir gets a fresh random id.
#[must_use]
pub fn cluster(
    points: &[StartPoint],
    endpoints: &[(f64, f64)],
    previous: &[PlaceAnchor],
    params: &PlaceParams,
) -> Clustering {
    let mut points: Vec<&StartPoint> = points.iter().collect();
    points.sort_by(|a, b| a.key.cmp(&b.key));
    let named: Vec<&PlaceAnchor> = previous
        .iter()
        .filter(|place| place.athlete_named)
        .collect();
    let home = home_candidate(endpoints, &named, params);

    // The home exclusion first: nothing near the door joins anything, but a
    // start the athlete placed by naming the spot is never the door's.
    let mut excluded = Vec::new();
    let mut kept: Vec<&StartPoint> = Vec::with_capacity(points.len());
    for point in points {
        let near_home = home.is_some_and(|(lat, lon)| {
            haversine_meters_between(point.lat, point.lon, lat, lon) <= params.home_exclusion_m
        });
        let named_spot = nearest_within(
            point.lat,
            point.lon,
            named
                .iter()
                .map(|place| (place.lat, place.lon, place.radius_m)),
        )
        .is_some();
        if near_home && !named_spot {
            excluded.push(point.key.clone());
        } else {
            kept.push(point);
        }
    }

    // Athlete-named places take every trimmed start inside their radius.
    let mut anchored: Vec<Vec<usize>> = vec![Vec::new(); named.len()];
    let mut free: Vec<usize> = Vec::new();
    let mut assign_only: Vec<usize> = Vec::new();
    for (index, point) in kept.iter().enumerate() {
        if point.kind == StartKind::RawAssignOnly {
            assign_only.push(index);
            continue;
        }
        match nearest_within(
            point.lat,
            point.lon,
            named
                .iter()
                .map(|place| (place.lat, place.lon, place.radius_m)),
        ) {
            Some(anchor) => anchored[anchor].push(index),
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
    let first_cluster = groups.len();
    for cluster in clusters {
        let members: Vec<usize> = cluster.into_iter().map(|i| free[i]).collect();
        let (lat, lon) = centroid(&kept, &members);
        groups.push(Group {
            id: None,
            members,
            lat,
            lon,
        });
    }
    assign_only.extend(noise.into_iter().map(|i| free[i]));

    inherit_ids(
        &mut groups[first_cluster..],
        previous,
        &taken,
        &kept,
        params,
    );

    let mut places: Vec<ClusteredPlace> = groups
        .into_iter()
        .map(|group| ClusteredPlace {
            id: group.id.unwrap_or_else(|| Uuid::new_v4().to_string()),
            radius_m: radius(&kept, &group.members, group.lat, group.lon),
            lat: group.lat,
            lon: group.lon,
            members: group.members.iter().map(|&i| kept[i].key.clone()).collect(),
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

/// The home candidate: of the clusters of trimmed endpoints at
/// [`HOME_REACH_METERS`], the one with the most endpoints whose centroid is
/// not within that reach of an athlete-named place.
fn home_candidate(
    endpoints: &[(f64, f64)],
    named: &[&PlaceAnchor],
    params: &PlaceParams,
) -> Option<(f64, f64)> {
    let mut sorted: Vec<(f64, f64)> = endpoints.to_vec();
    sorted.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    let reach = PlaceParams {
        eps_m: params.eps_m.max(HOME_REACH_METERS),
        ..*params
    };
    let (clusters, _) = dbscan(&sorted, &reach);
    clusters
        .iter()
        .map(|members| {
            let count = members.len().max(1) as f64;
            let lat = members.iter().map(|&i| sorted[i].0).sum::<f64>() / count;
            let lon = members.iter().map(|&i| sorted[i].1).sum::<f64>() / count;
            (members.len(), lat, lon)
        })
        .filter(|&(_, lat, lon)| {
            !named.iter().any(|place| {
                haversine_meters_between(lat, lon, place.lat, place.lon) <= reach.eps_m
            })
        })
        // Densest first; of equals, the one found first in sorted order.
        .fold(
            None,
            |best: Option<(usize, f64, f64)>, candidate| match best {
                Some(found) if found.0 >= candidate.0 => Some(found),
                _ => Some(candidate),
            },
        )
        .map(|(_, lat, lon)| (lat, lon))
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

    const ORIGIN: (f64, f64) = (46.0, -72.0);

    /// The point `east` and `north` metres from the origin.
    fn at(east: f64, north: f64) -> (f64, f64) {
        (
            ORIGIN.0 + north / METERS_PER_DEGREE,
            ORIGIN.1 + east / (METERS_PER_DEGREE * ORIGIN.0.to_radians().cos()),
        )
    }

    fn key(id: &str) -> CachedRowKey {
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
        }
    }

    fn trimmed(id: &str, east: f64, north: f64) -> StartPoint {
        start(id, east, north, StartKind::Trimmed)
    }

    fn anchor(id: &str, east: f64, north: f64, named: bool, count: u32) -> PlaceAnchor {
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
    fn visits(prefix: &str, count: usize, east: f64, north: f64) -> Vec<StartPoint> {
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
        cluster(points, &[], previous, &PlaceParams::default())
    }

    fn memberships(clustering: &Clustering) -> Vec<Vec<CachedRowKey>> {
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

    /// Trimmed starts ring the door at about 200 m; the track endpoints are
    /// thickest there. None of it becomes a place, and no centroid lands
    /// near the door.
    #[test]
    fn starts_around_home_form_no_place() {
        let ring: Vec<StartPoint> = (0..12)
            .map(|i| {
                let angle = f64::from(i) * 0.5;
                trimmed(&format!("h{i}"), angle.cos() * 200.0, angle.sin() * 200.0)
            })
            .collect();
        let endpoints: Vec<(f64, f64)> = (0..24)
            .map(|i| {
                let angle = f64::from(i) * 0.26;
                at(angle.cos() * 200.0, angle.sin() * 200.0)
            })
            .collect();
        let mut points = ring;
        points.extend(visits("park", 4, 4_000.0, 0.0));
        let after = cluster(&points, &endpoints, &[], &PlaceParams::default());
        assert_eq!(after.places.len(), 1, "bug: home formed a place: {after:?}");
        assert_eq!(
            after.excluded.len(),
            12,
            "bug: a home start was not excluded"
        );
        let door = at(0.0, 0.0);
        for place in &after.places {
            assert!(
                haversine_meters_between(place.lat, place.lon, door.0, door.1) > 1_000.0,
                "bug: a centroid sits near the door"
            );
        }
    }

    /// The athlete always drives to one trailhead, so it is the densest
    /// endpoint cluster and is excluded; once they name it, it is a place
    /// and the home candidate moves on.
    #[test]
    fn naming_an_excluded_trailhead_makes_it_a_place() {
        let starts = visits("t", 6, 0.0, 0.0);
        let endpoints: Vec<(f64, f64)> = starts.iter().flat_map(|s| [(s.lat, s.lon); 2]).collect();
        let before = cluster(&starts, &endpoints, &[], &PlaceParams::default());
        assert!(
            before.places.is_empty(),
            "bug: the trailhead was not excluded"
        );
        assert_eq!(before.excluded.len(), 6, "bug: wrong exclusion");

        let named = [anchor("gai-luron", 20.0, 0.0, true, 1)];
        let after = cluster(&starts, &endpoints, &named, &PlaceParams::default());
        assert!(
            after.excluded.is_empty(),
            "bug: the named trailhead is still home"
        );
        assert_eq!(after.places.len(), 1, "bug: wrong place count");
        assert_eq!(
            after.places[0].id, "gai-luron",
            "bug: the name was not kept"
        );
        assert_eq!(
            after.places[0].members.len(),
            6,
            "bug: the rides did not join"
        );
    }

    /// A loop leaves the trailhead north and comes back from the east: its
    /// trimmed start and end sit 358 m apart on the trim ring, farther than
    /// `eps`. The far ends are the named trailhead's own rides, never home,
    /// and the starts inside its radius stay its members.
    #[test]
    fn a_loops_far_end_never_takes_the_named_trailheads_starts() {
        let starts: Vec<StartPoint> = (0..5)
            .map(|i| trimmed(&format!("t{i}"), 0.0, 0.0))
            .collect();
        let mut endpoints = vec![at(0.0, 0.0); 5];
        endpoints.extend(vec![at(297.0, 200.0); 5]);
        let named = [anchor("gai-luron", 0.0, 0.0, true, 1)];
        let after = cluster(&starts, &endpoints, &named, &PlaceParams::default());
        assert!(
            after.excluded.is_empty(),
            "bug: the loops' far ends were taken for home and excluded the named starts: {after:?}"
        );
        let place = after
            .places
            .iter()
            .find(|place| place.id == "gai-luron")
            .expect("bug: the named place is gone");
        assert_eq!(
            place.members.len(),
            5,
            "bug: the named trailhead lost its starts"
        );
    }

    /// Endpoints 358 m from a named place, with starts there that lie
    /// outside its radius: the cluster is within reach of the name, so it is
    /// not home, and those starts are free to form a place.
    #[test]
    fn an_endpoint_cluster_within_reach_of_a_named_place_is_never_home() {
        let starts = visits("q", 4, 297.0, 200.0);
        let endpoints: Vec<(f64, f64)> = starts.iter().flat_map(|s| [(s.lat, s.lon); 2]).collect();
        let named = [anchor("gai-luron", 0.0, 0.0, true, 1)];
        let after = cluster(&starts, &endpoints, &named, &PlaceParams::default());
        assert!(
            after.excluded.is_empty(),
            "bug: a cluster within reach of a named place was taken for home: {after:?}"
        );
        assert!(
            after.places.iter().any(|place| place.members.len() == 4),
            "bug: the starts near the named place formed no place: {after:?}"
        );
    }

    /// Six loops leave the door north and come home from the south, so
    /// their trimmed ends sit 400 m apart; four out-and-back rides start at
    /// a trailhead. Per direction the door has six endpoints, the trailhead
    /// eight: counted as one spot the door has twelve, and is home.
    #[test]
    fn a_door_ridden_in_loops_outranks_a_trailhead_ridden_out_and_back() {
        let mut points: Vec<StartPoint> = (0..6)
            .map(|i| trimmed(&format!("h{i}"), 0.0, 200.0))
            .collect();
        let mut endpoints = vec![at(0.0, 200.0); 6];
        endpoints.extend(vec![at(0.0, -200.0); 6]);
        let trailhead = visits("t", 4, 4_000.0, 0.0);
        endpoints.extend(trailhead.iter().flat_map(|s| [(s.lat, s.lon); 2]));
        points.extend(trailhead);
        let after = cluster(&points, &endpoints, &[], &PlaceParams::default());
        let home: Vec<CachedRowKey> = (0..6).map(|i| key(&format!("h{i}"))).collect();
        assert_eq!(
            after.excluded, home,
            "bug: the door's loops were outranked by the trailhead: {after:?}"
        );
        assert_eq!(
            memberships(&after),
            vec![(0..4).map(|i| key(&format!("t{i}"))).collect::<Vec<_>>()],
            "bug: the trailhead is not the one place"
        );
    }
}
