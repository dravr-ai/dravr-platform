// ABOUTME: The home exclusion of the places clustering: the inferred or declared door no start is placed by
// ABOUTME: and the area points that place a ride from home by where its trimmed track went (carnet#1723)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The home exclusion [`super::cluster`] applies, and the area point a ride
//! from home is placed by. The module documentation of [`super`] states the
//! rules; this module holds their mechanics.

use pierre_core::models::HomeDeclaration;

use super::{
    centroid, dbscan, Group, PlaceAnchor, PlaceParams, StartKind, StartPoint,
    ONE_SPOT_REACH_METERS, POLYLINE_SLACK_METERS,
};
use crate::route_privacy::DEFAULT_PRIVACY_RADIUS_METERS;
use crate::routes::haversine_meters_between;

/// How near a declared seed, in metres, an athlete-named place's centroid
/// must lie for home to win over the name: two trimmed starts from one door,
/// each a trim ring and one end's whole slack out, on opposite sides.
const HOME_WINS_METERS: f64 = 2.0 * (DEFAULT_PRIVACY_RADIUS_METERS + POLYLINE_SLACK_METERS);

/// The home exclusion one clustering applies: its centre and radius, and the
/// athlete-named places whose starts it leaves alone.
pub(super) struct Exclusion<'anchors> {
    lat: f64,
    lon: f64,
    radius_m: f64,
    spared: Vec<&'anchors PlaceAnchor>,
}

impl Exclusion<'_> {
    /// Whether `point` lies inside the exclusion and no athlete-named place
    /// it spares claims it. A spared place claims a start inside its radius
    /// that lies nearer its centroid than the exclusion's centre, and only
    /// when its centroid lies within [`Self::area_from_m`] of the centre: a
    /// named spot by the door gets its own starts back, while a place out
    /// past it, an area above all, whose radius happens to reach the door,
    /// never takes a door start and never drifts toward home.
    pub(super) fn holds(&self, point: &StartPoint) -> bool {
        let from_centre = haversine_meters_between(point.lat, point.lon, self.lat, self.lon);
        let area_from = self.area_from_m();
        from_centre <= self.radius_m
            && !self.spared.iter().any(|place| {
                if haversine_meters_between(place.lat, place.lon, self.lat, self.lon) > area_from {
                    return false;
                }
                let from_place =
                    haversine_meters_between(point.lat, point.lon, place.lat, place.lon);
                from_place <= place.radius_m && from_place < from_centre
            })
    }

    /// How far from the centre, in metres, a track point counts as away from
    /// home: the exclusion plus the trim radius, so nothing within a trim
    /// ring of the excluded starts ever places a ride.
    fn area_from_m(&self) -> f64 {
        self.radius_m + DEFAULT_PRIVACY_RADIUS_METERS
    }

    /// The area point of an excluded start: the mean of its track's points
    /// farther than [`Self::area_from_m`] from the centre, as a trimmed
    /// start of the same row. `None` when the track never goes that far, or
    /// when the mean lands back within that distance, as it does for a ride
    /// that circles the door.
    pub(super) fn area_point(&self, point: &StartPoint) -> Option<StartPoint> {
        let from = self.area_from_m();
        let far: Vec<&(f64, f64)> = point
            .track
            .iter()
            .filter(|(lat, lon)| haversine_meters_between(*lat, *lon, self.lat, self.lon) > from)
            .collect();
        if far.is_empty() {
            return None;
        }
        let count = far.len() as f64;
        let lat = far.iter().map(|p| p.0).sum::<f64>() / count;
        let lon = far.iter().map(|p| p.1).sum::<f64>() / count;
        (haversine_meters_between(lat, lon, self.lat, self.lon) > from).then(|| StartPoint {
            key: point.key.clone(),
            lat,
            lon,
            kind: StartKind::Trimmed,
            track: Vec::new(),
        })
    }

    /// Keep `group` away from home: when its centroid lies within
    /// [`Self::area_from_m`] of the centre, its area points (indices from
    /// `first_area`) leave it and its centroid is taken again from what is
    /// left. Returns the area points that left.
    pub(super) fn keep_area_away(
        &self,
        group: &mut Group,
        kept: &[&StartPoint],
        first_area: usize,
    ) -> Vec<usize> {
        let near = haversine_meters_between(group.lat, group.lon, self.lat, self.lon)
            <= self.area_from_m();
        if !near || group.members.iter().all(|&i| i < first_area) {
            return Vec::new();
        }
        let (returned, left): (Vec<usize>, Vec<usize>) =
            group.members.iter().partition(|&&i| i >= first_area);
        group.members = left;
        if !group.members.is_empty() {
            (group.lat, group.lon) = centroid(kept, &group.members);
        }
        returned
    }
}

/// The exclusion `home` asks for. The inferred candidate spares every
/// athlete-named place; a declared seed spares only those farther than
/// [`HOME_WINS_METERS`] from it; `Elsewhere` excludes nothing.
pub(super) fn exclusion<'anchors>(
    home: HomeDeclaration,
    endpoints: &[(f64, f64)],
    named: &[&'anchors PlaceAnchor],
    params: &PlaceParams,
) -> Option<Exclusion<'anchors>> {
    match home {
        HomeDeclaration::Inferred => {
            home_candidate(endpoints, named, params).map(|(lat, lon)| Exclusion {
                lat,
                lon,
                radius_m: params.home_exclusion_m,
                spared: named.to_vec(),
            })
        }
        HomeDeclaration::At { lat, lon } => Some(Exclusion {
            lat,
            lon,
            radius_m: params.home_exclusion_m + DEFAULT_PRIVACY_RADIUS_METERS,
            spared: named
                .iter()
                .copied()
                .filter(|place| {
                    haversine_meters_between(lat, lon, place.lat, place.lon) > HOME_WINS_METERS
                })
                .collect(),
        }),
        HomeDeclaration::Elsewhere => None,
    }
}

/// The home candidate: of the clusters of trimmed endpoints at
/// [`ONE_SPOT_REACH_METERS`], the one with the most endpoints whose centroid is
/// not within that reach of an athlete-named place.
fn home_candidate(
    endpoints: &[(f64, f64)],
    named: &[&PlaceAnchor],
    params: &PlaceParams,
) -> Option<(f64, f64)> {
    let mut sorted: Vec<(f64, f64)> = endpoints.to_vec();
    sorted.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    let reach = PlaceParams {
        eps_m: params.eps_m.max(ONE_SPOT_REACH_METERS),
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

#[cfg(test)]
mod tests {
    use std::f64::consts::TAU;

    use pierre_core::models::CachedRowKey;

    use super::super::tests::{anchor, at, key, memberships, run_with, trimmed, visits, ORIGIN};
    use super::super::Clustering;
    use super::*;

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
        let after = run_with(&points, &endpoints, &[], HomeDeclaration::Inferred);
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
        let before = run_with(&starts, &endpoints, &[], HomeDeclaration::Inferred);
        assert!(
            before.places.is_empty(),
            "bug: the trailhead was not excluded"
        );
        assert_eq!(before.excluded.len(), 6, "bug: wrong exclusion");

        let named = [anchor("gai-luron", 20.0, 0.0, true, 1)];
        let after = run_with(&starts, &endpoints, &named, HomeDeclaration::Inferred);
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
        let after = run_with(&starts, &endpoints, &named, HomeDeclaration::Inferred);
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
        let after = run_with(&starts, &endpoints, &named, HomeDeclaration::Inferred);
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
        let after = run_with(&points, &endpoints, &[], HomeDeclaration::Inferred);
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

    /// The athlete declared home at `(east, north)` from the origin.
    fn declared(east: f64, north: f64) -> HomeDeclaration {
        let (lat, lon) = at(east, north);
        HomeDeclaration::At { lat, lon }
    }

    fn keys(prefix: &str, count: usize) -> Vec<CachedRowKey> {
        (0..count).map(|i| key(&format!("{prefix}{i}"))).collect()
    }

    /// The trailhead is the densest endpoint cluster; the door, 3 km north,
    /// has its starts on the trim ring and the seed is one of them.
    #[test]
    fn a_declared_home_replaces_the_inferred_trailhead() {
        let mut points = visits("t", 6, 0.0, 0.0);
        let endpoints: Vec<(f64, f64)> = points.iter().flat_map(|s| [(s.lat, s.lon); 2]).collect();
        points.extend(
            [(200.0, 0.0), (0.0, 200.0), (-200.0, 0.0), (0.0, -200.0)]
                .iter()
                .enumerate()
                .map(|(i, (east, north))| trimmed(&format!("h{i}"), *east, 3_000.0 + north)),
        );
        let after = run_with(&points, &endpoints, &[], declared(0.0, 3_200.0));
        assert_eq!(
            after.excluded,
            keys("h", 4),
            "bug: the door's starts were not excluded: {after:?}"
        );
        assert_eq!(
            memberships(&after),
            vec![keys("t", 6)],
            "bug: the trailhead is not the one place"
        );
    }

    /// The seed 245 m north of the door, another start 245 m south: inside
    /// the trim ring plus a sample's slack per end.
    #[test]
    fn a_declared_seed_excludes_starts_inside_the_model() {
        let points = [trimmed("n", 0.0, 245.0), trimmed("s", 0.0, -245.0)];
        let after = run_with(&points, &[], &[], declared(0.0, 245.0));
        assert_eq!(
            after.excluded,
            vec![key("n"), key("s")],
            "bug: a start from the door was kept"
        );
    }

    /// One end took the whole slack on each side: 600 m apart, past a
    /// 500 m radius.
    #[test]
    fn a_declared_seed_covers_the_slack_margin() {
        let points = [trimmed("n", 0.0, 300.0), trimmed("s", 0.0, -300.0)];
        let after = run_with(&points, &[], &[], declared(0.0, 300.0));
        assert_eq!(
            after.excluded,
            vec![key("n"), key("s")],
            "bug: the far start escaped the exclusion"
        );
    }

    #[test]
    fn elsewhere_excludes_nothing() {
        let points = visits("t", 6, 0.0, 0.0);
        let endpoints: Vec<(f64, f64)> = points.iter().flat_map(|s| [(s.lat, s.lon); 2]).collect();
        let after = run_with(&points, &endpoints, &[], HomeDeclaration::Elsewhere);
        assert!(
            after.excluded.is_empty(),
            "bug: elsewhere excluded starts: {after:?}"
        );
        assert_eq!(
            memberships(&after),
            vec![keys("t", 6)],
            "bug: the trailhead formed no place"
        );
    }

    /// A named place 650 m from the seed, inside the 700 m exclusion, is
    /// past `HOME_WINS_METERS`: its name wins and its starts stay.
    #[test]
    fn a_named_place_far_from_the_seed_keeps_its_starts() {
        let named = [anchor("club", 0.0, 650.0, true, 3)];
        let after = run_with(&visits("c", 3, 0.0, 650.0), &[], &named, declared(0.0, 0.0));
        assert!(
            after.excluded.is_empty(),
            "bug: a far named place lost starts: {after:?}"
        );
        assert_eq!(
            after.places[0].members,
            keys("c", 3),
            "bug: the named place lost its starts"
        );
    }

    /// A named place 200 m from the seed: home wins, and the place survives
    /// empty under its id.
    #[test]
    fn a_named_place_near_the_seed_loses_its_starts_and_keeps_its_name() {
        let named = [anchor("porch", 0.0, 200.0, true, 3)];
        let after = run_with(&visits("p", 3, 0.0, 200.0), &[], &named, declared(0.0, 0.0));
        assert_eq!(
            after.excluded,
            keys("p", 3),
            "bug: a named place near the seed kept the door's starts"
        );
        assert_eq!(
            after.places.len(),
            1,
            "bug: the named place is gone: {after:?}"
        );
        assert_eq!(
            after.places[0].id, "porch",
            "bug: the named place lost its id"
        );
        assert!(
            after.places[0].members.is_empty(),
            "bug: the named place kept members"
        );
    }

    /// A ride from the door at the origin to an area centred `(east, north)`
    /// and back: its trimmed track leaves the trim ring toward the area in
    /// 100 m steps, rides 48 points around a 150 m circle there (`phase`
    /// turns where it starts), and comes back the way it went.
    fn trip(id: &str, east: f64, north: f64, phase: f64) -> StartPoint {
        let reach = east.hypot(north);
        let (unit_east, unit_north) = (east / reach, north / reach);
        let along = |meters: f64| at(unit_east * meters, unit_north * meters);
        let steps = ((reach - 400.0) / 100.0).floor() as usize;
        let out: Vec<(f64, f64)> = (0..=steps)
            .map(|step| along((step as f64).mul_add(100.0, 200.0)))
            .collect();
        let circle = (0..48).map(|i| {
            let angle = (f64::from(i) / 48.0).mul_add(TAU, phase);
            at(
                angle.cos().mul_add(150.0, east),
                angle.sin().mul_add(150.0, north),
            )
        });
        let mut track = out.clone();
        track.extend(circle);
        track.extend(out.iter().rev());
        StartPoint {
            key: key(id),
            lat: track[0].0,
            lon: track[0].1,
            kind: StartKind::Trimmed,
            track,
        }
    }

    /// A walk around the block: a 300 m square off the trim ring, its
    /// farthest point about 580 m from the door.
    fn walk(id: &str, turn: f64) -> StartPoint {
        let (sin, cos) = turn.sin_cos();
        let corner = |east: f64, north: f64| {
            at(
                east.mul_add(cos, -(north * sin)),
                east.mul_add(sin, north * cos),
            )
        };
        let track = vec![
            corner(200.0, 0.0),
            corner(500.0, 0.0),
            corner(500.0, 300.0),
            corner(200.0, 300.0),
            corner(0.0, 200.0),
        ];
        StartPoint {
            key: key(id),
            lat: track[0].0,
            lon: track[0].1,
            kind: StartKind::Trimmed,
            track,
        }
    }

    /// Every start's trimmed first and last point.
    fn ends(points: &[StartPoint]) -> Vec<(f64, f64)> {
        points
            .iter()
            .flat_map(|point| {
                [
                    point
                        .track
                        .first()
                        .copied()
                        .unwrap_or((point.lat, point.lon)),
                    point
                        .track
                        .last()
                        .copied()
                        .unwrap_or((point.lat, point.lon)),
                ]
            })
            .collect()
    }

    /// Three rides from the door to an area 2.6 km east-north-east, three
    /// to one 2.6 km east-south-east, about 1.3 km apart (jf's Gailuron and
    /// Forêt Héritage), and four walks around the block.
    fn one_door_two_areas() -> Vec<StartPoint> {
        let mut points: Vec<StartPoint> = (0..3)
            .map(|i| trip(&format!("g{i}"), 2_500.0, 650.0, f64::from(i)))
            .collect();
        points.extend((0..3).map(|i| trip(&format!("f{i}"), 2_500.0, -650.0, f64::from(i))));
        points.extend((0..4).map(|i| walk(&format!("w{i}"), f64::from(i) * 1.5)));
        points
    }

    /// Each place's area flag and members, ordered by members.
    fn area_members(clustering: &Clustering) -> Vec<(bool, Vec<CachedRowKey>)> {
        let mut found: Vec<(bool, Vec<CachedRowKey>)> = clustering
            .places
            .iter()
            .map(|place| (place.area, place.members.clone()))
            .collect();
        found.sort_by(|a, b| a.1.cmp(&b.1));
        found
    }

    #[test]
    fn rides_from_one_door_into_two_areas_make_two_area_places() {
        let points = one_door_two_areas();
        let after = run_with(&points, &ends(&points), &[], HomeDeclaration::Inferred);
        assert_eq!(
            area_members(&after),
            vec![(true, keys("f", 3)), (true, keys("g", 3))],
            "bug: the rides from the door are not placed by the area they rode to: {after:?}"
        );
        assert_eq!(
            after.excluded,
            keys("w", 4),
            "bug: only the walks that never left the block stay excluded: {after:?}"
        );
    }

    #[test]
    fn a_walk_that_never_leaves_the_block_stays_excluded() {
        let points: Vec<StartPoint> = (0..6)
            .map(|i| walk(&format!("w{i}"), f64::from(i)))
            .collect();
        let after = run_with(&points, &ends(&points), &[], HomeDeclaration::Inferred);
        assert!(
            after.places.is_empty(),
            "bug: a walk made a place: {after:?}"
        );
        assert_eq!(
            after.excluded,
            keys("w", 6),
            "bug: a walk left the exclusion"
        );
    }

    /// Rides that circle the door 1 km out: every far point is far, and
    /// their mean is the door itself. Nothing near the door is ever a place.
    #[test]
    fn no_centroid_lies_near_the_door_even_for_a_ride_around_it() {
        let circling: Vec<StartPoint> = (0..3)
            .map(|i| {
                let mut track = vec![at(200.0, 0.0)];
                track.extend((0..64).map(|step| {
                    let angle = (f64::from(step) / 64.0).mul_add(TAU, f64::from(i));
                    at(angle.cos() * 1_000.0, angle.sin() * 1_000.0)
                }));
                track.push(at(0.0, 200.0));
                StartPoint {
                    key: key(&format!("c{i}")),
                    lat: track[0].0,
                    lon: track[0].1,
                    kind: StartKind::Trimmed,
                    track,
                }
            })
            .collect();
        let mut points = one_door_two_areas();
        points.extend(circling);
        for home in [HomeDeclaration::Inferred, declared(0.0, 200.0)] {
            let after = run_with(&points, &ends(&points), &[], home);
            assert_eq!(
                after.places.len(),
                2,
                "bug: precondition: the two areas must be places, or this proves nothing: {after:?}"
            );
            assert!(
                after
                    .excluded
                    .iter()
                    .any(|row| row.activity_id.starts_with('c')),
                "bug: a ride around the door was placed by the door's own point: {after:?}"
            );
            for place in &after.places {
                assert!(
                    haversine_meters_between(place.lat, place.lon, ORIGIN.0, ORIGIN.1)
                        > PlaceParams::default().home_exclusion_m + DEFAULT_PRIVACY_RADIUS_METERS,
                    "bug: a place's centroid lies near the door: {place:?}"
                );
            }
        }
    }

    /// The athlete named the first area: a recompute keeps its id and
    /// centroid, its rides, and the door's walks out of it.
    #[test]
    fn a_named_area_keeps_its_rides_and_never_takes_the_doors_starts() {
        let points = one_door_two_areas();
        let endpoints = ends(&points);
        let first = run_with(&points, &endpoints, &[], HomeDeclaration::Inferred);
        let area = first
            .places
            .iter()
            .find(|place| place.members == keys("g", 3))
            .expect("bug: precondition: the first area must be a place");
        let named = [PlaceAnchor {
            id: "gailuron".to_owned(),
            lat: area.lat,
            lon: area.lon,
            radius_m: 900.0,
            athlete_named: true,
            activity_count: 3,
        }];
        let after = run_with(&points, &endpoints, &named, HomeDeclaration::Inferred);
        let place = after
            .places
            .iter()
            .find(|place| place.id == "gailuron")
            .expect("bug: the named area lost its id");
        assert_eq!(
            place.members,
            keys("g", 3),
            "bug: the named area's rides changed: {after:?}"
        );
        assert!(
            haversine_meters_between(place.lat, place.lon, area.lat, area.lon) < 1.0,
            "bug: the named area's centroid moved"
        );
        assert_eq!(
            after.excluded,
            keys("w", 4),
            "bug: a walk from the door joined a place"
        );
    }

    /// The athlete named an area after one ride; the next ride there ends
    /// up 450 m off. It joins the name, and makes no second place.
    #[test]
    fn a_named_area_takes_the_next_ride_into_it() {
        let ride = trip("g0", 2_500.0, 650.0, 0.0);
        let mut points = vec![ride, walk("w0", 0.0), walk("w1", 2.0), walk("w2", 4.0)];
        let first = run_with(&points, &ends(&points), &[], HomeDeclaration::Inferred);
        let area = first
            .places
            .iter()
            .find(|place| place.area)
            .expect("bug: precondition: one ride from home must make an area")
            .clone();
        let named = [PlaceAnchor {
            id: "gailuron".to_owned(),
            lat: area.lat,
            lon: area.lon,
            radius_m: area.radius_m,
            athlete_named: true,
            activity_count: 1,
        }];
        points.push(trip("g1", 2_500.0, 1_100.0, 1.0));
        let after = run_with(&points, &ends(&points), &named, HomeDeclaration::Inferred);
        assert_eq!(
            memberships(&after),
            vec![vec![key("g0"), key("g1")]],
            "bug: the next ride into the named area made its own place: {after:?}"
        );
        assert_eq!(
            after.places[0].id, "gailuron",
            "bug: the name did not stick"
        );
    }

    /// An unnamed area keeps its id across a recompute with one more ride.
    #[test]
    fn an_area_keeps_its_id_when_new_rides_arrive() {
        let mut points = one_door_two_areas();
        let first = run_with(&points, &ends(&points), &[], HomeDeclaration::Inferred);
        assert_eq!(
            first.places.len(),
            2,
            "bug: precondition: the two areas must be places, or this proves nothing: {first:?}"
        );
        let previous: Vec<PlaceAnchor> = first
            .places
            .iter()
            .map(|place| PlaceAnchor {
                id: place.id.clone(),
                lat: place.lat,
                lon: place.lon,
                radius_m: place.radius_m,
                athlete_named: false,
                activity_count: 3,
            })
            .collect();
        points.push(trip("g3", 2_500.0, 650.0, 3.0));
        let after = run_with(
            &points,
            &ends(&points),
            &previous,
            HomeDeclaration::Inferred,
        );
        let mut before_ids: Vec<&str> = first.places.iter().map(|p| p.id.as_str()).collect();
        let mut after_ids: Vec<&str> = after.places.iter().map(|p| p.id.as_str()).collect();
        before_ids.sort_unstable();
        after_ids.sort_unstable();
        assert_eq!(after_ids, before_ids, "bug: an area lost its id: {after:?}");
    }

    /// A declared door: the rides still go to their areas; elsewhere, no
    /// start is home and no place is an area.
    #[test]
    fn a_declared_door_has_areas_and_elsewhere_has_none() {
        let points = one_door_two_areas();
        let at_door = run_with(&points, &[], &[], declared(0.0, 200.0));
        assert_eq!(
            area_members(&at_door),
            vec![(true, keys("f", 3)), (true, keys("g", 3))],
            "bug: a declared door's rides are not placed by their areas: {at_door:?}"
        );
        let elsewhere = run_with(&points, &[], &[], HomeDeclaration::Elsewhere);
        assert!(
            elsewhere.places.iter().all(|place| !place.area),
            "bug: an area with no home: {elsewhere:?}"
        );
    }

    /// A named area 945 m from a declared seed, its radius reaching a door
    /// start 455 m off it and 490 m from the seed: the area keeps its rides
    /// and its centroid, and the door start stays home.
    #[test]
    fn a_named_area_beyond_the_door_never_spares_a_door_start() {
        let area = trip("g0", 0.0, -1_400.0, 0.0);
        let mut points = vec![area, trimmed("door", 0.0, -245.0)];
        points.push(walk("w0", 0.0));
        let named = [PlaceAnchor {
            radius_m: 500.0,
            ..anchor("area", 0.0, -700.0, true, 1)
        }];
        let after = run_with(&points, &[], &named, declared(0.0, 245.0));
        assert!(
            after.excluded.contains(&key("door")),
            "bug: a named area spared a door start: {after:?}"
        );
        let place = after
            .places
            .iter()
            .find(|place| place.id == "area")
            .expect("bug: the named area is gone");
        assert!(
            !place.members.contains(&key("door")),
            "bug: a door start joined the named area: {place:?}"
        );
    }
}
