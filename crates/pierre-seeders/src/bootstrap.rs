// ABOUTME: Bootstrap seeder for fresh Cloud Run deployments (admin, plus demo and test accounts on request)
// ABOUTME: Creates the admin from env/CLI credentials; the demo/test accounts only with a password given from outside the source
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Bootstrap seeder for Pierre MCP Server.
//!
//! Creates the minimum set of users needed for a working deployment:
//! - Admin user (email/password from env or CLI)
//! - Two demo and two test accounts, only when a demo password is given
//!   (`--demo-password` / `BOOTSTRAP_DEMO_PASSWORD`)
//!
//! The source holds no password for any of them: the repository is public.
//! Without a demo password the run neither creates nor touches the demo and
//! test accounts (not their password, their role or their status), and says
//! so in one info log. The deployed seed job sets none, so the accounts
//! there are managed out of band.
//!
//! All operations are idempotent via `seed_check_user_exists`. An account
//! that already exists gets its credentials refreshed and its role brought
//! to the seed's through `UserRepository::set_admin_status`, never by SQL.
//! Only an active account is the bootstrap admin: one `ADMIN_EMAIL` names
//! that is pending or suspended is left as it is, with the role, the
//! password, the display name and the locale it has, and the run says so in
//! a warning.
//!
//! Usage:
//! ```bash
//! # Minimal: password from environment
//! ADMIN_PASSWORD=SecurePass123 pierre-cli seed bootstrap
//!
//! # Override email
//! pierre-cli seed bootstrap --admin-email ops@dravr.ai
//!
//! # Also seed the demo and test accounts, with a password from the environment
//! BOOTSTRAP_DEMO_PASSWORD=$LOCAL_DEMO_PASSWORD pierre-cli seed bootstrap
//! ```

use bcrypt::{hash, DEFAULT_COST};
use chrono::Utc;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::permissions::UserRole;
use pierre_database::repositories::AuditSource;
use pierre_database::seed_models::{SeedDemoUser, SeedTenant, SEED_LOCALE};
use pierre_database::RepositoryRegistry;
use pierre_middleware::mask_email;
use pierre_services::admin_ops::{
    record_withdrawn_admin_credentials, refuse_promoting_unless_active, AdminStandingLost,
    StandingChange,
};
use std::fmt;
use tracing::{info, warn};
use uuid::Uuid;

/// CLI arguments for the bootstrap seeder.
#[derive(clap::Args)]
pub struct SeedArgs {
    /// Admin email address
    #[arg(long, env = "ADMIN_EMAIL", default_value = "admin@example.com")]
    pub admin_email: String,

    /// Admin password (required — no hardcoded default)
    #[arg(long, env = "ADMIN_PASSWORD")]
    pub admin_password: String,

    /// Password for the demo and test accounts. Without one (or with an
    /// empty one) the run neither creates nor touches them. Prefer
    /// `BOOTSTRAP_DEMO_PASSWORD` over the flag: a flag value lands in the
    /// process argument list and the shell history.
    #[arg(long, env = "BOOTSTRAP_DEMO_PASSWORD")]
    pub demo_password: Option<String>,
}

impl fmt::Debug for SeedArgs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SeedArgs")
            .field("admin_email", &self.admin_email)
            .field("admin_password", &"[REDACTED]")
            .field(
                "demo_password",
                &self.demo_password.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

/// Bootstrap user definition
struct BootstrapUser {
    email: &'static str,
    display_name: &'static str,
    tier: &'static str,
    role: UserRole,
}

/// The demo and test accounts, seeded only when the run is given a demo
/// password ([`SeedArgs::demo_password`])
const DEMO_USERS: &[BootstrapUser] = &[
    BootstrapUser {
        email: "alice@demo.pierre.dev",
        display_name: "Alice Demo",
        tier: "professional",
        role: UserRole::User,
    },
    BootstrapUser {
        email: "bob@demo.pierre.dev",
        display_name: "Bob Demo",
        tier: "starter",
        role: UserRole::User,
    },
    BootstrapUser {
        email: "phil_test@dravr.ai",
        display_name: "Phil Test",
        tier: "professional",
        role: UserRole::User,
    },
    BootstrapUser {
        email: "jf_test@dravr.ai",
        display_name: "JF Test",
        tier: "professional",
        role: UserRole::User,
    },
];

/// Refresh the password hash, display name and locale of the existing user
/// `user_id` (upsert via ON CONFLICT), then bring its role to the seed's.
async fn upsert_user_credentials(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    email: &str,
    display_name: &str,
    password: &str,
    tier: &str,
    role: UserRole,
) -> AppResult<()> {
    let password_hash =
        hash(password, DEFAULT_COST).map_err(|e| AppError::config(format!("bcrypt error: {e}")))?;
    let now = Utc::now();

    // The ON CONFLICT(email) DO UPDATE in seed_insert_demo_user handles the upsert.
    // We use a dummy UUID since the INSERT won't execute (user already exists),
    // and the role bound here is not written: `sync_role` below changes it.
    let seed_user = SeedDemoUser {
        id: Uuid::new_v4(),
        email: email.to_owned(),
        display_name: display_name.to_owned(),
        password_hash,
        tier: tier.to_owned(),
        status: "active".to_owned(),
        role,
        created_at: now,
        locale: SEED_LOCALE.to_owned(),
    };
    repos.seeder.seed_insert_demo_user(&seed_user).await?;
    sync_role(repos, user_id, role).await
}

/// Bring the existing account `user_id` to the seed's `role`, through the
/// write every other surface uses.
///
/// The bootstrap admin is made an admin again on every run, and a demo
/// account someone promoted goes back to a plain user: its password is the
/// shared demo password. Only an active account reaches a promotion here
/// ([`left_as_it_is`] is asked first). A demotion takes no such look: it
/// removes standing, whatever the status.
/// The repository refuses to demote the last active admin, which
/// fails the run rather than leaving the platform without one, and a demoted
/// admin loses the device logins and impersonations it held, on the trail
/// under the `pierre-cli` database source.
///
/// # Errors
///
/// Returns the repository's refusal or error.
async fn sync_role(repos: &RepositoryRegistry, user_id: Uuid, role: UserRole) -> AppResult<()> {
    let Some(stored) = repos.users.get_global(user_id).await? else {
        return Ok(());
    };
    if stored.role == role {
        return Ok(());
    }
    let write = repos
        .users
        .set_admin_status(user_id, role.is_admin())
        .await?;
    info!(
        user_id = %user_id,
        role = role.as_str(),
        "Seeder brought an existing account to its seeded role"
    );
    record_withdrawn_admin_credentials(
        repos,
        &write.withdrawn,
        StandingChange {
            cause: AdminStandingLost::Demoted,
            account: Some(user_id),
            actor: None,
            source: AuditSource::CliDatabase,
        },
    )
    .await;
    Ok(())
}

/// Whether the run leaves alone the existing account `user_id`, which
/// `ADMIN_EMAIL` names: it is pending or suspended.
///
/// An admin is an approved account first, the rule every other promotion
/// takes ([`refuse_promoting_unless_active`]), so the run does not make such
/// an account the bootstrap admin, whatever role it holds. It does not take
/// its sign-in either: the password, the display name and the locale the
/// run asserts are the bootstrap admin's, and replacing them on an account
/// the run does not make one would lock its holder out and hand the account
/// to whoever holds `ADMIN_PASSWORD`. The run goes on with a warning naming
/// the account.
///
/// # Errors
///
/// Returns the repository's error when the account cannot be read.
async fn left_as_it_is(repos: &RepositoryRegistry, user_id: Uuid) -> AppResult<bool> {
    let Some(stored) = repos.users.get_global(user_id).await? else {
        return Ok(false);
    };
    let Err(refusal) = refuse_promoting_unless_active(&stored) else {
        return Ok(false);
    };
    warn!(
        user_id = %user_id,
        status = %stored.user_status,
        "Seeder left an existing account as it is, with the role and the credentials it \
         has: {refusal}"
    );
    Ok(true)
}

/// Create a single user with a personal tenant (reuses the `seed_demo_data` pattern)
async fn create_user(
    repos: &RepositoryRegistry,
    email: &str,
    display_name: &str,
    password: &str,
    tier: &str,
    role: UserRole,
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
        role,
        created_at: now,
        locale: SEED_LOCALE.to_owned(),
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

/// Seed an admin user, plus the fixed demo and test accounts when a demo
/// password is given, upserting credentials if they already exist.
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
        UserRole::Admin,
    )
    .await?;

    sync_demo_accounts(repos, args.demo_password.as_deref()).await?;

    info!("=== Bootstrap seeding complete ===");
    Ok(())
}

/// Seed the demo and test accounts with `demo_password`, or, without one (or
/// with an empty one), leave them as they are: neither created nor touched.
///
/// # Errors
///
/// Returns an error if bcrypt hashing or a repository operation fails.
async fn sync_demo_accounts(
    repos: &RepositoryRegistry,
    demo_password: Option<&str>,
) -> AppResult<()> {
    let Some(demo_password) = demo_password.filter(|p| !p.is_empty()) else {
        info!(
            "No demo password given (--demo-password / BOOTSTRAP_DEMO_PASSWORD): the demo and \
             test accounts are neither created nor touched"
        );
        return Ok(());
    };
    for user in DEMO_USERS {
        sync_user(
            repos,
            user.email,
            user.display_name,
            demo_password,
            user.tier,
            user.role,
        )
        .await?;
    }
    Ok(())
}

/// Idempotently create or update a single user, picking the right path based on existence.
///
/// An existing account the run would make the bootstrap admin is written
/// only while it is active ([`left_as_it_is`]).
async fn sync_user(
    repos: &RepositoryRegistry,
    email: &str,
    display_name: &str,
    password: &str,
    tier: &str,
    role: UserRole,
) -> AppResult<()> {
    let Some(id) = repos.seeder.seed_check_user_exists(email).await? else {
        let id = create_user(repos, email, display_name, password, tier, role).await?;
        info!("Created user: {} ({id})", mask_email(email));
        return Ok(());
    };
    // Asked before anything is written on the account, its credentials
    // included.
    if role.is_admin() && left_as_it_is(repos, id).await? {
        return Ok(());
    }
    upsert_user_credentials(repos, id, email, display_name, password, tier, role).await?;
    info!("Updated user credentials: {} ({id})", mask_email(email));
    Ok(())
}
