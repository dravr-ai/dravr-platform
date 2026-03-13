// ABOUTME: Bootstrap seeder for fresh Cloud Run deployments (admin + demo users)
// ABOUTME: Creates admin user from env/CLI credentials and demo users with personal tenants
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Bootstrap seeder for Pierre MCP Server.
//!
//! Creates the minimum set of users needed for a working deployment:
//! - Admin user (email/password from env or CLI)
//! - Two demo users for testing
//!
//! All operations are idempotent via `seed_check_user_exists`.
//!
//! Usage:
//! ```bash
//! # Minimal: password from environment
//! ADMIN_PASSWORD=SecurePass123 cargo run --bin seed-bootstrap
//!
//! # Override email and database URL
//! cargo run --bin seed-bootstrap -- --admin-email ops@dravr.ai --database-url postgresql://...
//!
//! # Verbose output
//! cargo run --bin seed-bootstrap -- -v
//! ```

use bcrypt::{hash, DEFAULT_COST};
use chrono::Utc;
use clap::Parser;
use pierre_core::errors::{AppError, AppResult};
use pierre_database::plugins::factory::Database;
use pierre_database::repositories::SeederRepository;
use pierre_database::seed_models::{SeedDemoUser, SeedTenant};
use std::env;
use tracing::info;
use uuid::Uuid;

/// Password for demo users (not the admin — admin password comes from env/CLI)
const DEMO_USER_PASSWORD: &str = "DemoUser123!";

#[derive(Parser)]
#[command(
    name = "seed-bootstrap",
    about = "Pierre MCP Server Bootstrap Seeder",
    long_about = "Create admin and demo users for a fresh deployment (idempotent)"
)]
struct SeedArgs {
    /// Database URL override (falls back to `DATABASE_URL` env var)
    #[arg(long)]
    database_url: Option<String>,

    /// Admin email address
    #[arg(long, env = "ADMIN_EMAIL", default_value = "admin@example.com")]
    admin_email: String,

    /// Admin password (required — no hardcoded default)
    #[arg(long, env = "ADMIN_PASSWORD")]
    admin_password: String,

    /// Enable verbose logging
    #[arg(long, short = 'v')]
    verbose: bool,
}

/// Bootstrap user definition
struct BootstrapUser {
    email: &'static str,
    display_name: &'static str,
    tier: &'static str,
    is_admin: bool,
}

/// Fixed set of demo users created on every fresh deployment
const DEMO_USERS: &[BootstrapUser] = &[
    BootstrapUser {
        email: "alice@demo.pierre.dev",
        display_name: "Alice Demo",
        tier: "professional",
        is_admin: false,
    },
    BootstrapUser {
        email: "bob@demo.pierre.dev",
        display_name: "Bob Demo",
        tier: "starter",
        is_admin: false,
    },
];

/// Update password hash and role for an existing user (upsert via ON CONFLICT)
async fn upsert_user_credentials(
    db: &Database,
    email: &str,
    display_name: &str,
    password: &str,
    tier: &str,
    is_admin: bool,
) -> AppResult<()> {
    let password_hash =
        hash(password, DEFAULT_COST).map_err(|e| AppError::config(format!("bcrypt error: {e}")))?;
    let now = Utc::now();

    // The ON CONFLICT(email) DO UPDATE in seed_insert_demo_user handles the upsert.
    // We use a dummy UUID since the INSERT won't execute (user already exists).
    let seed_user = SeedDemoUser {
        id: Uuid::new_v4(),
        email: email.to_owned(),
        display_name: display_name.to_owned(),
        password_hash,
        tier: tier.to_owned(),
        status: "active".to_owned(),
        is_admin,
        created_at: now,
    };
    db.seed_insert_demo_user(&seed_user).await?;
    Ok(())
}

/// Create a single user with a personal tenant (reuses the `seed_demo_data` pattern)
async fn create_user(
    db: &Database,
    email: &str,
    display_name: &str,
    password: &str,
    tier: &str,
    is_admin: bool,
) -> AppResult<Uuid> {
    let user_id = Uuid::new_v4();
    let password_hash =
        hash(password, DEFAULT_COST).map_err(|e| AppError::config(format!("bcrypt error: {e}")))?;
    let now = Utc::now();

    // Insert user row
    let seed_user = SeedDemoUser {
        id: user_id,
        email: email.to_owned(),
        display_name: display_name.to_owned(),
        password_hash,
        tier: tier.to_owned(),
        status: "active".to_owned(),
        is_admin,
        created_at: now,
    };
    db.seed_insert_demo_user(&seed_user).await?;

    // Create personal tenant (plan matches user tier)
    let tenant_id = Uuid::new_v4();
    let tenant_slug = format!("user-{}", user_id.as_simple());
    let tenant_name = format!("{display_name}'s Workspace");

    let seed_tenant = SeedTenant {
        id: tenant_id,
        name: tenant_name,
        slug: tenant_slug,
        plan: tier.to_owned(),
        owner_user_id: user_id,
        created_at: now,
        updated_at: now,
    };
    db.seed_insert_tenant(&seed_tenant).await?;

    // Add user as tenant owner in junction table
    let tenant_user_id = Uuid::new_v4();
    db.seed_insert_tenant_user(tenant_user_id, tenant_id, user_id, now)
        .await?;

    // Update tenant_id column on user for backwards compatibility
    db.seed_update_user_tenant(user_id, tenant_id).await?;

    Ok(user_id)
}

#[tokio::main]
async fn main() -> AppResult<()> {
    let args = SeedArgs::parse();

    let log_level = if args.verbose { "debug" } else { "info" };
    tracing_subscriber::fmt().with_env_filter(log_level).init();

    info!("=== Pierre Bootstrap Seeder ===");

    let database_url = args
        .database_url
        .or_else(|| env::var("DATABASE_URL").ok())
        .unwrap_or_else(|| "sqlite:./data/users.db".into());

    info!("Connecting to database...");
    let db = Database::init_for_seeding(&database_url).await?;

    // --- Admin user ---
    let admin_email = &args.admin_email;
    let existing_admin = db.seed_check_user_exists(admin_email).await?;
    if let Some(id) = existing_admin {
        // Update password/role for existing admin (upsert via ON CONFLICT)
        upsert_user_credentials(
            &db,
            admin_email,
            "Admin",
            &args.admin_password,
            "enterprise",
            true,
        )
        .await?;
        info!("Updated admin user credentials: {admin_email} ({id})");
    } else {
        let id = create_user(
            &db,
            admin_email,
            "Admin",
            &args.admin_password,
            "enterprise",
            true,
        )
        .await?;
        info!("Created admin user: {admin_email} ({id})");
    }

    // --- Demo users ---
    for user in DEMO_USERS {
        let existing = db.seed_check_user_exists(user.email).await?;
        if let Some(id) = existing {
            // Update password/role for existing demo user (upsert via ON CONFLICT)
            upsert_user_credentials(
                &db,
                user.email,
                user.display_name,
                DEMO_USER_PASSWORD,
                user.tier,
                user.is_admin,
            )
            .await?;
            info!("Updated demo user credentials: {} ({id})", user.email);
        } else {
            let id = create_user(
                &db,
                user.email,
                user.display_name,
                DEMO_USER_PASSWORD,
                user.tier,
                user.is_admin,
            )
            .await?;
            info!("Created demo user: {} ({id})", user.email);
        }
    }

    info!("=== Bootstrap seeding complete ===");
    Ok(())
}
