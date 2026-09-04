// ABOUTME: Integration tests for intelligence engines and universal tool handlers
// ABOUTME: Tests interaction between fitness intelligence and universal tool execution
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
//! Integration tests for intelligence engines and universal tool handlers
//!
//! Tests the interaction between fitness intelligence engines and the universal
//! tool execution system to ensure proper data flow and analysis integration.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_core::permissions::scopes::OAuthScope;
use std::env;

use anyhow::Result;
use chrono::Utc;
use pierre_core::models::{Activity, ActivityBuilder, SportType, User, UserOAuthToken};
use pierre_database::backends::factory::Database;
use pierre_intelligence::{
    insights::ActivityContext, ActivityAnalyzer, FitnessLevel, MetricsCalculator, TimeAvailability,
    UserFitnessProfile, UserPreferences,
};
use pierre_tool_runtime::protocols::{UniversalRequest, UniversalToolExecutor};
use serde_json::json;
use sqlx::Row;
use uuid::Uuid;

mod common;
use common::*;

/// Test data flow from tool execution through intelligence analysis
#[tokio::test]
async fn test_activity_analysis_through_universal_tools() -> Result<()> {
    let database = create_test_database().await?;

    // Create user first
    let user = User::new(
        "test@example.com".to_owned(),
        "password_hash".to_owned(),
        Some("Test User".to_owned()),
    );
    database.repositories().users.create(&user).await?;

    // Create test user and fitness profile
    let fitness_profile = UserFitnessProfile {
        user_id: user.id.to_string(),
        age: Some(30),
        gender: Some("male".to_owned()),
        weight: Some(75.0),
        height: Some(180.0),
        fitness_level: FitnessLevel::Intermediate,
        primary_sports: vec!["running".to_owned(), "cycling".to_owned()],
        training_history_months: 24,
        preferences: UserPreferences {
            preferred_units: "metric".to_owned(),
            training_focus: vec!["endurance".to_owned(), "speed".to_owned()],
            injury_history: vec![],
            time_availability: TimeAvailability {
                hours_per_week: 6.0,
                preferred_days: vec![
                    "monday".to_owned(),
                    "wednesday".to_owned(),
                    "friday".to_owned(),
                ],
                preferred_duration_minutes: Some(60),
            },
        },
        seasonal_context: None,
    };

    // Store user fitness profile in database
    let profile_data = serde_json::to_value(&fitness_profile)?;
    database
        .repositories()
        .profiles
        .upsert_profile(user.id, profile_data)
        .await?;

    // Create test activity with advanced metrics
    let activity = ActivityBuilder::new(
        "test_activity_001",
        "Morning Tempo Run",
        SportType::Run,
        Utc::now() - chrono::Duration::hours(2),
        3600, // 60 minutes
        "strava",
    )
    .distance_meters(10000.0) // 10km
    .elevation_gain(100.0)
    .average_heart_rate(165)
    .max_heart_rate(185)
    .average_speed(2.78) // ~4:00/km pace
    .max_speed(3.33)
    .calories(600)
    .steps(12000)
    .average_cadence(180) // steps per minute
    .max_cadence(200)
    .hrv_score(45.2)
    .recovery_heart_rate(25) // HR drop in first minute
    .temperature(18.0)
    .humidity(65.0)
    .average_altitude(120.0)
    .wind_speed(2.0)
    .ground_contact_time(240)
    .vertical_oscillation(8.5)
    .stride_length(1.25)
    .running_power(280)
    .breathing_rate(32)
    .spo2(98.0)
    .training_stress_score(75.0)
    .intensity_factor(0.82)
    .suffer_score(85)
    .start_latitude(45.5017)
    .start_longitude(-73.5673)
    .city("Montreal".to_owned())
    .region("Quebec".to_owned())
    .country("Canada".to_owned())
    .trail_name("Lachine Canal".to_owned())
    .build();

    // Test intelligence engine integration
    let analyzer = ActivityAnalyzer::new();
    let context = ActivityContext {
        location: None,
        recent_activities: None,
    };
    let intelligence = analyzer.analyze_activity(&activity, Some(&context))?;

    // Verify intelligence analysis results
    assert!(!intelligence.summary.is_empty());
    assert!(!intelligence.key_insights.is_empty());

    // Check that performance indicators are calculated
    assert!(
        intelligence
            .performance_indicators
            .relative_effort
            .is_some()
            || intelligence
                .performance_indicators
                .efficiency_score
                .is_some()
    );

    // Test metrics calculation integration
    let calculator = MetricsCalculator::new().with_user_data(
        None,        // No FTP for running
        Some(175.0), // LTHR estimate
        Some(190.0), // Max HR
        Some(55.0),  // Resting HR
        Some(75.0),  // Weight
    );

    let calculated_metrics = calculator.calculate_metrics(&activity)?;

    // Verify calculated metrics
    assert!(calculated_metrics.trimp.is_some());
    assert!(calculated_metrics.running_effectiveness.is_some());
    assert!(calculated_metrics.stride_efficiency.is_some());
    assert!(calculated_metrics.temperature_stress.is_some());

    // Test integration with universal tool execution
    // This simulates how the MCP protocol would trigger intelligence analysis
    let tool_name = "analyze_activity";
    let _ = tool_name;
    let _tool_args = serde_json::json!({
        "activity_id": activity.id(),
        "user_id": user.id.to_string(),
        "include_advanced_metrics": true
    });

    // Note: This would normally be handled by the universal tool executor
    // but we're testing the integration point here
    let context2 = ActivityContext {
        location: None,
        recent_activities: None,
    };
    let analysis_result = analyzer.analyze_activity(&activity, Some(&context2))?;

    // Verify the analysis includes all expected components
    assert!(!analysis_result.summary.is_empty());
    assert!(!analysis_result.key_insights.is_empty());
    assert!(
        analysis_result
            .performance_indicators
            .relative_effort
            .is_some()
            || analysis_result
                .performance_indicators
                .efficiency_score
                .is_some()
    );

    Ok(())
}

/// Test recommendation engine integration with tool handlers
#[tokio::test]
async fn test_recommendation_engine_integration() -> Result<()> {
    let database = create_test_database().await?;

    // Create user first
    let user = User::new(
        "test2@example.com".to_owned(),
        "password_hash".to_owned(),
        Some("Test User 2".to_owned()),
    );
    database.repositories().users.create(&user).await?;

    // Create test user profile
    let fitness_profile = UserFitnessProfile {
        user_id: user.id.to_string(),
        age: Some(25),
        gender: Some("female".to_owned()),
        weight: Some(60.0),
        height: Some(165.0),
        fitness_level: FitnessLevel::Advanced,
        primary_sports: vec!["cycling".to_owned()],
        training_history_months: 36,
        preferences: UserPreferences {
            preferred_units: "metric".to_owned(),
            training_focus: vec!["power".to_owned(), "endurance".to_owned()],
            injury_history: vec![],
            time_availability: TimeAvailability {
                hours_per_week: 10.0,
                preferred_days: vec![
                    "tuesday".to_owned(),
                    "thursday".to_owned(),
                    "saturday".to_owned(),
                    "sunday".to_owned(),
                ],
                preferred_duration_minutes: Some(90),
            },
        },
        seasonal_context: None,
    };

    // Store fitness profile
    let profile_data = serde_json::to_value(&fitness_profile)?;
    database
        .repositories()
        .profiles
        .upsert_profile(user.id, profile_data)
        .await?;

    // Create cycling activity with power data
    let activity = ActivityBuilder::new(
        "cycling_test_001",
        "Threshold Intervals",
        SportType::Ride,
        Utc::now() - chrono::Duration::hours(1),
        4500, // 75 minutes
        "strava",
    )
    .distance_meters(45000.0) // 45km
    .elevation_gain(300.0)
    .average_heart_rate(160)
    .max_heart_rate(180)
    .average_speed(11.11) // 40km/h
    .max_speed(15.28) // 55km/h
    .calories(900)
    .average_power(250)
    .max_power(450)
    .normalized_power(265)
    .ftp(280)
    .average_cadence(90)
    .max_cadence(120)
    .hrv_score(40.0)
    .recovery_heart_rate(30)
    .temperature(22.0)
    .humidity(55.0)
    .average_altitude(200.0)
    .wind_speed(8.0)
    .breathing_rate(28)
    .spo2(97.5)
    .training_stress_score(95.0)
    .intensity_factor(0.89)
    .suffer_score(120)
    .start_latitude(45.5017)
    .start_longitude(-73.5673)
    .city("Montreal".to_owned())
    .region("Quebec".to_owned())
    .country("Canada".to_owned())
    .build();

    // Test recommendation engine
    let analyzer = ActivityAnalyzer::new();
    let context = ActivityContext {
        location: None,
        recent_activities: None,
    };
    let intelligence = analyzer.analyze_activity(&activity, Some(&context))?;

    // Verify analysis is generated
    assert!(!intelligence.summary.is_empty());
    assert!(!intelligence.key_insights.is_empty());
    assert!(
        intelligence
            .performance_indicators
            .relative_effort
            .is_some()
            || intelligence
                .performance_indicators
                .efficiency_score
                .is_some()
    );

    // Check that some insights are generated (content may vary)
    // Note: The specific content of insights depends on the implementation
    // of the insight generation algorithms, so we just verify they exist
    assert!(
        !intelligence.key_insights.is_empty(),
        "Should generate insights"
    );

    Ok(())
}

/// Test goal tracking integration with universal tool handlers
#[tokio::test]
async fn test_goal_tracking_integration() -> Result<()> {
    let database = create_test_database().await?;

    // Create user first
    let user = User::new(
        "test3@example.com".to_owned(),
        "password_hash".to_owned(),
        Some("Test User 3".to_owned()),
    );
    database.repositories().users.create(&user).await?;

    // Create a test goal
    let goal_data = serde_json::json!({
        "type": "distance",
        "target_value": 100_000.0, // 100km total distance
        "current_value": 0.0,
        "time_period": "monthly",
        "sport_type": "run",
        "created_date": Utc::now().to_rfc3339(),
        "target_date": (Utc::now() + chrono::Duration::days(30)).to_rfc3339()
    });

    let repos = database.repositories();
    let goal_id = repos.profiles.create_goal(user.id, goal_data).await?;

    // Create activities that contribute to the goal
    let activities = [
        create_test_activity("run_001", &SportType::Run, 5000.0), // 5km
        create_test_activity("run_002", &SportType::Run, 8000.0), // 8km
        create_test_activity("run_003", &SportType::Run, 10000.0), // 10km
    ];

    // Calculate total distance from activities
    let total_distance: f64 = activities
        .iter()
        .filter_map(Activity::distance_meters)
        .sum();

    // Update goal progress
    repos
        .profiles
        .update_goal_progress(&goal_id, user.id, total_distance)
        .await?;

    // Verify goal was updated correctly
    let goals = repos.profiles.get_goals(user.id).await?;
    assert_eq!(goals.len(), 1);

    let updated_goal = &goals[0];
    assert!(
        (updated_goal["current_value"].as_f64().unwrap() - total_distance).abs() < f64::EPSILON
    );
    assert!((updated_goal["progress_percentage"].as_f64().unwrap() - 23.0).abs() < f64::EPSILON); // 23% of 100km

    // Test goal integration with activity analysis
    let _fitness_profile = UserFitnessProfile {
        user_id: user.id.to_string(),
        age: Some(35),
        gender: Some("male".to_owned()),
        weight: Some(70.0),
        height: Some(175.0),
        fitness_level: FitnessLevel::Intermediate,
        primary_sports: vec!["running".to_owned()],
        training_history_months: 18,
        preferences: UserPreferences {
            preferred_units: "metric".to_owned(),
            training_focus: vec!["distance".to_owned()],
            injury_history: vec![],
            time_availability: TimeAvailability {
                hours_per_week: 5.0,
                preferred_days: vec![
                    "monday".to_owned(),
                    "wednesday".to_owned(),
                    "friday".to_owned(),
                ],
                preferred_duration_minutes: Some(45),
            },
        },
        seasonal_context: None,
    };

    let analyzer = ActivityAnalyzer::new();
    let context = ActivityContext {
        location: None,
        recent_activities: None,
    };
    let intelligence = analyzer.analyze_activity(&activities[0], Some(&context))?;

    // Verify analysis is generated and includes relevant insights
    assert!(!intelligence.summary.is_empty());
    assert!(!intelligence.key_insights.is_empty());
    assert!(
        intelligence
            .performance_indicators
            .relative_effort
            .is_some()
            || intelligence
                .performance_indicators
                .efficiency_score
                .is_some()
    );

    Ok(())
}

/// A goal created by `set_goal` is findable by `track_progress`.
///
/// `track_progress` looks a goal up by `goal_data.goal_id`, so the id the
/// creation path generates has to live inside the stored JSON. When it did
/// not, the lookup key was absent from every row on every backend and the tool
/// answered "Goal not found" for goals it had just created — the whole feature
/// was unreachable. This drives both halves through the production path: the
/// `set_goal` tool creates, then the `track_progress` tool reads the goal's own
/// type, target and timeframe back out of the store, then the repository
/// updates progress against the same id.
#[tokio::test]
async fn test_track_progress_finds_a_goal_created_by_set_goal() -> Result<()> {
    // The activity fetch that follows the goal lookup builds a real Strava
    // provider. Point it at a closed local port so the fetch fails at once and
    // the tool reports progress over zero activities, instead of the suite
    // reaching out to strava.com.
    env::set_var("PIERRE_STRAVA_API_BASE_URL", "http://127.0.0.1:1/api/v3");
    common::init_server_config();
    common::init_test_http_clients();

    let resources = common::create_test_server_resources().await?;
    let database = resources.coach.database.clone();
    let (user_id, _user, tenant_id) =
        common::create_test_user_with_plan(&database, "goal-tracker@example.com", "starter")
            .await?;

    // The dispatch chokepoint refuses provider-requiring tools for an athlete
    // with no data source at all, which would short-circuit before the goal
    // lookup this test is about.
    let now = Utc::now();
    database
        .repositories()
        .oauth_tokens
        .upsert_token(&UserOAuthToken {
            id: Uuid::new_v4().to_string(),
            user_id,
            tenant_id: tenant_id.to_string(),
            provider: "strava".to_owned(),
            access_token: "test_access_token".to_owned(),
            refresh_token: Some("test_refresh_token".to_owned()),
            token_type: "Bearer".to_owned(),
            expires_at: Some(now + chrono::Duration::hours(6)),
            scope: Some("activity:read_all".to_owned()),
            provider_user_id: None,
            oauth_app_client_id: None,
            created_at: now,
            updated_at: now,
        })
        .await?;

    let executor = UniversalToolExecutor::new(resources).with_scopes(OAuthScope::self_grant());

    let created = executor
        .execute_tool(UniversalRequest {
            tool_name: "set_goal".to_owned(),
            parameters: json!({
                "goal_type": "distance",
                "target_value": 100.0,
                "timeframe": "month",
                "title": "100 km this month"
            }),
            user_id: user_id.to_string(),
            protocol: "test".to_owned(),
            tenant_id: Some(tenant_id.to_string()),
            progress_token: None,
            cancellation_token: None,
            progress_reporter: None,
        })
        .await?;
    assert!(
        created.success,
        "set_goal must succeed: {:?}",
        created.error
    );
    let goal_id = created.result.as_ref().unwrap()["goal_id"]
        .as_str()
        .expect("set_goal returns the created goal's id")
        .to_owned();

    let tracked = executor
        .execute_tool(UniversalRequest {
            tool_name: "track_progress".to_owned(),
            parameters: json!({ "goal_id": goal_id, "provider": "strava" }),
            user_id: user_id.to_string(),
            protocol: "test".to_owned(),
            tenant_id: Some(tenant_id.to_string()),
            progress_token: None,
            cancellation_token: None,
            progress_reporter: None,
        })
        .await?;
    assert!(
        tracked.success,
        "track_progress must find the goal set_goal just created: {:?}",
        tracked.error
    );

    // Every field below is read back out of the stored goal, so they are the
    // proof the lookup hit the right row rather than a default-shaped miss.
    let progress = tracked.result.as_ref().unwrap();
    assert_eq!(progress["goal_id"].as_str(), Some(goal_id.as_str()));
    assert_eq!(progress["goal_type"].as_str(), Some("distance"));
    assert_eq!(progress["target_value"].as_f64(), Some(100.0));
    assert_eq!(progress["timeframe"].as_str(), Some("month"));
    assert_eq!(progress["unit"].as_str(), Some("km"));
    assert_eq!(progress["days_remaining"].as_u64(), Some(30));
    assert_eq!(progress["summary"]["total_activities"].as_u64(), Some(0));

    // The same id addresses the row for writes: progress recorded against it
    // lands in the stored goal with the percentage derived from its target.
    let repos = database.repositories();
    repos
        .profiles
        .update_goal_progress(&goal_id, user_id, 25.0)
        .await?;

    let goals = repos.profiles.get_goals(user_id).await?;
    assert_eq!(goals.len(), 1);
    assert_eq!(goals[0]["goal_id"].as_str(), Some(goal_id.as_str()));
    assert_eq!(goals[0]["current_value"].as_f64(), Some(25.0));
    assert_eq!(goals[0]["progress_percentage"].as_f64(), Some(25.0));

    Ok(())
}

/// The exact backfill statement each backend ships, so the test exercises the
/// migration rather than a paraphrase that could drift from it.
const SQLITE_GOAL_ID_BACKFILL_SQL: &str =
    include_str!("../../../migrations/20260831000002_goal_data_goal_id_backfill.sql");
#[cfg(feature = "postgresql")]
const POSTGRES_GOAL_ID_BACKFILL_SQL: &str =
    include_str!("../../../migrations_pg/20260831000002_goal_data_goal_id_backfill.sql");

/// Insert a goal row carrying exactly `goal_data`, bypassing `create_goal` —
/// the point is to plant the shape rows written before the id was embedded had.
async fn plant_goal_row(db: &Database, id: &str, user_id: Uuid, goal_data: &serde_json::Value) {
    const SQL: &str = "INSERT INTO goals (id, user_id, goal_data, created_at, updated_at) \
                       VALUES ($1, $2, $3, $4, $4)";
    let now = Utc::now();
    match db {
        Database::SQLite(d) => {
            sqlx::query(SQL)
                .bind(id)
                .bind(user_id.to_string())
                .bind(serde_json::to_string(goal_data).unwrap())
                .bind(now.to_rfc3339())
                .execute(d.pool())
                .await
                .unwrap();
        }
        #[cfg(feature = "postgresql")]
        Database::PostgreSQL(d) => {
            sqlx::query(SQL)
                .bind(id)
                .bind(user_id)
                .bind(goal_data)
                .bind(now)
                .execute(d.pool())
                .await
                .unwrap();
        }
    }
}

/// Run the backfill statement the backend ships.
async fn run_goal_id_backfill(db: &Database) {
    match db {
        Database::SQLite(d) => {
            sqlx::query(SQLITE_GOAL_ID_BACKFILL_SQL)
                .execute(d.pool())
                .await
                .unwrap();
        }
        #[cfg(feature = "postgresql")]
        Database::PostgreSQL(d) => {
            sqlx::query(POSTGRES_GOAL_ID_BACKFILL_SQL)
                .execute(d.pool())
                .await
                .unwrap();
        }
    }
}

/// Read one goal row's stored JSON by row id.
async fn read_goal_row(db: &Database, id: &str) -> serde_json::Value {
    const SQL: &str = "SELECT goal_data FROM goals WHERE id = $1";
    match db {
        Database::SQLite(d) => {
            let row = sqlx::query(SQL).bind(id).fetch_one(d.pool()).await.unwrap();
            let stored: String = row.get("goal_data");
            serde_json::from_str(&stored).unwrap()
        }
        #[cfg(feature = "postgresql")]
        Database::PostgreSQL(d) => {
            let row = sqlx::query(SQL).bind(id).fetch_one(d.pool()).await.unwrap();
            row.get("goal_data")
        }
    }
}

/// The shipped backfill migration stamps `goal_data.goal_id` on rows that lack
/// it and leaves rows that already carry one alone.
///
/// Goals written before the id was embedded are unreachable to
/// `track_progress`, which finds a goal by that key; deployed databases hold
/// those rows, so the fix is only complete once they carry it too. The
/// untouched row doubles as the idempotence check — re-running the migration
/// must not rewrite a goal that already identifies itself.
#[tokio::test]
async fn test_goal_id_backfill_migration() -> Result<()> {
    let database = create_test_database().await?;
    let (user_id, _user) = create_test_user(&database).await?;

    let legacy_id = Uuid::new_v4().to_string();
    plant_goal_row(
        &database,
        &legacy_id,
        user_id,
        &json!({
            "goal_type": "distance",
            "target_value": 100.0,
            "timeframe": "month",
            "title": "Written before the id was embedded"
        }),
    )
    .await;

    let stamped_id = Uuid::new_v4().to_string();
    plant_goal_row(
        &database,
        &stamped_id,
        user_id,
        &json!({
            "goal_id": "already-identified",
            "goal_type": "frequency",
            "target_value": 12.0,
            "timeframe": "month",
            "title": "Written with the id embedded"
        }),
    )
    .await;

    run_goal_id_backfill(&database).await;

    let backfilled = read_goal_row(&database, &legacy_id).await;
    assert_eq!(
        backfilled["goal_id"].as_str(),
        Some(legacy_id.as_str()),
        "the backfill copies the row id into the stored JSON"
    );
    assert_eq!(
        backfilled["title"].as_str(),
        Some("Written before the id was embedded"),
        "the backfill leaves the rest of the goal untouched"
    );

    let untouched = read_goal_row(&database, &stamped_id).await;
    assert_eq!(
        untouched["goal_id"].as_str(),
        Some("already-identified"),
        "a goal that already identifies itself is left alone, so re-running is a no-op"
    );

    Ok(())
}

/// Helper function to create test activities
fn create_test_activity(id: &str, sport_type: &SportType, distance: f64) -> Activity {
    let mut builder = ActivityBuilder::new(
        id,
        format!("Test {sport_type:?}"),
        sport_type.clone(),
        Utc::now() - chrono::Duration::hours(1),
        1800, // 30 minutes
        "test",
    )
    .distance_meters(distance)
    .elevation_gain(50.0)
    .average_heart_rate(150)
    .max_heart_rate(170)
    .average_speed(distance / 1800.0) // Calculate speed
    .max_speed(distance / 1500.0)
    .calories(300)
    .start_latitude(45.5017)
    .start_longitude(-73.5673)
    .city("Montreal".to_owned())
    .region("Quebec".to_owned())
    .country("Canada".to_owned());

    if *sport_type == SportType::Run {
        builder = builder.steps(6000);
    }

    builder.build()
}
