// ABOUTME: A group roster shares where a member's ride is picked up past the privacy trim, or nothing
// ABOUTME: Never the raw start coordinates a provider recorded at the athlete's door (carnet#1592)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The roster a group's members read never carries a recent activity's raw
//! start, the point a ride is recorded from, which is usually the member's
//! home. Its `start_latitude`/`start_longitude` are the first point of the
//! trimmed route overview (the line with its endpoint neighbourhoods cut, as
//! the Home map and every route sketch draw it), or no coordinates at all.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

#[cfg(feature = "tools-groups")]
mod roster_start_tests {
    use crate::common::create_test_server_resources;
    use dravr_tronc::dates::{Elapsed, Timestamp};
    use pierre_core::models::groups::RosterActivity;
    use pierre_core::models::{
        Activity, ActivityBuilder, ConnectionType, SportType, Tenant, TenantId, User, UserStatus,
    };
    use pierre_fitness_compute::encode_polyline;
    use pierre_fitness_compute::route_track::RouteTrack;
    use pierre_mcp_server::mcp::resources::ServerContext;
    use pierre_tool_runtime::group_fitness::fetch_member_snapshots;
    use pierre_tool_runtime::runtime::ToolRuntime;
    use std::sync::Arc;
    use tokio::task::spawn_blocking;
    use uuid::Uuid;

    /// Where every test ride is recorded as starting: the member's door.
    const DOOR: (f64, f64) = (45.5017, -73.5673);

    /// Metres per degree of latitude, for the test's own distance check.
    const METERS_PER_DEGREE: f64 = 111_320.0;

    async fn seed_member(resources: &ServerContext) -> (Uuid, TenantId) {
        let password_hash =
            spawn_blocking(|| bcrypt::hash("Pass123!", bcrypt::DEFAULT_COST).unwrap())
                .await
                .unwrap();
        let mut user = User::new(
            format!("roster-start-{}@test.com", Uuid::new_v4()),
            password_hash,
            Some("Roster Start".to_owned()),
        );
        user.user_status = UserStatus::Active;
        let user_id = user.id;
        resources.common.repos.users.create(&user).await.unwrap();

        let tenant_id = TenantId::generate();
        let now = Timestamp::now();
        let tenant = Tenant {
            id: tenant_id,
            name: "Roster Start".to_owned(),
            slug: format!("roster-start-{tenant_id}"),
            domain: None,
            plan: "professional".to_owned(),
            owner_user_id: user_id,
            created_at: now,
            updated_at: now,
        };
        resources
            .common
            .repos
            .tenants
            .create(&tenant)
            .await
            .unwrap();
        resources
            .common
            .repos
            .provider_connections
            .register_connection(user_id, tenant_id, "strava", &ConnectionType::OAuth, None)
            .await
            .unwrap();
        (user_id, tenant_id)
    }

    /// A line due east from the door, `steps` × 50 m long, one point every 50 m.
    fn line_east_from_door(steps: u32) -> Vec<(f64, f64)> {
        let meters_per_degree_lon = METERS_PER_DEGREE * DOOR.0.to_radians().cos();
        (0..=steps)
            .map(|i| {
                let east = f64::from(i) * 50.0;
                (DOOR.0, DOOR.1 + east / meters_per_degree_lon)
            })
            .collect()
    }

    /// A ride recorded as starting at the door, with `overview` as its
    /// encoded summary polyline when given.
    fn ride_from_door(id: &str, days_ago: i64, overview: Option<String>) -> Activity {
        ActivityBuilder::new(
            id.to_owned(),
            format!("ride {id}"),
            SportType::Ride,
            Timestamp::now() - Elapsed::days(days_ago),
            3_600,
            "strava".to_owned(),
        )
        .distance_meters(20_000.0)
        .start_latitude(DOOR.0)
        .start_longitude(DOOR.1)
        .summary_polyline_opt(overview)
        .build()
    }

    fn meters_between(a: (f64, f64), b: (f64, f64)) -> f64 {
        let dlat = (a.0 - b.0) * METERS_PER_DEGREE;
        let dlon = (a.1 - b.1) * METERS_PER_DEGREE * a.0.to_radians().cos();
        dlat.hypot(dlon)
    }

    fn start_of(activity: &RosterActivity) -> Option<(f64, f64)> {
        match (activity.start_latitude, activity.start_longitude) {
            (Some(lat), Some(lon)) => Some((lat, lon)),
            (None, None) => None,
            pair => panic!("bug: the roster pairs latitude and longitude, got {pair:?}"),
        }
    }

    async fn roster_of(activities: &[Activity]) -> Vec<RosterActivity> {
        let resources = create_test_server_resources().await.unwrap();
        let (member, tenant) = seed_member(&resources).await;
        resources
            .common
            .repos
            .activity_cache
            .upsert_activities(member, &tenant, "strava", activities)
            .await
            .unwrap();
        let runtime: Arc<dyn ToolRuntime> = resources.clone();
        let snapshots = fetch_member_snapshots(&runtime, &[member], tenant).await;
        assert_eq!(snapshots.len(), 1, "bug: one member asked, one snapshot");
        snapshots[0].recent_activities.clone()
    }

    fn by_name<'a>(roster: &'a [RosterActivity], id: &str) -> &'a RosterActivity {
        let name = format!("ride {id}");
        roster
            .iter()
            .find(|a| a.name == name)
            .unwrap_or_else(|| panic!("bug: {name} is missing from the roster: {roster:?}"))
    }

    /// A ride with a drawable overview shares where the trimmed line picks it
    /// up: never the door, and well clear of it.
    #[tokio::test]
    async fn a_tracked_ride_shares_the_trimmed_start_not_the_door() {
        let overview = encode_polyline(&line_east_from_door(60));
        let trimmed: (f64, f64) = RouteTrack::trimmed_overview(&overview)
            .and_then(|track| track.first_point())
            .expect("bug: a 3 km overview survives the privacy trim");

        let roster = roster_of(&[ride_from_door("tracked", 1, Some(overview))]).await;
        let start = start_of(by_name(&roster, "tracked"));

        let start = start.expect("bug: a ride with a drawable overview shares its trimmed start");
        assert!(
            meters_between(start, DOOR) >= 150.0,
            "bug: the roster shares a point {:.0} m from the door, the raw start",
            meters_between(start, DOOR)
        );
        assert!(
            meters_between(start, trimmed) < 1.0,
            "bug: the roster start {start:?} is not the trimmed overview's first point {trimmed:?}"
        );
    }

    /// A ride with only a raw start, no overview, shares no coordinates.
    #[tokio::test]
    async fn a_ride_with_only_a_raw_start_shares_no_coordinates() {
        let roster = roster_of(&[ride_from_door("bare", 1, None)]).await;
        assert_eq!(
            start_of(by_name(&roster, "bare")),
            None,
            "bug: a ride with no overview shared its raw start"
        );
    }

    /// An overview too short to survive the trim shares nothing either: the
    /// raw start is never the fallback.
    #[tokio::test]
    async fn an_overview_the_trim_consumes_shares_no_coordinates() {
        let overview = encode_polyline(&line_east_from_door(6));
        assert!(
            RouteTrack::trimmed_overview(&overview).is_none(),
            "bug: the fixture must be too short to survive the 200 m trim at each end"
        );

        let roster = roster_of(&[ride_from_door("short", 1, Some(overview))]).await;
        assert_eq!(
            start_of(by_name(&roster, "short")),
            None,
            "bug: a ride whose overview the trim consumed shared its raw start"
        );
    }
}
