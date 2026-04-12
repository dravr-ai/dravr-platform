// ABOUTME: Demo data seeder for Pierre MCP Server dashboard testing
// ABOUTME: Generates realistic time-series data for users, API keys, and usage analytics
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Demo data seeder for Pierre MCP Server.
//!
//! This binary populates the database with realistic demo data for testing
//! the dashboard, analytics, and user management features.
//!
//! Usage:
//! ```bash
//! # Seed with default settings (assigns data to first admin user)
//! pierre-cli seed demo-data
//!
//! # Seed with specific admin email
//! pierre-cli seed demo-data --admin-email admin@example.com
//!
//! # Reset database before seeding
//! pierre-cli seed demo-data --reset
//! ```

use bcrypt::{hash, DEFAULT_COST};
use chrono::{DateTime, Datelike, Duration, Timelike, Utc, Weekday};
use pierre_core::errors::{AppError, AppResult};
use pierre_database::repositories::SeedTable;
use pierre_database::seed_models::{
    SeedA2AClient, SeedA2AUsage, SeedApiKey, SeedApiKeyUsage, SeedDemoUser, SeedTenant,
};
use pierre_database::RepositoryRegistry;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use tracing::info;
use uuid::Uuid;

/// Default password for all demo users - allows login for testing.
/// Password: `DemoUser123!`
const DEMO_USER_PASSWORD: &str = "DemoUser123!";

/// CLI arguments for the demo data seeder.
#[derive(clap::Args)]
pub struct SeedArgs {
    /// Admin email to assign primary data to (uses first admin if not specified)
    #[arg(long)]
    pub admin_email: Option<String>,

    /// Reset usage data before seeding (keeps users and API keys)
    #[arg(long)]
    pub reset: bool,

    /// Number of days of historical data to generate
    #[arg(long, default_value = "30")]
    pub days: u32,
}

/// Demo user configuration
struct DemoUser {
    email: &'static str,
    display_name: &'static str,
    tier: &'static str,
    status: &'static str,
    /// Optional custom password (defaults to `DEMO_USER_PASSWORD` if None)
    password: Option<&'static str>,
}

/// Demo API key configuration
struct DemoApiKey {
    name: &'static str,
    description: &'static str,
    tier: &'static str,
    rate_limit: Option<i32>,
}

/// Demo A2A client configuration
struct DemoA2AClient {
    name: &'static str,
    description: &'static str,
    capabilities: &'static str,
}

/// Tool names for usage generation
const TOOLS: &[&str] = &[
    "get_activities",
    "analyze_workout",
    "get_profile",
    "sync_data",
    "generate_insights",
    "get_goals",
    "update_preferences",
    "get_recommendations",
    "get_heart_rate",
    "get_power_zones",
    "calculate_ftp",
    "predict_race",
    "get_training_load",
    "analyze_sleep",
    "get_nutrition_log",
    "sync_garmin",
    "sync_strava",
    "export_gpx",
    "import_tcx",
    "get_leaderboard",
];

/// A2A tool names
const A2A_TOOLS: &[&str] = &[
    "send_message",
    "analyze_activity",
    "get_recommendations",
    "sync_data",
    "export_report",
];

/// Get demo user definitions (part 1) - extracted for function length
/// Includes visual testing users at the start for easy identification
fn get_demo_users_part1() -> Vec<DemoUser> {
    vec![
        // Visual Testing Users (created first for testing)
        DemoUser {
            email: "webtest@pierre.dev",
            display_name: "Web Test User",
            tier: "professional",
            status: "active",
            password: Some("WebTest123!"),
        },
        DemoUser {
            email: "mobiletest@pierre.dev",
            display_name: "Mobile Test User",
            tier: "professional",
            status: "active",
            password: Some("MobileTest1234"),
        },
        // Regular demo users
        DemoUser {
            email: "alice@acme.com",
            display_name: "Alice Johnson",
            tier: "professional",
            status: "active",
            password: None,
        },
        DemoUser {
            email: "bob@startup.io",
            display_name: "Bob Smith",
            tier: "starter",
            status: "active",
            password: None,
        },
        DemoUser {
            email: "charlie@enterprise.co",
            display_name: "Charlie Brown",
            tier: "enterprise",
            status: "active",
            password: None,
        },
        DemoUser {
            email: "diana@freelance.dev",
            display_name: "Diana Prince",
            tier: "starter",
            status: "active",
            password: None,
        },
        DemoUser {
            email: "eve@pending.com",
            display_name: "Eve Wilson",
            tier: "starter",
            status: "pending",
            password: None,
        },
        DemoUser {
            email: "frank@pending.org",
            display_name: "Frank Miller",
            tier: "starter",
            status: "pending",
            password: None,
        },
        DemoUser {
            email: "grace@suspended.net",
            display_name: "Grace Lee",
            tier: "professional",
            status: "suspended",
            password: None,
        },
        DemoUser {
            email: "henry@techcorp.io",
            display_name: "Henry Zhang",
            tier: "enterprise",
            status: "active",
            password: None,
        },
        DemoUser {
            email: "isabella@fitness.app",
            display_name: "Isabella Martinez",
            tier: "professional",
            status: "active",
            password: None,
        },
        DemoUser {
            email: "james@healthtrack.com",
            display_name: "James OBrien",
            tier: "starter",
            status: "active",
            password: None,
        },
    ]
}

/// Get demo user definitions (part 2) - extracted for function length
fn get_demo_users_part2() -> Vec<DemoUser> {
    vec![
        DemoUser {
            email: "kate@runclub.org",
            display_name: "Kate Williams",
            tier: "starter",
            status: "active",
            password: None,
        },
        DemoUser {
            email: "leo@gym.pro",
            display_name: "Leo Thompson",
            tier: "professional",
            status: "active",
            password: None,
        },
        DemoUser {
            email: "maria@cycling.team",
            display_name: "Maria Garcia",
            tier: "enterprise",
            status: "active",
            password: None,
        },
        DemoUser {
            email: "noah@swim.club",
            display_name: "Noah Davis",
            tier: "starter",
            status: "active",
            password: None,
        },
        DemoUser {
            email: "olivia@yoga.studio",
            display_name: "Olivia Taylor",
            tier: "professional",
            status: "active",
            password: None,
        },
        DemoUser {
            email: "peter@triathlon.org",
            display_name: "Peter Anderson",
            tier: "enterprise",
            status: "active",
            password: None,
        },
        DemoUser {
            email: "quinn@pending.io",
            display_name: "Quinn Roberts",
            tier: "starter",
            status: "pending",
            password: None,
        },
        DemoUser {
            email: "rachel@marathon.run",
            display_name: "Rachel Clark",
            tier: "professional",
            status: "active",
            password: None,
        },
        DemoUser {
            email: "sam@crossfit.box",
            display_name: "Sam Wilson",
            tier: "starter",
            status: "active",
            password: None,
        },
        DemoUser {
            email: "tina@pilates.center",
            display_name: "Tina Brown",
            tier: "professional",
            status: "active",
            password: None,
        },
    ]
}

/// Get combined demo users
fn get_demo_users() -> Vec<DemoUser> {
    let mut users = get_demo_users_part1();
    users.extend(get_demo_users_part2());
    users
}

/// Get demo API key definitions - extracted for function length (part 1)
fn get_demo_api_keys_part1() -> Vec<DemoApiKey> {
    vec![
        DemoApiKey {
            name: "Production API",
            description: "Main production workload",
            tier: "professional",
            rate_limit: Some(10000),
        },
        DemoApiKey {
            name: "Staging Environment",
            description: "Pre-production testing",
            tier: "starter",
            rate_limit: Some(1000),
        },
        DemoApiKey {
            name: "Mobile App Backend",
            description: "iOS and Android API",
            tier: "professional",
            rate_limit: Some(5000),
        },
        DemoApiKey {
            name: "Analytics Pipeline",
            description: "Data processing jobs",
            tier: "enterprise",
            rate_limit: None,
        },
        DemoApiKey {
            name: "Trial Key - Evaluation",
            description: "Testing the platform",
            tier: "trial",
            rate_limit: Some(100),
        },
        DemoApiKey {
            name: "Partner Integration",
            description: "Third-party integration",
            tier: "starter",
            rate_limit: Some(2000),
        },
        DemoApiKey {
            name: "Development",
            description: "Local dev testing",
            tier: "trial",
            rate_limit: Some(500),
        },
        DemoApiKey {
            name: "High Volume Batch",
            description: "Batch processing jobs",
            tier: "enterprise",
            rate_limit: None,
        },
        DemoApiKey {
            name: "Strava Sync",
            description: "Automated Strava activity sync",
            tier: "professional",
            rate_limit: Some(3000),
        },
        DemoApiKey {
            name: "Garmin Connect",
            description: "Garmin device integration",
            tier: "professional",
            rate_limit: Some(3000),
        },
    ]
}

/// Get demo API key definitions - extracted for function length (part 2)
fn get_demo_api_keys_part2() -> Vec<DemoApiKey> {
    vec![
        DemoApiKey {
            name: "Terra Bridge",
            description: "Multi-provider workout imports via Terra",
            tier: "starter",
            rate_limit: Some(1500),
        },
        DemoApiKey {
            name: "Apple Health",
            description: "HealthKit data sync",
            tier: "professional",
            rate_limit: Some(5000),
        },
        DemoApiKey {
            name: "Workout Analyzer",
            description: "AI-powered workout analysis",
            tier: "enterprise",
            rate_limit: None,
        },
        DemoApiKey {
            name: "Recovery Tracker",
            description: "Sleep and recovery metrics",
            tier: "starter",
            rate_limit: Some(1000),
        },
        DemoApiKey {
            name: "Nutrition Logger",
            description: "Meal and calorie tracking",
            tier: "starter",
            rate_limit: Some(800),
        },
        DemoApiKey {
            name: "Training Plan Bot",
            description: "Automated plan generation",
            tier: "professional",
            rate_limit: Some(4000),
        },
        DemoApiKey {
            name: "Race Predictor",
            description: "Performance prediction engine",
            tier: "enterprise",
            rate_limit: None,
        },
        DemoApiKey {
            name: "Social Feed",
            description: "Activity sharing and comments",
            tier: "starter",
            rate_limit: Some(2000),
        },
        DemoApiKey {
            name: "Coaching Dashboard",
            description: "Personal trainer tools",
            tier: "professional",
            rate_limit: Some(6000),
        },
        DemoApiKey {
            name: "Challenge Manager",
            description: "Competition and challenge API",
            tier: "starter",
            rate_limit: Some(1500),
        },
    ]
}

/// Get combined demo API keys
fn get_demo_api_keys() -> Vec<DemoApiKey> {
    let mut keys = get_demo_api_keys_part1();
    keys.extend(get_demo_api_keys_part2());
    keys
}

/// Get demo A2A client definitions - extracted for function length
fn get_demo_a2a_clients() -> Vec<DemoA2AClient> {
    vec![
        DemoA2AClient {
            name: "Claude Desktop",
            description: "AI Assistant Integration",
            capabilities: r#"["chat", "analyze"]"#,
        },
        DemoA2AClient {
            name: "Fitness Bot",
            description: "Automated workout analysis",
            capabilities: r#"["sync", "analyze", "recommend"]"#,
        },
        DemoA2AClient {
            name: "Data Pipeline",
            description: "ETL processing agent",
            capabilities: r#"["sync", "export"]"#,
        },
        DemoA2AClient {
            name: "GPT-4 Fitness Coach",
            description: "OpenAI-powered coaching",
            capabilities: r#"["chat", "recommend", "plan"]"#,
        },
        DemoA2AClient {
            name: "Gemini Analyzer",
            description: "Google AI workout insights",
            capabilities: r#"["analyze", "summarize"]"#,
        },
        DemoA2AClient {
            name: "Slack Bot",
            description: "Team fitness notifications",
            capabilities: r#"["notify", "report"]"#,
        },
        DemoA2AClient {
            name: "Discord Bot",
            description: "Community challenges",
            capabilities: r#"["notify", "leaderboard"]"#,
        },
        DemoA2AClient {
            name: "Zapier Integration",
            description: "Workflow automation",
            capabilities: r#"["sync", "export", "webhook"]"#,
        },
        DemoA2AClient {
            name: "Training Peaks Sync",
            description: "TrainingPeaks data bridge",
            capabilities: r#"["sync", "import", "export"]"#,
        },
        DemoA2AClient {
            name: "Garmin Agent",
            description: "Garmin Connect automation",
            capabilities: r#"["sync", "analyze"]"#,
        },
    ]
}

/// Status codes with realistic distribution (mostly 200s)
fn random_status_code(rng: &mut impl Rng) -> i32 {
    let roll: u8 = rng.gen_range(0..100);
    match roll {
        0..=85 => 200,  // 86% success
        86..=90 => 201, // 5% created
        91..=93 => 400, // 3% bad request
        94..=95 => 401, // 2% unauthorized
        96 => 403,      // 1% forbidden
        97 => 429,      // 1% rate limited
        98 => 500,      // 1% server error
        _ => 502,       // 1% bad gateway
    }
}

/// Generate realistic response time in ms
fn random_response_time(rng: &mut impl Rng, tool: &str) -> i32 {
    // Different tools have different baseline performance
    let base = match tool {
        "get_profile" | "get_goals" => 30,
        "get_activities" | "get_heart_rate" => 80,
        "analyze_workout" | "generate_insights" => 200,
        "sync_data" | "sync_garmin" | "sync_strava" => 500,
        "predict_race" | "calculate_ftp" => 300,
        _ => 100,
    };

    // Add variance (50-150% of base)
    let variance: f64 = rng.gen_range(0.5..1.5);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let result = (f64::from(base) * variance) as i32;
    result.max(20) // Minimum 20ms
}

/// Check if a date is a weekend
fn is_weekend(dt: DateTime<Utc>) -> bool {
    matches!(dt.weekday(), Weekday::Sat | Weekday::Sun)
}

/// Seed demo users, API keys, A2A clients, and time-series usage data for dashboard analytics.
///
/// # Errors
///
/// Returns an error if no admin user is found or if any repository operation fails.
pub async fn run(args: SeedArgs, repos: &RepositoryRegistry) -> AppResult<()> {
    let (admin_id, admin_email) = find_admin_user(repos, args.admin_email.as_deref()).await?;
    info!("=== Pierre MCP Server Demo Data Seeder: admin {admin_email} ({admin_id}) ===");

    if args.reset {
        reset_usage_tables(repos).await?;
    }
    seed_demo_pipeline(repos, &admin_id, args.days).await?;
    print_summary(repos).await
}

/// Clear all time-series usage data while keeping users, API keys, and A2A clients intact.
async fn reset_usage_tables(repos: &RepositoryRegistry) -> AppResult<()> {
    info!("Resetting usage data...");
    repos
        .seeder
        .seed_reset_table(SeedTable::ApiKeyUsage)
        .await?;
    repos.seeder.seed_reset_table(SeedTable::A2AUsage).await?;
    Ok(())
}

/// Run the five-step demo data seeding pipeline: users, API keys, A2A clients, and usage data.
async fn seed_demo_pipeline(
    repos: &RepositoryRegistry,
    admin_id: &Uuid,
    days: u32,
) -> AppResult<()> {
    let user_ids = step_demo_users(repos).await?;
    let api_key_ids = step_api_keys(repos, admin_id, &user_ids).await?;
    let a2a_client_ids = step_a2a_clients(repos, admin_id, &user_ids).await?;
    step_api_usage(repos, &api_key_ids, days).await?;
    step_a2a_usage(repos, &a2a_client_ids, days).await?;
    Ok(())
}

async fn step_demo_users(repos: &RepositoryRegistry) -> AppResult<Vec<Uuid>> {
    info!("Step 1: Creating demo users...");
    let user_ids = seed_demo_users(repos).await?;
    info!("  Created/found {} demo users", user_ids.len());
    Ok(user_ids)
}

async fn step_api_keys(
    repos: &RepositoryRegistry,
    admin_id: &Uuid,
    user_ids: &[Uuid],
) -> AppResult<Vec<Uuid>> {
    info!("Step 2: Creating API keys...");
    let ids = seed_api_keys(repos, admin_id, user_ids).await?;
    info!("  Created/found {} API keys", ids.len());
    Ok(ids)
}

async fn step_a2a_clients(
    repos: &RepositoryRegistry,
    admin_id: &Uuid,
    user_ids: &[Uuid],
) -> AppResult<Vec<Uuid>> {
    info!("Step 3: Creating A2A clients...");
    let ids = seed_a2a_clients(repos, admin_id, user_ids).await?;
    info!("  Created/found {} A2A clients", ids.len());
    Ok(ids)
}

async fn step_api_usage(
    repos: &RepositoryRegistry,
    api_key_ids: &[Uuid],
    days: u32,
) -> AppResult<()> {
    info!("Step 4: Generating API usage data ({days} days)...");
    let usage_count = seed_api_usage(repos, api_key_ids, days).await?;
    info!("  Generated {usage_count} usage records");
    Ok(())
}

async fn step_a2a_usage(
    repos: &RepositoryRegistry,
    a2a_client_ids: &[Uuid],
    days: u32,
) -> AppResult<()> {
    info!("Step 5: Generating A2A usage data...");
    let a2a_usage_count = seed_a2a_usage(repos, a2a_client_ids, days / 2).await?;
    info!("  Generated {a2a_usage_count} A2A usage records");
    Ok(())
}

/// Find admin user by email or get first admin
async fn find_admin_user(
    repos: &RepositoryRegistry,
    email: Option<&str>,
) -> AppResult<(Uuid, String)> {
    let user = if let Some(email) = email {
        repos.seeder.seed_find_user_by_email(email).await?
    } else {
        repos.seeder.seed_get_admin_user().await?
    };

    let Some(user) = user else {
        return Err(AppError::config(
            "No admin user found. Run 'cargo run --bin pierre-cli -- user create' first.",
        ));
    };

    Ok((user.id, user.email))
}

/// Seed demo users with direct DB operations (creates user + personal tenant).
/// Follows the same pattern as pierre-cli user creation: insert user row,
/// create personal tenant, link via `tenant_users` junction table, and
/// update the `tenant_id` column on users for backwards compatibility.
async fn seed_demo_users(repos: &RepositoryRegistry) -> AppResult<Vec<Uuid>> {
    let demo_users = get_demo_users();
    let mut user_ids = Vec::new();

    for user in &demo_users {
        // Check if user already exists
        let existing = repos.seeder.seed_check_user_exists(user.email).await?;

        let user_id = if let Some(id) = existing {
            info!("  Found existing user: {}", user.email);
            id
        } else {
            let id = create_demo_user(repos, user).await?;
            info!("  Created user: {} ({})", user.email, user.status);
            id
        };

        user_ids.push(user_id);
    }

    Ok(user_ids)
}

/// Create a single demo user with tenant via [`SeederRepository`] operations
async fn create_demo_user(repos: &RepositoryRegistry, user: &DemoUser) -> AppResult<Uuid> {
    let user_id = Uuid::new_v4();
    let password = user.password.unwrap_or(DEMO_USER_PASSWORD);
    let password_hash =
        hash(password, DEFAULT_COST).map_err(|e| AppError::config(format!("bcrypt error: {e}")))?;

    let now = Utc::now();

    // Insert user row
    let seed_user = SeedDemoUser {
        id: user_id,
        email: user.email.to_owned(),
        display_name: user.display_name.to_owned(),
        password_hash,
        tier: user.tier.to_owned(),
        status: user.status.to_owned(),
        is_admin: false,
        created_at: now,
    };
    repos.seeder.seed_insert_demo_user(&seed_user).await?;

    // Create personal tenant (plan matches user tier)
    let tenant_id = Uuid::new_v4();
    let tenant_slug = format!("user-{}", user_id.as_simple());
    let tenant_name = format!("{}'s Workspace", user.display_name);

    let seed_tenant = SeedTenant {
        id: tenant_id,
        name: tenant_name,
        slug: tenant_slug,
        plan: user.tier.to_owned(),
        owner_user_id: user_id,
        created_at: now,
        updated_at: now,
    };
    repos.seeder.seed_insert_tenant(&seed_tenant).await?;

    // Add user as tenant owner in junction table
    let tenant_user_id = Uuid::new_v4();
    repos
        .seeder
        .seed_insert_tenant_user(tenant_user_id, tenant_id, user_id, now)
        .await?;

    // Update tenant_id column on user for backwards compatibility
    repos
        .seeder
        .seed_update_user_tenant(user_id, tenant_id)
        .await?;

    Ok(user_id)
}

/// Seed API keys
async fn seed_api_keys(
    repos: &RepositoryRegistry,
    admin_id: &Uuid,
    user_ids: &[Uuid],
) -> AppResult<Vec<Uuid>> {
    let api_keys = get_demo_api_keys();
    let mut key_ids = Vec::new();
    let mut rng = StdRng::from_entropy();

    for (i, key) in api_keys.iter().enumerate() {
        // Check if exists
        let existing = repos.seeder.seed_check_api_key_by_name(key.name).await?;

        let key_id = if let Some(id) = existing {
            info!("  Found existing API key: {}", key.name);
            id
        } else {
            let id = Uuid::new_v4();

            // First 10 keys go to admin, rest distributed to demo users
            let user_id = if i < 10 {
                *admin_id
            } else {
                user_ids[(i - 10) % user_ids.len()]
            };

            let key_prefix = format!("pk_{:08x}", rng.gen::<u32>());
            let key_hash = format!("{:064x}", rng.gen::<u128>());
            let days_ago: i64 = rng.gen_range(5..30);
            let created_at = Utc::now() - Duration::days(days_ago);

            let expires_at = if key.tier == "trial" {
                Some(Utc::now() + Duration::days(14))
            } else {
                None
            };

            let seed_key = SeedApiKey {
                id,
                user_id,
                name: key.name.to_owned(),
                description: key.description.to_owned(),
                key_hash,
                key_prefix,
                tier: key.tier.to_owned(),
                rate_limit: key.rate_limit,
                expires_at,
                created_at,
            };
            repos.seeder.seed_insert_api_key(&seed_key).await?;

            info!("  Created API key: {} ({})", key.name, key.tier);
            id
        };

        key_ids.push(key_id);
    }

    Ok(key_ids)
}

/// Seed A2A clients
async fn seed_a2a_clients(
    repos: &RepositoryRegistry,
    admin_id: &Uuid,
    user_ids: &[Uuid],
) -> AppResult<Vec<Uuid>> {
    let clients = get_demo_a2a_clients();
    let mut client_ids = Vec::new();
    let mut rng = StdRng::from_entropy();

    for (i, client) in clients.iter().enumerate() {
        let existing = repos
            .seeder
            .seed_check_a2a_client_by_name(client.name)
            .await?;

        let client_id = if let Some(id) = existing {
            info!("  Found existing A2A client: {}", client.name);
            id
        } else {
            let id = Uuid::new_v4();
            let user_id = if i < 5 {
                *admin_id
            } else {
                user_ids[i % user_ids.len()]
            };
            let public_key = format!("pk_a2a_{:016x}", rng.gen::<u64>());
            let client_secret = format!("{:064x}", rng.gen::<u128>());
            let permissions = r#"["read", "write"]"#.to_owned();
            let days_ago: i64 = rng.gen_range(10..45);
            let created_at = Utc::now() - Duration::days(days_ago);
            let updated_at = Utc::now();

            let seed_client = SeedA2AClient {
                id,
                user_id,
                name: client.name.to_owned(),
                description: client.description.to_owned(),
                public_key,
                client_secret,
                permissions,
                capabilities: client.capabilities.to_owned(),
                created_at,
                updated_at,
            };
            repos.seeder.seed_insert_a2a_client(&seed_client).await?;

            info!("  Created A2A client: {}", client.name);
            id
        };

        client_ids.push(client_id);
    }

    Ok(client_ids)
}

/// Seed API usage data with realistic patterns
async fn seed_api_usage(
    repos: &RepositoryRegistry,
    api_key_ids: &[Uuid],
    days: u32,
) -> AppResult<u64> {
    let mut rng = StdRng::from_entropy();
    let mut total_records: u64 = 0;

    for (idx, key_id) in api_key_ids.iter().enumerate() {
        // Determine base traffic based on key position (enterprise keys get more)
        let base_requests: u32 = if idx < 5 {
            200 // High volume keys
        } else if idx < 10 {
            100 // Medium volume
        } else {
            50 // Lower volume
        };

        for day_offset in 0..days {
            let day = Utc::now() - Duration::days(i64::from(day_offset));

            // Weekend adjustment (30% of normal)
            let weekend_factor: f64 = if is_weekend(day) { 0.3 } else { 1.0 };

            // Random daily variance
            let variance: f64 = rng.gen_range(0.7..1.3);

            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let daily_requests = (f64::from(base_requests) * weekend_factor * variance) as u32;

            for _ in 0..daily_requests {
                let id = Uuid::new_v4();
                let tool = TOOLS[rng.gen_range(0..TOOLS.len())];
                let status_code = random_status_code(&mut rng);
                let response_time = random_response_time(&mut rng, tool);

                // Generate timestamp with business hours bias
                let hour: u32 = if rng.gen_bool(0.7) {
                    rng.gen_range(8..20) // 70% during business hours
                } else {
                    rng.gen_range(0..24)
                };
                let minute: u32 = rng.gen_range(0..60);
                let second: u32 = rng.gen_range(0..60);

                let timestamp = day
                    .with_hour(hour)
                    .unwrap_or(day)
                    .with_minute(minute)
                    .unwrap_or(day)
                    .with_second(second)
                    .unwrap_or(day);

                // Ignore errors for duplicate inserts
                let usage = SeedApiKeyUsage {
                    id,
                    api_key_id: *key_id,
                    timestamp,
                    tool_name: tool.to_owned(),
                    status_code,
                    response_time_ms: response_time,
                };
                if repos.seeder.seed_insert_api_key_usage(&usage).await.is_ok() {
                    total_records += 1;
                }
            }
        }
        info!("  Generated usage for key: {}...", &key_id.to_string()[..8]);
    }

    Ok(total_records)
}

/// Seed A2A usage data
async fn seed_a2a_usage(
    repos: &RepositoryRegistry,
    client_ids: &[Uuid],
    days: u32,
) -> AppResult<u64> {
    let mut rng = StdRng::from_entropy();
    let mut total_records: u64 = 0;

    for client_id in client_ids {
        let base_requests: u32 = rng.gen_range(20..50);

        for day_offset in 0..days {
            let day = Utc::now() - Duration::days(i64::from(day_offset));
            let daily_requests: u32 = rng.gen_range(base_requests / 2..base_requests * 2);

            for _ in 0..daily_requests {
                let id = Uuid::new_v4();
                let tool = A2A_TOOLS[rng.gen_range(0..A2A_TOOLS.len())];
                let status_code = random_status_code(&mut rng);
                let response_time: i32 = rng.gen_range(100..600);

                let hour: u32 = rng.gen_range(0..24);
                let minute: u32 = rng.gen_range(0..60);

                let timestamp = day
                    .with_hour(hour)
                    .unwrap_or(day)
                    .with_minute(minute)
                    .unwrap_or(day);

                // Ignore errors for duplicate inserts
                let usage = SeedA2AUsage {
                    id,
                    client_id: *client_id,
                    timestamp,
                    tool_name: tool.to_owned(),
                    status_code,
                    response_time_ms: response_time,
                };
                if repos.seeder.seed_insert_a2a_usage(&usage).await.is_ok() {
                    total_records += 1;
                }
            }
        }
    }

    Ok(total_records)
}

/// Print visual testing credentials
fn print_test_credentials() {
    info!(
        "\n\
         === Visual Testing Credentials ===\n\
         Web Test User:    webtest@pierre.dev / WebTest123!\n\
         Mobile Test User: mobiletest@pierre.dev / MobileTest1234\n\
         Demo Users:       DemoUser123! (for alice@acme.com, bob@startup.io, etc.)\n\
         \n\
         Done! Restart the server to see the demo data in the dashboard."
    );
}

/// Print summary statistics
async fn print_summary(repos: &RepositoryRegistry) -> AppResult<()> {
    let tables: &[(&str, SeedTable)] = &[
        ("Users", SeedTable::Users),
        ("API Keys", SeedTable::ApiKeys),
        ("API Usage Records", SeedTable::ApiKeyUsage),
        ("A2A Clients", SeedTable::A2AClients),
        ("A2A Usage Records", SeedTable::A2AUsage),
    ];

    for (label, table) in tables {
        let count = repos.seeder.seed_count_table(*table).await?;
        info!("{label}: {count}");
    }

    print_test_credentials();
    Ok(())
}
