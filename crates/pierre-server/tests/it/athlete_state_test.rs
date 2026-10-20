// ABOUTME: carnet#828 — the athlete state the use-case starters rank from, read from a real database
// ABOUTME: Pins each field to the rows it comes from, and the transport stamp to what was read
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use chrono::{DateTime, Datelike, Duration, NaiveDate, Utc};
use pierre_contremaitre::use_case_catalogue::{Predicate, Stage};
use pierre_core::models::periodization::PhaseKind;
use pierre_core::models::{
    ActivityBuilder, ConnectionType, SportType, Tenant, TenantId, User, UserStatus,
};
use pierre_core::transport::TransportPolicy;
use pierre_database::repositories::training_plans::PlanAuthor;
use pierre_database::repositories::{PlanOutlineInput, PlanWeekInput, SavePlanBundleParams};
use pierre_database::RepositoryRegistry;
use pierre_memory::training_plans::{GoalRace, PlanPhase, PlannedDay, RacePriority};
use pierre_services::athlete_state::{ActivityHistory, AthleteState};
use pierre_test_support::db::create_test_db;
use std::collections::BTreeMap;
use uuid::Uuid;

/// A user created `age` ago, and a tenant they own.
async fn athlete(repos: &RepositoryRegistry, age: Duration) -> (Uuid, TenantId) {
    let mut user = User::new(
        format!("athlete-{}@example.com", Uuid::new_v4()),
        "hash".to_owned(),
        Some("Athlete".to_owned()),
    );
    user.user_status = UserStatus::Active;
    user.created_at = Utc::now() - age;
    let user_id = user.id;
    repos.users.create(&user).await.unwrap();
    let tenant_id = TenantId::generate();
    repos
        .tenants
        .create(&Tenant {
            id: tenant_id,
            name: "athlete tenant".to_owned(),
            slug: format!("athlete-{tenant_id}"),
            domain: None,
            plan: "starter".to_owned(),
            owner_user_id: user_id,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
        .await
        .unwrap();
    (user_id, tenant_id)
}

async fn connect(repos: &RepositoryRegistry, (user_id, tenant): (Uuid, TenantId), provider: &str) {
    repos
        .provider_connections
        .register_connection(user_id, tenant, provider, &ConnectionType::OAuth, None)
        .await
        .unwrap();
}

async fn ride(
    repos: &RepositoryRegistry,
    (user_id, tenant): (Uuid, TenantId),
    provider: &str,
    at: DateTime<Utc>,
) {
    let activity = ActivityBuilder::new(
        format!("{provider}-{}", Uuid::new_v4()),
        "Morning ride".to_owned(),
        SportType::Ride,
        at,
        3_600,
        provider.to_owned(),
    )
    .build();
    repos
        .activity_cache
        .upsert_activities(user_id, &tenant, provider, &[activity])
        .await
        .unwrap();
}

/// A season, and a week starting `week_start` with one session on `day`.
async fn season(
    repos: &RepositoryRegistry,
    (user_id, tenant): (Uuid, TenantId),
    week_start: NaiveDate,
    policy: TransportPolicy,
) {
    let race = GoalRace {
        name: "Gran Fondo".to_owned(),
        date: (week_start + Duration::weeks(12)).to_string(),
        discipline: "road".to_owned(),
        priority: RacePriority::A,
    };
    let phases = [PlanPhase {
        kind: PhaseKind::Build,
        start: week_start.to_string(),
        weeks: 12,
        intent: "build the engine".to_owned(),
        target_hours: Some(8.0),
        purpose: String::new(),
        volume_share_of_peak: None,
        tid_target: None,
        hard_sessions_max: None,
        session_mix: BTreeMap::new(),
        flavour_override: None,
        loading_pattern: None,
        skeleton_id: None,
    }];
    let days = [PlannedDay {
        date: week_start.to_string(),
        sport: "ride".to_owned(),
        workout: "endurance".to_owned(),
        duration_min: Some(90),
        intensity: "Z2".to_owned(),
        steps: Vec::new(),
        fueling: None,
        template_slug: None,
        template_params: None,
        template_source: None,
    }];
    let week_start = week_start.to_string();
    let (tenant, user) = (tenant.to_string(), user_id.to_string());
    repos
        .training_plans
        .save_plan_bundle(&SavePlanBundleParams {
            tenant_id: &tenant,
            user_id: &user,
            author: PlanAuthor::agent("endurance-coach"),
            goal_fact_id: None,
            replace_season: false,
            outline: Some(PlanOutlineInput {
                goal_race: &race,
                races: Some(&[]),
                strategy: "build, then sharpen",
                flavour: None,
                season_start: None,
                season_end: None,
                phases: &phases,
                source_conversation_id: None,
            }),
            weeks: &[PlanWeekInput {
                week_start: &week_start,
                focus: "volume",
                days: &days,
                adjustment_reason: "",
                phase_index: Some(0),
            }],
            transport_policy: policy,
        })
        .await
        .unwrap();
}

fn this_monday() -> NaiveDate {
    let today = Utc::now().date_naive();
    today - Duration::days(i64::from(today.weekday().num_days_from_monday()))
}

async fn read(
    repos: &RepositoryRegistry,
    (user_id, tenant): (Uuid, TenantId),
) -> (AthleteState, TransportPolicy) {
    AthleteState::read(repos, tenant, user_id, Utc::now())
        .await
        .unwrap()
}

#[tokio::test]
async fn a_new_athlete_with_nothing_connected_is_a_blank_first_session() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let who = athlete(repos, Duration::minutes(5)).await;

    let (state, policy) = read(repos, who).await;

    assert_eq!(state.stage, Stage::FirstSession);
    assert!(!state.has_provider);
    assert_eq!(state.activities, ActivityHistory::Absent);
    assert_eq!(state.weeks_of_data, 0);
    assert!(state.dossier.empty);
    assert!(!state.season_set && !state.plan_active);
    assert!(!state.recovery_source && !state.calendar_writable);
    assert!(state.holds(Predicate::NoProvider) && !state.holds(Predicate::HasActivities));
    assert_eq!(policy, TransportPolicy::AnyTransport, "nothing was read");
}

#[tokio::test]
async fn a_provider_nobody_has_read_yet_counts_as_having_activities() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let who = athlete(repos, Duration::days(3)).await;
    connect(repos, who, "strava").await;

    let (state, _) = read(repos, who).await;

    assert_eq!(state.stage, Stage::FirstWeek);
    assert!(state.has_provider);
    assert_eq!(state.activities, ActivityHistory::NotYetRead);
    assert!(state.holds(Predicate::HasActivities));
    assert!(!state.holds(Predicate::NoProvider));
}

#[tokio::test]
async fn strava_rides_over_two_weeks_count_and_stamp_the_state_first_party() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let who = athlete(repos, Duration::days(40)).await;
    connect(repos, who, "strava").await;
    let now = Utc::now();
    ride(repos, who, "strava", now - Duration::hours(2)).await;
    ride(repos, who, "strava", now - Duration::days(8)).await;
    ride(repos, who, "strava", now - Duration::days(9)).await;

    let (state, policy) = read(repos, who).await;

    assert_eq!(state.stage, Stage::Any);
    assert_eq!(state.activities, ActivityHistory::Present);
    assert!(
        (2..=3).contains(&state.weeks_of_data),
        "two or three ISO weeks depending on the weekday, read {}",
        state.weeks_of_data
    );
    assert!(state.holds(Predicate::WeeksOfData(2)));
    assert!(!state.holds(Predicate::WeeksOfData(4)));
    assert_eq!(
        policy,
        TransportPolicy::FirstPartyOnly,
        "starters chosen from Strava rides are derived from first-party-only data"
    );
}

#[tokio::test]
async fn a_ride_older_than_the_window_is_still_an_activity_but_no_week_of_data() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let who = athlete(repos, Duration::days(90)).await;
    connect(repos, who, "strava").await;
    ride(repos, who, "strava", Utc::now() - Duration::days(60)).await;

    let (state, _) = read(repos, who).await;

    assert_eq!(state.activities, ActivityHistory::Present);
    assert_eq!(state.weeks_of_data, 0);
}

#[tokio::test]
async fn whoop_is_a_recovery_source_and_intervals_takes_a_pushed_plan() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let who = athlete(repos, Duration::days(10)).await;
    connect(repos, who, "strava").await;

    let (strava_only, _) = read(repos, who).await;
    assert!(!strava_only.recovery_source);
    assert!(!strava_only.calendar_writable);

    connect(repos, who, "whoop").await;
    connect(repos, who, "intervals_icu").await;
    let (state, _) = read(repos, who).await;
    assert!(state.holds(Predicate::HasRecoverySource));
    assert!(state.holds(Predicate::CalendarWritable));
}

#[tokio::test]
async fn a_season_with_a_week_covering_today_is_an_active_plan() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let current = athlete(repos, Duration::days(10)).await;
    season(repos, current, this_monday(), TransportPolicy::AnyTransport).await;

    let (state, policy) = read(repos, current).await;
    assert!(state.holds(Predicate::SeasonSet) && !state.holds(Predicate::NoSeason));
    assert!(state.holds(Predicate::PlanActive));
    assert_eq!(policy, TransportPolicy::AnyTransport);

    let later = athlete(repos, Duration::days(10)).await;
    season(
        repos,
        later,
        this_monday() + Duration::weeks(3),
        TransportPolicy::FirstPartyOnly,
    )
    .await;
    let (state, policy) = read(repos, later).await;
    assert!(state.holds(Predicate::SeasonSet));
    assert!(
        !state.holds(Predicate::PlanActive),
        "its first week starts in three weeks"
    );
    assert_eq!(
        policy,
        TransportPolicy::FirstPartyOnly,
        "a season derived from first-party-only data stamps what is read from it"
    );
}
