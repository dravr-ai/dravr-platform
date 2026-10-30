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
    ActivityBuilder, CheckInCadence, ConnectionType, DataSource, DeviceType, SportType,
    StoredRecoveryMetrics, Tenant, TenantId, User, UserStatus,
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

/// One stored recovery day `days_ago`, synced by a data source of `provider`
/// — whatever the ingress: a provider sync, a Garmin push, an export import.
async fn recovery_day(
    repos: &RepositoryRegistry,
    (user_id, tenant): (Uuid, TenantId),
    provider: &str,
    days_ago: i64,
) {
    let data_source_id = repos
        .data_sources
        .upsert_data_source(
            &tenant,
            &DataSource {
                id: String::new(),
                user_id: user_id.to_string(),
                provider: provider.to_owned(),
                device_model: None,
                software_version: None,
                source: None,
                device_type: DeviceType::Watch,
                original_source_name: None,
            },
        )
        .await
        .unwrap();
    let at = Utc::now() - Duration::days(days_ago);
    repos
        .recovery
        .upsert_recovery_metrics(
            &tenant,
            &StoredRecoveryMetrics {
                id: Uuid::new_v4().to_string(),
                user_id: user_id.to_string(),
                data_source_id,
                date: at.date_naive(),
                recovery_score: None,
                readiness_score: None,
                hrv_ms: None,
                hrv_rmssd: Some(58.0),
                resting_heart_rate: Some(47),
                stress_score: None,
                body_battery: None,
                spo2: None,
                respiratory_rate: None,
                skin_temp_deviation: None,
                daily_strain: None,
                athlete_note: None,
                source_name: provider.to_owned(),
                recorded_at: at,
            },
        )
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

/// A recovery source is a stored night or morning, not a connection: the
/// tools answer from rows, so a WHOOP connected but never synced is not one
/// yet, and becomes one with its first stored morning (carnet#1281).
#[tokio::test]
async fn a_recovery_source_is_a_stored_morning_and_intervals_takes_a_pushed_plan() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let who = athlete(repos, Duration::days(10)).await;
    connect(repos, who, "strava").await;

    let (strava_only, _) = read(repos, who).await;
    assert!(!strava_only.recovery_source);
    assert!(!strava_only.calendar_writable);

    connect(repos, who, "whoop").await;
    connect(repos, who, "intervals_icu").await;
    let (connected, _) = read(repos, who).await;
    assert!(
        !connected.holds(Predicate::HasRecoverySource),
        "connected, nothing synced yet"
    );
    assert!(
        connected.holds(Predicate::CalendarWritable),
        "intervals.icu takes a pushed plan: a bug in the calendar target"
    );

    recovery_day(repos, who, "whoop", 1).await;
    let (state, _) = read(repos, who).await;
    assert!(
        state.holds(Predicate::HasRecoverySource),
        "a stored WHOOP morning makes a recovery source"
    );
}

/// An athlete whose wellness arrived only from an imported Garmin export (or
/// a push) holds no recovery-capable connection, and is a recovery source
/// all the same; a morning older than the four-week window is not.
#[tokio::test]
async fn an_export_only_athletes_stored_mornings_make_a_recovery_source() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();

    let imported = athlete(repos, Duration::days(200)).await;
    recovery_day(repos, imported, "garmin_archive", 3).await;
    let (state, _) = read(repos, imported).await;
    assert!(
        state.holds(Predicate::HasRecoverySource),
        "an imported morning three days old counts"
    );

    let lapsed = athlete(repos, Duration::days(200)).await;
    recovery_day(repos, lapsed, "garmin_archive", 40).await;
    let (state, _) = read(repos, lapsed).await;
    assert!(
        !state.holds(Predicate::HasRecoverySource),
        "a morning older than the window does not count"
    );
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

#[tokio::test]
async fn check_in_on_follows_the_cadence_the_athlete_chose() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let who @ (user_id, tenant) = athlete(repos, Duration::days(2)).await;

    let (never_chose, _) = read(repos, who).await;
    assert!(!never_chose.check_in_on, "no cadence row");
    assert!(!never_chose.holds(Predicate::CheckInOn));

    let set = |cadence| {
        repos
            .check_in_preferences
            .set_check_in_cadence(tenant, user_id, cadence, Utc::now())
    };
    set(CheckInCadence::ThreePerWeek).await.unwrap();
    let (armed, _) = read(repos, who).await;
    assert!(armed.holds(Predicate::CheckInOn));

    set(CheckInCadence::Off).await.unwrap();
    let (off, _) = read(repos, who).await;
    assert!(
        !off.holds(Predicate::CheckInOn),
        "off is a choice not to be checked in on"
    );

    let other = athlete(repos, Duration::days(2)).await;
    set(CheckInCadence::Daily).await.unwrap();
    let (stranger, _) = read(repos, other).await;
    assert!(
        !stranger.holds(Predicate::CheckInOn),
        "another athlete's cadence is not theirs"
    );
}
