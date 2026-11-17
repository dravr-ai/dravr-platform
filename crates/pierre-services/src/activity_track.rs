// ABOUTME: The one read and decode of a stored route track, and the provider-free local track read the places path uses
// ABOUTME: A cached row's trimmed track from its polyline, its stored Home map or its stored samples, never from a provider

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Activity tracks read from what the platform already stores (carnet#1484).
//!
//! [`stored_track`] is the one read and decode of `activity_route_tracks`:
//! the Home map reads its stored answer through it, and so does the places
//! path. [`local_track`] finds a cached row's trimmed track without asking any
//! provider: the route overview the row carries, else the map Home stored for
//! it, else the samples stored for it (a push, an upload, an archive). All
//! three are trimmed by [`RouteTrack`] before they reach a caller, so their
//! first and last points sit past the privacy radius.

use chrono::{DateTime, Utc};
use pierre_core::errors::AppResult;
use pierre_core::models::{Activity, SportType, TenantId};
use pierre_database::repositories::{CachedActivityRow, StoredRouteOutcome, StoredRouteTrack};
use pierre_database::RepositoryRegistry;
use pierre_fitness_compute::route_track::{RouteTrack, RouteTrackError};
use pierre_providers::core::OwnerStores;
use tracing::warn;
use uuid::Uuid;

/// One activity's stored route read, decoded.
#[derive(Debug, Clone, PartialEq)]
pub enum StoredTrack {
    /// A drawable track, trimmed and simplified to a Home map's points.
    Drawn {
        /// The track.
        track: RouteTrack,
    },
    /// No drawable track, and why.
    Unavailable {
        /// Why there is none: `no_gps` or `too_short` when a read settled
        /// it, `unavailable` when it did not.
        reason: String,
        /// When the answer is read again; `None` when it stands.
        expires_at: Option<DateTime<Utc>>,
    },
}

/// The stored route read for one activity, decoded.
///
/// `None` when no read is stored, the stored one has expired, or a drawn row
/// no longer decodes. The last is logged: the
/// caller reads the route again and its write replaces the row.
///
/// # Errors
///
/// Returns the repository error when the stored read fails.
pub async fn stored_track(
    repos: &RepositoryRegistry,
    tenant_id: &TenantId,
    user_id: Uuid,
    provider_key: &str,
    activity_id: &str,
) -> AppResult<Option<StoredTrack>> {
    let stored = repos
        .activity_route_tracks
        .get_route_track(tenant_id, user_id, provider_key, activity_id)
        .await?;
    Ok(stored.and_then(|stored| match stored {
        StoredRouteTrack::Drawn { track_json, .. } => {
            let decoded = serde_json::from_str::<RouteTrack>(&track_json).ok();
            if decoded.is_none() {
                // The track shape has evolved since the row was written: a
                // miss, so the caller reads again and the upsert overwrites it.
                warn!(
                    activity_id,
                    "stored route track no longer decodes; reading it again"
                );
            }
            decoded.map(|track| StoredTrack::Drawn { track })
        }
        StoredRouteTrack::Unavailable {
            reason, expires_at, ..
        } => Some(StoredTrack::Unavailable { reason, expires_at }),
    }))
}

/// A cached row's trimmed track, read from what is stored locally, cheapest
/// first: the route overview the row carries, then the map Home stored for it,
/// then its stored samples. Every source gives the same trimmed endpoints, so
/// the order only spares reads.
///
/// No provider is asked. A row whose sport records no outdoor route (a
/// trainer ride, a virtual sport, anything outside the outdoor GPS sports)
/// reads nothing. Nor does a row whose stored route read already settled that
/// it has none (`no_gps`, `too_short`): the cached row carries that answer,
/// so no read repeats it. The stored map is read only when the row says one is
/// drawn.
///
/// # Errors
///
/// Returns the repository error when a stored read fails.
pub(crate) async fn local_track(
    repos: &RepositoryRegistry,
    tenant_id: &TenantId,
    user_id: Uuid,
    row: &CachedActivityRow,
) -> AppResult<Option<RouteTrack>> {
    if !records_outdoor_route(&row.activity) || route_settled_absent(row.route.as_ref()) {
        return Ok(None);
    }
    if let Some(track) = row
        .activity
        .summary_polyline()
        .and_then(RouteTrack::trimmed_overview)
    {
        return Ok(Some(track));
    }
    let activity_id = row.activity.id();
    if row.route == Some(StoredRouteOutcome::Drawn) {
        let stored = stored_track(repos, tenant_id, user_id, &row.provider, activity_id).await?;
        if let Some(StoredTrack::Drawn { track, .. }) = stored {
            return Ok(Some(track));
        }
    }
    let samples = OwnerStores::from_repos(repos, user_id, *tenant_id)
        .stored_activity_streams(&row.provider, activity_id)
        .await?;
    Ok(samples.and_then(|samples| RouteTrack::from_streams(&samples).ok()))
}

/// Whether the cached row's stored route read settled that it has no track.
fn route_settled_absent(route: Option<&StoredRouteOutcome>) -> bool {
    matches!(
        route,
        Some(StoredRouteOutcome::Unavailable { reason })
            if RouteTrackError::from_slug(reason).is_some()
    )
}

/// Whether the activity is an outdoor GPS sport that records a route: a ride
/// (road, mountain, gravel, e-bike), a run or trail run, a hike or walk, or a
/// nordic ski. A trainer session never is, whatever its sport.
pub(crate) fn records_outdoor_route(activity: &Activity) -> bool {
    activity.trainer() != Some(true)
        && matches!(
            activity.sport_type(),
            SportType::Ride
                | SportType::MountainBike
                | SportType::GravelRide
                | SportType::EbikeRide
                | SportType::Run
                | SportType::TrailRunning
                | SportType::Hike
                | SportType::Walk
                | SportType::CrossCountrySkiing
        )
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use pierre_core::models::{ActivityBuilder, TimeSeriesData, User};
    use pierre_database::backends::factory::Database;
    use pierre_database::repositories::{ActivityStreamsWrite, StoredRouteTrack, StreamOrigin};
    use pierre_fitness_compute::encode_polyline;
    use pierre_fitness_compute::routes::haversine_meters_between;
    use pierre_test_support::db::create_test_db;

    use super::*;

    /// A straight 5 km line north from `(lat, lon)`, a point every ~100 m.
    fn line(lat: f64, lon: f64) -> Vec<(f64, f64)> {
        (0..50)
            .map(|i| (f64::from(i).mul_add(0.0009, lat), lon))
            .collect()
    }

    /// The three places a track can come from, far apart, so a returned
    /// track names its source by where it starts.
    const POLYLINE: (f64, f64) = (45.50, -73.60);
    const MAP: (f64, f64) = (46.50, -72.60);
    const SAMPLES: (f64, f64) = (47.50, -71.60);

    struct World {
        db: Database,
        user: Uuid,
        tenant: TenantId,
    }

    impl World {
        async fn new() -> Self {
            let db = create_test_db().await.unwrap();
            let user = User::new(
                format!("track-{}@example.com", Uuid::new_v4()),
                "argon2-hash-placeholder".to_owned(),
                None,
            );
            let user = db.repositories().users.create(&user).await.unwrap();
            Self {
                db,
                user,
                tenant: TenantId::generate(),
            }
        }

        fn repos(&self) -> &RepositoryRegistry {
            self.db.repositories()
        }

        async fn store_map(&self, activity_id: &str) {
            let track = RouteTrack::from_overview(&line(MAP.0, MAP.1)).unwrap();
            self.repos()
                .activity_route_tracks
                .upsert_route_track(
                    &self.tenant,
                    self.user,
                    "garmin",
                    activity_id,
                    &StoredRouteTrack::Drawn {
                        source: "streams".to_owned(),
                        track_json: serde_json::to_string(&track).unwrap(),
                    },
                )
                .await
                .unwrap();
        }

        async fn store_samples(&self, activity_id: &str) {
            let points = line(SAMPLES.0, SAMPLES.1);
            let streams = TimeSeriesData {
                timestamps: (0..50).collect(),
                heart_rate: None,
                power: None,
                cadence: None,
                speed: None,
                altitude: None,
                temperature: None,
                gps_coordinates: Some(points.into_iter().map(Some).collect()),
                distance: None,
            };
            self.repos()
                .activity_streams
                .upsert_activity_streams(&ActivityStreamsWrite {
                    tenant_id: &self.tenant,
                    user_id: self.user,
                    provider: "garmin",
                    activity_id,
                    origin: StreamOrigin::Push,
                    source_ref: None,
                    streams: &streams,
                    synced_at: Utc::now(),
                })
                .await
                .unwrap();
        }

        async fn track(&self, row: &CachedActivityRow) -> Option<RouteTrack> {
            local_track(self.repos(), &self.tenant, self.user, row)
                .await
                .unwrap()
        }
    }

    fn row(
        id: &str,
        sport: SportType,
        polyline: bool,
        route: Option<StoredRouteOutcome>,
    ) -> CachedActivityRow {
        let start = Utc.with_ymd_and_hms(2026, 6, 1, 8, 0, 0).unwrap();
        let builder = ActivityBuilder::new(id, "Ride", sport, start, 3_600, "garmin");
        let builder = if polyline {
            builder.summary_polyline(encode_polyline(&line(POLYLINE.0, POLYLINE.1)))
        } else {
            builder
        };
        CachedActivityRow {
            provider: "garmin".to_owned(),
            activity: builder.build(),
            route,
            detail_read: false,
            segment_read: false,
            stored_at: None,
            place: None,
        }
    }

    /// Which of the three sources a track came from, by its first point.
    fn source_of(track: &RouteTrack) -> (f64, f64) {
        let (lat, _) = track.first_point().unwrap();
        [POLYLINE, MAP, SAMPLES]
            .into_iter()
            .find(|origin| (lat - origin.0).abs() < 0.01)
            .unwrap_or_else(|| panic!("bug: a track starting at {lat} came from no source"))
    }

    #[tokio::test]
    async fn the_polyline_answers_before_the_stored_map_and_the_samples() {
        let world = World::new().await;
        world.store_map("a1").await;
        world.store_samples("a1").await;
        let all = row("a1", SportType::Ride, true, Some(StoredRouteOutcome::Drawn));
        let track = world
            .track(&all)
            .await
            .expect("bug: a row with a polyline has a track");
        assert_eq!(
            source_of(&track),
            POLYLINE,
            "bug: the polyline costs no read, so it answers first"
        );

        let no_polyline = row(
            "a1",
            SportType::Ride,
            false,
            Some(StoredRouteOutcome::Drawn),
        );
        let track = world
            .track(&no_polyline)
            .await
            .expect("bug: a drawn stored map is a track");
        assert_eq!(
            source_of(&track),
            MAP,
            "bug: the stored map answers before the samples"
        );

        let samples_only = row("a1", SportType::Ride, false, None);
        let track = world
            .track(&samples_only)
            .await
            .expect("bug: stored samples are a track");
        assert_eq!(
            source_of(&track),
            SAMPLES,
            "bug: the samples answer a row with no polyline and no drawn map"
        );
    }

    #[tokio::test]
    async fn every_track_is_trimmed_past_the_privacy_radius() {
        let world = World::new().await;
        world.store_samples("a1").await;
        for (row, origin) in [
            (row("a1", SportType::Ride, true, None), POLYLINE),
            (row("a1", SportType::Ride, false, None), SAMPLES),
        ] {
            let track = world.track(&row).await.unwrap();
            let (lat, lon) = track.first_point().unwrap();
            let meters = haversine_meters_between(lat, lon, origin.0, origin.1);
            assert!(
                meters >= 150.0,
                "bug: a local track starts {meters:.0} m from the recorded start, inside the privacy radius"
            );
        }
    }

    #[tokio::test]
    async fn a_settled_absence_reads_nothing_even_when_a_track_is_stored() {
        // A stored map and samples exist under the row's key; a read would
        // find them. The row's projected route says no GPS, so none happens.
        let world = World::new().await;
        world.store_map("a1").await;
        world.store_samples("a1").await;
        for reason in ["no_gps", "too_short"] {
            let settled = row(
                "a1",
                SportType::Ride,
                false,
                Some(StoredRouteOutcome::Unavailable {
                    reason: reason.to_owned(),
                }),
            );
            assert_eq!(
                world.track(&settled).await,
                None,
                "bug: a row whose route settled {reason} must read neither the map nor the samples"
            );
        }
    }

    #[tokio::test]
    async fn the_stored_map_is_read_only_when_the_row_says_one_is_drawn() {
        let world = World::new().await;
        world.store_map("a1").await;
        for route in [
            None,
            Some(StoredRouteOutcome::Unavailable {
                reason: "unavailable".to_owned(),
            }),
        ] {
            let unread = row("a1", SportType::Ride, false, route.clone());
            assert_eq!(
                world.track(&unread).await,
                None,
                "bug: a row whose route is {route:?} must not read the stored map"
            );
        }
    }

    #[tokio::test]
    async fn indoor_rows_read_no_track() {
        let world = World::new().await;
        world.store_samples("a1").await;
        let mut trainer = row("a1", SportType::Ride, true, None);
        trainer.activity = ActivityBuilder::new(
            "a1",
            "Trainer",
            SportType::Ride,
            trainer.activity.start_date(),
            3_600,
            "garmin",
        )
        .summary_polyline(encode_polyline(&line(POLYLINE.0, POLYLINE.1)))
        .trainer(true)
        .build();
        assert_eq!(
            world.track(&trainer).await,
            None,
            "bug: a trainer session has no outdoor route"
        );
        for sport in [
            SportType::VirtualRide,
            SportType::VirtualRun,
            SportType::Yoga,
        ] {
            let indoor = row("a1", sport.clone(), true, None);
            assert_eq!(
                world.track(&indoor).await,
                None,
                "bug: a {sport:?} row records no outdoor route"
            );
        }
    }
}
