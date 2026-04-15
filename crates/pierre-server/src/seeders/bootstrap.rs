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
//! ADMIN_PASSWORD=SecurePass123 pierre-cli seed bootstrap
//!
//! # Override email
//! pierre-cli seed bootstrap --admin-email ops@dravr.ai
//! ```

use bcrypt::{hash, DEFAULT_COST};
use chrono::Utc;
use pierre_core::errors::{AppError, AppResult};
use pierre_database::seed_models::{SeedDemoUser, SeedTenant};
use pierre_database::RepositoryRegistry;
use tracing::info;
use uuid::Uuid;

/// Password assigned to seed users (not the admin — admin password comes from env/CLI)
const DEMO_USER_PASSWORD: &str = "DemoUser123!";

/// CLI arguments for the bootstrap seeder.
#[derive(clap::Args)]
pub struct SeedArgs {
    /// Admin email address
    #[arg(long, env = "ADMIN_EMAIL", default_value = "admin@example.com")]
    pub admin_email: String,

    /// Admin password (required — no hardcoded default)
    #[arg(long, env = "ADMIN_PASSWORD")]
    pub admin_password: String,
}

/// Bootstrap user definition
struct BootstrapUser {
    email: &'static str,
    display_name: &'static str,
    tier: &'static str,
    is_admin: bool,
    /// Optional custom password (defaults to `DEMO_USER_PASSWORD` if None)
    password: Option<&'static str>,
}

/// Fixed set of demo users created on every fresh deployment
const DEMO_USERS: &[BootstrapUser] = &[
    BootstrapUser {
        email: "alice@demo.pierre.dev",
        display_name: "Alice Demo",
        tier: "professional",
        is_admin: false,
        password: None,
    },
    BootstrapUser {
        email: "bob@demo.pierre.dev",
        display_name: "Bob Demo",
        tier: "starter",
        is_admin: false,
        password: None,
    },
    BootstrapUser {
        email: "phil_test@dravr.ai",
        display_name: "Phil Test",
        tier: "professional",
        is_admin: false,
        password: Some("fougeresEtSapin2017#!$"),
    },
    BootstrapUser {
        email: "jf_test@dravr.ai",
        display_name: "JF Test",
        tier: "professional",
        is_admin: false,
        password: Some("fougeresEtSapin2017#!$"),
    },
];

/// Update password hash and role for an existing user (upsert via ON CONFLICT)
async fn upsert_user_credentials(
    repos: &RepositoryRegistry,
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
    repos.seeder.seed_insert_demo_user(&seed_user).await?;
    Ok(())
}

/// Create a single user with a personal tenant (reuses the `seed_demo_data` pattern)
async fn create_user(
    repos: &RepositoryRegistry,
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
    repos.seeder.seed_insert_demo_user(&seed_user).await?;

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

/// Seed an admin user plus the fixed demo users, upserting credentials if they already exist.
///
/// # Errors
///
/// Returns an error if bcrypt hashing fails or if any of the repository operations fail.
pub async fn run(args: SeedArgs, repos: &RepositoryRegistry) -> AppResult<()> {
    info!("=== Pierre Bootstrap Seeder ===");

    sync_user(
        repos,
        &args.admin_email,
        "Admin",
        &args.admin_password,
        "enterprise",
        true,
    )
    .await?;

    for user in DEMO_USERS {
        let password = user.password.unwrap_or(DEMO_USER_PASSWORD);
        sync_user(
            repos,
            user.email,
            user.display_name,
            password,
            user.tier,
            user.is_admin,
        )
        .await?;
    }

    info!("=== Bootstrap seeding complete ===");
    Ok(())
}

/// Idempotently create or update a single user, picking the right path based on existence.
async fn sync_user(
    repos: &RepositoryRegistry,
    email: &str,
    display_name: &str,
    password: &str,
    tier: &str,
    is_admin: bool,
) -> AppResult<()> {
    let existing = repos.seeder.seed_check_user_exists(email).await?;
    if let Some(id) = existing {
        upsert_user_credentials(repos, email, display_name, password, tier, is_admin).await?;
        info!("Updated user credentials: {email} ({id})");
    } else {
        let id = create_user(repos, email, display_name, password, tier, is_admin).await?;
        info!("Created user: {email} ({id})");
    }
    Ok(())
}
