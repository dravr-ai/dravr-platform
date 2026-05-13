// ABOUTME: Synthetic activity seeder for Pierre MCP Server testing without OAuth
// ABOUTME: Generates 100+ diverse activities (nordic ski, MTB, trail run, etc.) for any user
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Synthetic activity seeder for Pierre MCP Server.
//!
//! This binary populates the database with diverse synthetic activities for testing
//! without requiring Strava or other OAuth providers.
//!
//! Usage:
//! ```bash
//! # Seed activities for the default test user (user@example.com)
//! pierre-cli seed synthetic-activities
//!
//! # Seed for a specific user
//! pierre-cli seed synthetic-activities --email alice@example.com
//!
//! # Generate more activities (default: 100)
//! pierre-cli seed synthetic-activities --count 200
//!
//! # Spread over more days (default: 90)
//! pierre-cli seed synthetic-activities --days 180
//!
//! # Reset activities before seeding
//! pierre-cli seed synthetic-activities --reset
//! ```

use chrono::{DateTime, Duration, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_database::seed_models::{SeedProviderConnection, SeedSyntheticActivity};
use pierre_database::RepositoryRegistry;
use rand::prelude::IndexedRandom;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::info;
use uuid::Uuid;

/// CLI arguments for the synthetic activities seeder.
#[derive(clap::Args)]
pub struct SeedArgs {
    /// User email to seed activities for (default: user@example.com)
    #[arg(long, default_value = "user@example.com")]
    pub email: String,

    /// Number of activities to generate
    #[arg(long, default_value = "100")]
    pub count: u32,

    /// Number of days to spread activities over
    #[arg(long, default_value = "90")]
    pub days: u32,

    /// Reset synthetic activities before seeding
    #[arg(long)]
    pub reset: bool,

    /// Random seed for reproducible data (optional)
    #[arg(long)]
    pub seed: Option<u64>,
}

/// Sport type configuration for activity generation
struct SportConfig {
    sport_type: &'static str,
    display_name: &'static str,
    /// Weight for random selection (higher = more common)
    weight: u32,
    /// Duration range in seconds (min, max)
    duration_range: (u64, u64),
    /// Distance range in meters (min, max), None for non-distance activities
    distance_range: Option<(f64, f64)>,
    /// Elevation gain range in meters (min, max), None for flat activities
    elevation_range: Option<(f64, f64)>,
    /// Average heart rate range
    heart_rate_range: (u32, u32),
    /// Activity name templates
    names: &'static [&'static str],
}

/// Get all sport configurations with realistic parameters
fn get_sport_configs() -> Vec<SportConfig> {
    vec![
        // Running activities (most common)
        SportConfig {
            sport_type: "run",
            display_name: "Run",
            weight: 25,
            duration_range: (1200, 7200),            // 20 min - 2 hours
            distance_range: Some((3000.0, 25000.0)), // 3km - 25km
            elevation_range: Some((20.0, 300.0)),
            heart_rate_range: (140, 175),
            names: &[
                "Morning Run",
                "Easy Run",
                "Tempo Run",
                "Long Run",
                "Recovery Run",
                "Interval Session",
            ],
        },
        SportConfig {
            sport_type: "trail_run",
            display_name: "Trail Run",
            weight: 8,
            duration_range: (2400, 10800), // 40 min - 3 hours
            distance_range: Some((5000.0, 30000.0)),
            elevation_range: Some((200.0, 1500.0)),
            heart_rate_range: (145, 180),
            names: &[
                "Trail Adventure",
                "Mountain Trail",
                "Forest Run",
                "Technical Trail",
                "Ridge Run",
            ],
        },
        // Cycling activities
        SportConfig {
            sport_type: "ride",
            display_name: "Ride",
            weight: 20,
            duration_range: (1800, 18000), // 30 min - 5 hours
            distance_range: Some((15_000.0, 150_000.0)),
            elevation_range: Some((100.0, 2000.0)),
            heart_rate_range: (130, 170),
            names: &[
                "Morning Ride",
                "Endurance Ride",
                "Tempo Ride",
                "Group Ride",
                "Solo Spin",
            ],
        },
        SportConfig {
            sport_type: "mountain_bike_ride",
            display_name: "Mountain Bike",
            weight: 8,
            duration_range: (2400, 14400),
            distance_range: Some((10000.0, 60000.0)),
            elevation_range: Some((300.0, 2500.0)),
            heart_rate_range: (140, 180),
            names: &[
                "MTB Session",
                "Single Track",
                "Trail Ride",
                "Downhill Fun",
                "Technical Climb",
            ],
        },
        SportConfig {
            sport_type: "gravel_ride",
            display_name: "Gravel Ride",
            weight: 5,
            duration_range: (3600, 21600),
            distance_range: Some((30_000.0, 200_000.0)),
            elevation_range: Some((200.0, 3000.0)),
            heart_rate_range: (135, 170),
            names: &[
                "Gravel Adventure",
                "Mixed Surface",
                "Backroads Explorer",
                "Gravel Century",
            ],
        },
        SportConfig {
            sport_type: "virtual_ride",
            display_name: "Virtual Ride",
            weight: 6,
            duration_range: (1800, 5400),
            distance_range: Some((15000.0, 60000.0)),
            elevation_range: Some((100.0, 1000.0)),
            heart_rate_range: (135, 175),
            names: &[
                "Zwift Session",
                "Indoor Training",
                "Trainer Workout",
                "Virtual Race",
            ],
        },
        // Winter sports
        SportConfig {
            sport_type: "nordic_ski",
            display_name: "Nordic Ski",
            weight: 6,
            duration_range: (2400, 10800),
            distance_range: Some((5000.0, 50000.0)),
            elevation_range: Some((100.0, 800.0)),
            heart_rate_range: (140, 180),
            names: &[
                "Classic Ski",
                "Skate Ski",
                "Nordic Tour",
                "Ski Marathon Training",
                "Trail Ski",
            ],
        },
        SportConfig {
            sport_type: "backcountry_ski",
            display_name: "Backcountry Ski",
            weight: 3,
            duration_range: (3600, 18000),
            distance_range: Some((3000.0, 20000.0)),
            elevation_range: Some((500.0, 2500.0)),
            heart_rate_range: (130, 170),
            names: &[
                "Backcountry Tour",
                "Skin Up",
                "Powder Day",
                "Alpine Tour",
                "Summit Push",
            ],
        },
        SportConfig {
            sport_type: "alpine_ski",
            display_name: "Alpine Ski",
            weight: 4,
            duration_range: (3600, 21600),
            distance_range: Some((10000.0, 50000.0)),
            elevation_range: Some((1000.0, 5000.0)),
            heart_rate_range: (100, 140),
            names: &[
                "Ski Day",
                "Resort Laps",
                "Powder Hunting",
                "Groomer Day",
                "All Mountain",
            ],
        },
        SportConfig {
            sport_type: "snowshoe",
            display_name: "Snowshoe",
            weight: 2,
            duration_range: (2400, 10800),
            distance_range: Some((3000.0, 15000.0)),
            elevation_range: Some((100.0, 800.0)),
            heart_rate_range: (120, 155),
            names: &[
                "Snowshoe Hike",
                "Winter Trail",
                "Snow Trek",
                "Backcountry Snowshoe",
            ],
        },
        // Swimming
        SportConfig {
            sport_type: "swim",
            display_name: "Swim",
            weight: 6,
            duration_range: (1200, 5400),
            distance_range: Some((500.0, 5000.0)),
            elevation_range: None,
            heart_rate_range: (120, 160),
            names: &[
                "Pool Swim",
                "Lap Session",
                "Endurance Swim",
                "Drill Work",
                "Speed Set",
            ],
        },
        SportConfig {
            sport_type: "open_water_swim",
            display_name: "Open Water Swim",
            weight: 2,
            duration_range: (1800, 7200),
            distance_range: Some((1000.0, 10000.0)),
            elevation_range: None,
            heart_rate_range: (130, 165),
            names: &[
                "Lake Swim",
                "Ocean Swim",
                "River Crossing",
                "Triathlon Practice",
            ],
        },
        // Walking and hiking
        SportConfig {
            sport_type: "walk",
            display_name: "Walk",
            weight: 8,
            duration_range: (1200, 7200),
            distance_range: Some((2000.0, 15000.0)),
            elevation_range: Some((10.0, 200.0)),
            heart_rate_range: (90, 120),
            names: &[
                "Morning Walk",
                "Lunch Walk",
                "Evening Stroll",
                "Active Recovery",
            ],
        },
        SportConfig {
            sport_type: "hike",
            display_name: "Hike",
            weight: 6,
            duration_range: (3600, 28800),
            distance_range: Some((5000.0, 30000.0)),
            elevation_range: Some((200.0, 2000.0)),
            heart_rate_range: (110, 150),
            names: &[
                "Day Hike",
                "Summit Hike",
                "Ridge Walk",
                "Canyon Hike",
                "Peak Bagging",
            ],
        },
        // Strength and indoor
        SportConfig {
            sport_type: "weight_training",
            display_name: "Weight Training",
            weight: 8,
            duration_range: (1800, 5400),
            distance_range: None,
            elevation_range: None,
            heart_rate_range: (100, 145),
            names: &[
                "Strength Session",
                "Leg Day",
                "Upper Body",
                "Full Body",
                "Core Work",
            ],
        },
        SportConfig {
            sport_type: "yoga",
            display_name: "Yoga",
            weight: 4,
            duration_range: (1800, 5400),
            distance_range: None,
            elevation_range: None,
            heart_rate_range: (70, 110),
            names: &[
                "Morning Yoga",
                "Vinyasa Flow",
                "Recovery Yoga",
                "Power Yoga",
                "Stretch Session",
            ],
        },
        SportConfig {
            sport_type: "workout",
            display_name: "Workout",
            weight: 5,
            duration_range: (1200, 3600),
            distance_range: None,
            elevation_range: None,
            heart_rate_range: (130, 170),
            names: &[
                "HIIT Session",
                "CrossFit WOD",
                "Circuit Training",
                "Cardio Blast",
                "Functional Fitness",
            ],
        },
        // Water sports
        SportConfig {
            sport_type: "rowing",
            display_name: "Rowing",
            weight: 3,
            duration_range: (1200, 5400),
            distance_range: Some((2000.0, 20000.0)),
            elevation_range: None,
            heart_rate_range: (140, 175),
            names: &[
                "Erg Session",
                "On-Water Row",
                "2K Test",
                "Steady State",
                "Intervals",
            ],
        },
        SportConfig {
            sport_type: "kayaking",
            display_name: "Kayaking",
            weight: 2,
            duration_range: (2400, 14400),
            distance_range: Some((5000.0, 40000.0)),
            elevation_range: None,
            heart_rate_range: (110, 150),
            names: &[
                "Paddle Session",
                "River Run",
                "Lake Tour",
                "Sea Kayak",
                "Whitewater",
            ],
        },
        SportConfig {
            sport_type: "stand_up_paddling",
            display_name: "SUP",
            weight: 2,
            duration_range: (1800, 7200),
            distance_range: Some((2000.0, 15000.0)),
            elevation_range: None,
            heart_rate_range: (100, 140),
            names: &[
                "SUP Session",
                "Paddle Tour",
                "SUP Yoga",
                "Downwind Run",
                "Flatwater Cruise",
            ],
        },
    ]
}

/// Build weighted selection vector from sport configs
fn build_weighted_sports(configs: &[SportConfig]) -> Vec<usize> {
    let mut weighted = Vec::new();
    for (index, config) in configs.iter().enumerate() {
        for _ in 0..config.weight {
            weighted.push(index);
        }
    }
    weighted
}

/// Generate diverse synthetic activities for a user across many sport types without needing OAuth.
///
/// # Errors
///
/// Returns an error if the target user is not found, if the user has no tenant, or if
/// any repository operation fails while inserting activities or the provider connection.
pub async fn run(args: SeedArgs, repos: &RepositoryRegistry) -> AppResult<()> {
    log_run_header(&args);

    let (user_id, tenant_id) = resolve_user_tenant(repos, &args.email).await?;
    if args.reset {
        info!("Resetting synthetic activities...");
        repos.seeder.seed_delete_synthetic_by_user(user_id).await?;
    }

    let mut rng = init_rng(args.seed);
    let sport_configs = get_sport_configs();
    let weighted_sports = build_weighted_sports(&sport_configs);

    let counts = generate_activities(
        &mut rng,
        GenerateActivitiesParams {
            repos,
            sport_configs: &sport_configs,
            weighted_sports: &weighted_sports,
            user_id,
            tenant_id,
            count: args.count,
            days: args.days,
        },
    )
    .await?;

    register_provider_connection(repos, user_id, tenant_id).await?;
    log_activity_summary(args.count, &counts);
    Ok(())
}

fn log_run_header(args: &SeedArgs) {
    info!(
        "Pierre Synthetic Activity Seeder: email={}, count={}, days={}",
        args.email, args.count, args.days
    );
}

async fn resolve_user_tenant(repos: &RepositoryRegistry, email: &str) -> AppResult<(Uuid, Uuid)> {
    let user = repos
        .seeder
        .seed_find_user_by_email(email)
        .await?
        .ok_or_else(|| {
            AppError::config(format!(
                "User not found: {email}. Run ./scripts/complete-user-workflow.sh first."
            ))
        })?;

    let tenant_id_str = repos
        .seeder
        .seed_get_user_tenant(user.id)
        .await?
        .ok_or_else(|| AppError::config(format!("User {email} has no tenant_id")))?;
    let tenant_id = Uuid::parse_str(&tenant_id_str)
        .map_err(|e| AppError::config(format!("Invalid tenant_id UUID: {e}")))?;

    info!("   User ID: {}", user.id);
    info!("   Tenant ID: {tenant_id}");
    Ok((user.id, tenant_id))
}

fn init_rng(seed: Option<u64>) -> StdRng {
    let resolved = seed.unwrap_or_else(|| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(12345, |d| d.as_secs())
    });
    info!("   Random seed: {resolved}");
    StdRng::seed_from_u64(resolved)
}

/// Bundled inputs for [`generate_activities`] — bundles the parameters that
/// drive synthetic activity construction so the entry point doesn't need a
/// nine-arg positional signature.
struct GenerateActivitiesParams<'a> {
    repos: &'a RepositoryRegistry,
    sport_configs: &'a [SportConfig],
    weighted_sports: &'a [usize],
    user_id: Uuid,
    tenant_id: Uuid,
    count: u32,
    days: u32,
}

async fn generate_activities(
    rng: &mut StdRng,
    params: GenerateActivitiesParams<'_>,
) -> AppResult<HashMap<&'static str, u32>> {
    info!(
        "Generating {} activities over {} days...",
        params.count, params.days
    );
    let now = Utc::now();
    let mut activities_by_type: HashMap<&str, u32> = HashMap::new();

    for i in 0..params.count {
        let sport_index = *params.weighted_sports.choose(rng).unwrap_or(&0);
        let sport = &params.sport_configs[sport_index];
        let activity = build_activity(
            rng,
            sport,
            now,
            params.user_id,
            params.tenant_id,
            i,
            params.days,
        );
        params
            .repos
            .seeder
            .seed_insert_synthetic_activity(&activity)
            .await?;
        *activities_by_type.entry(sport.sport_type).or_insert(0) += 1;
    }
    Ok(activities_by_type)
}

#[allow(clippy::cast_possible_wrap)]
fn build_activity(
    rng: &mut StdRng,
    sport: &SportConfig,
    now: DateTime<Utc>,
    user_id: Uuid,
    tenant_id: Uuid,
    index: u32,
    days: u32,
) -> SeedSyntheticActivity {
    let days_ago = rng.random_range(0..days);
    let hour = rng.random_range(5..21); // 5 AM to 9 PM
    let minute = rng.random_range(0..60);
    let start_date =
        now - Duration::days(i64::from(days_ago)) - Duration::hours(24 - i64::from(hour))
            + Duration::minutes(i64::from(minute));

    let duration = rng.random_range(sport.duration_range.0..=sport.duration_range.1);
    let distance = sport
        .distance_range
        .map(|(min, max)| rng.random_range(min..=max));
    let elevation = sport
        .elevation_range
        .map(|(min, max)| rng.random_range(min..=max));
    let avg_hr = rng.random_range(sport.heart_rate_range.0..=sport.heart_rate_range.1);
    let max_hr = avg_hr + rng.random_range(10..30);
    let calories = Some(rng.random_range(200..1200));

    let avg_speed = distance.map(|d| d / duration as f64);
    let max_speed = avg_speed.map(|s| s * rng.random_range(1.15..1.4));

    let name = format!(
        "{} #{}",
        sport.names.choose(rng).unwrap_or(&sport.display_name),
        index + 1
    );

    SeedSyntheticActivity {
        id: Uuid::new_v4(),
        user_id,
        tenant_id,
        name,
        sport_type: sport.sport_type.to_owned(),
        start_date,
        duration_seconds: duration as i64, // max ~86400 seconds
        distance_meters: distance,
        elevation_gain: elevation,
        average_heart_rate: avg_hr as i32, // heart rate 50-220 bpm
        max_heart_rate: max_hr as i32,
        average_speed: avg_speed,
        max_speed,
        calories,
        city: "Montreal".to_owned(),
        region: "Quebec".to_owned(),
        country: "Canada".to_owned(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

async fn register_provider_connection(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    tenant_id: Uuid,
) -> AppResult<()> {
    let connection = SeedProviderConnection {
        id: Uuid::new_v4(),
        user_id,
        tenant_id,
        provider: "synthetic".to_owned(),
        connection_type: "synthetic".to_owned(),
        connected_at: Utc::now(),
        metadata: r#"{"source": "seed-synthetic-activities"}"#.to_owned(),
    };
    repos
        .seeder
        .seed_upsert_provider_connection(&connection)
        .await?;
    info!("Registered synthetic provider connection");
    Ok(())
}

fn log_activity_summary(total: u32, activities_by_type: &HashMap<&str, u32>) {
    info!("Created {total} synthetic activities");
    let mut sorted_types: Vec<_> = activities_by_type.iter().collect();
    sorted_types.sort_by(|a, b| b.1.cmp(a.1));
    for (sport_type, count) in sorted_types {
        info!("   {sport_type}: {count}");
    }
}
