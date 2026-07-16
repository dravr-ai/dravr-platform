// ABOUTME: Pierre CLI - unified command-line tool for Pierre MCP Server management
// ABOUTME: Handles user creation, token management, and administrative operations
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
//!
//! Usage:
//! ```bash
//! # Create admin user for frontend login
//! pierre-cli user create --email admin@example.com --password yourpassword
//!
//! # Create super admin user
//! pierre-cli user create --email admin@example.com --password yourpassword --super-admin
//!
//! # Promote an existing user to admin
//! pierre-cli user promote --email user@example.com
//!
//! # Promote an existing user to super admin (use `user create --force`)
//! pierre-cli user create --email user@example.com --password X --force --super-admin
//!
//! # Demote an admin user back to regular user
//! pierre-cli user demote --email user@example.com
//!
//! # List all admin users
//! pierre-cli user list-admins
//!
//! # Generate a new admin token
//! pierre-cli token generate --service pierre_admin_service
//!
//! # Generate a super admin token
//! pierre-cli token generate --service admin_console --super-admin
//!
//! # List all admin tokens
//! pierre-cli token list
//!
//! # Revoke an admin token
//! pierre-cli token revoke admin_token_123
//!
//! # Show token statistics
//! pierre-cli token stats
//!
//! # Seed reference data (admin/demo users, coaches, mobility, etc.)
//! ADMIN_PASSWORD=secret pierre-cli seed bootstrap
//! pierre-cli seed demo-data
//! pierre-cli seed coaches
//! pierre-cli seed mobility
//! pierre-cli seed social
//! pierre-cli seed synthetic-activities --email alice@example.com --count 200
//! pierre-cli seed llm-usage --days 60
//! pierre-cli seed insight-samples --validate
//! ```

mod commands;
mod helpers;

use clap::{Parser, Subcommand};
use pierre_auth::key_management::KeyManager;
#[cfg(feature = "postgresql")]
use pierre_core::config::database::PostgresPoolConfig;
use pierre_core::errors::AppResult;
use pierre_core::redaction::redact_url;
use pierre_database::backends::{factory::Database, DatabaseProvider};

type Result<T> = AppResult<T>;
use std::env;
use std::sync::Arc;
use tracing::info;

use helpers::jwks::initialize_jwks_manager;

#[derive(Parser)]
#[command(
    name = "pierre-cli",
    about = "Pierre MCP Server Management CLI",
    long_about = "Unified command-line tool for managing Pierre MCP Server users, tokens, and administration."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,

    /// Database URL override
    #[arg(long, global = true)]
    database_url: Option<String>,

    /// Encryption key override (base64 encoded)
    #[arg(long, global = true)]
    encryption_key: Option<String>,

    /// Enable debug logging
    #[arg(long, short = 'v', global = true)]
    verbose: bool,
}

#[non_exhaustive]
#[derive(Subcommand)]
enum Command {
    /// User management commands
    User {
        #[command(subcommand)]
        action: UserCommand,
    },

    /// Token management commands
    Token {
        #[command(subcommand)]
        action: TokenCommand,
    },

    /// Reference and demo data seeders
    Seed {
        #[command(subcommand)]
        action: commands::seed::SeedCommand,
    },

    /// Drift detection between contremaitre source files and the prod DB
    ///
    /// Background: the seed-coaches Cloud Run job silently exit-1'd for 3.5
    /// weeks after c6630e46 (2026-05-01) and nothing caught it. This command
    /// is the daily drift gate — Cloud Run runs `check-drift coaches`,
    /// non-zero exit triggers the `dravr-mcp-server-job-failures` alert.
    CheckDrift {
        #[command(subcommand)]
        action: commands::drift::DriftCommand,
    },

    /// Encryption key operations (DEK rotation, KEK re-wrap) — ADR-017
    Key {
        #[command(subcommand)]
        action: commands::key::KeyCommand,
    },

    /// Coaching harness operator commands (claim verdict backfill, etc.)
    #[cfg(feature = "tools-verification")]
    Harness {
        #[command(subcommand)]
        action: commands::harness::HarnessCommand,
    },

    /// Authenticated remote session (gcloud-style login to a pierre-server)
    Auth {
        #[command(subcommand)]
        action: AuthCommand,
    },

    /// Manage the Strava shared-app OAuth pool on a remote server
    StravaPool {
        #[command(subcommand)]
        action: StravaPoolCommand,
    },

    /// Tenant administration (set plan)
    Tenant {
        #[command(subcommand)]
        action: TenantCommand,
    },

    /// Per-user MCP tool allow/deny
    Tool {
        #[command(subcommand)]
        action: ToolCommand,
    },
}

#[non_exhaustive]
#[derive(Subcommand)]
enum AuthCommand {
    /// Log in to a remote pierre-server via the device flow, caching the token
    Login {
        /// Base URL of the pierre-server (e.g. `https://api.dev.dravr.ai`)
        #[arg(long)]
        server: String,

        /// Print the approval URL instead of auto-opening it (for headless/SSH/CI)
        #[arg(long)]
        no_browser: bool,
    },

    /// Remove cached credentials
    Logout,

    /// Show the current cached login
    Status,

    /// Approve a pending device login (super-admin)
    Approve {
        /// The user code shown by the operator's `auth login`
        user_code: String,

        /// Server override (defaults to the cached login's server)
        #[arg(long)]
        server: Option<String>,

        /// Super-admin token authorizing the approval (defaults to cached login)
        #[arg(long)]
        token: Option<String>,
    },

    /// Deny a pending device login (super-admin)
    Deny {
        /// The user code shown by the operator's `auth login`
        user_code: String,

        /// Server override (defaults to the cached login's server)
        #[arg(long)]
        server: Option<String>,

        /// Super-admin token authorizing the denial (defaults to cached login)
        #[arg(long)]
        token: Option<String>,
    },
}

#[non_exhaustive]
#[derive(Subcommand)]
enum StravaPoolCommand {
    /// Add or update a Strava OAuth pool app (grows athlete-seat capacity)
    Add {
        /// Strava app `client_id` (public)
        #[arg(long)]
        client_id: String,

        /// Strava app `client_secret` (sent over TLS, encrypted server-side)
        #[arg(long)]
        client_secret: String,

        /// Athlete seat cap for this app
        #[arg(long)]
        seat_cap: u32,

        /// Optional human-readable label
        #[arg(long)]
        label: Option<String>,
    },

    /// List pool apps and aggregate seat usage
    List,

    /// Enable a pool app
    Enable {
        /// The pool app's `client_id`
        #[arg(long)]
        client_id: String,
    },

    /// Disable a pool app (skipped for new connections)
    Disable {
        /// The pool app's `client_id`
        #[arg(long)]
        client_id: String,
    },

    /// Delete a pool app
    Delete {
        /// The pool app's `client_id`
        #[arg(long)]
        client_id: String,
    },
}

#[non_exhaustive]
#[derive(Subcommand)]
enum UserCommand {
    /// Create a new admin user for frontend login
    Create {
        /// Admin email (required)
        #[arg(long)]
        email: String,

        /// Admin password (required)
        #[arg(long)]
        password: String,

        /// Admin display name (defaults to email prefix if not specified)
        #[arg(long)]
        name: Option<String>,

        /// Force update if user already exists
        #[arg(long)]
        force: bool,

        /// Create super admin user (can impersonate other users)
        #[arg(long)]
        super_admin: bool,
    },

    /// Promote an existing user to admin
    Promote {
        /// Email of the user to promote
        #[arg(long)]
        email: String,
    },

    /// Demote an admin user back to a regular user
    Demote {
        /// Email of the user to demote
        #[arg(long)]
        email: String,
    },

    /// List all admin users
    ListAdmins,

    /// Set a user's billing/quota tier (Starter / Professional / Enterprise)
    SetTier {
        /// Email of the user
        #[arg(long)]
        email: String,

        /// Tier: starter | professional | enterprise
        #[arg(long)]
        tier: String,

        /// Operator note recorded on the override marker
        #[arg(long)]
        note: Option<String>,
    },

    /// Clear a user's tier override so the billing webhook drives the tier again
    ClearTier {
        /// Email of the user
        #[arg(long)]
        email: String,
    },

    /// Approve a pending user (status → active) and provision their MCP token
    Approve {
        /// Email of the user
        #[arg(long)]
        email: String,

        /// Reason recorded in the audit log
        #[arg(long)]
        reason: Option<String>,
    },

    /// Suspend a user (status → suspended); they can no longer log in
    Suspend {
        /// Email of the user
        #[arg(long)]
        email: String,

        /// Reason recorded in the audit log
        #[arg(long)]
        reason: Option<String>,
    },

    /// Issue a one-time password reset token (printed once, deliver out-of-band)
    ResetPassword {
        /// Email of the user
        #[arg(long)]
        email: String,
    },

    /// Set a per-user rate-limit override (omit a cap for unlimited at that dimension)
    SetRateLimit {
        /// Email of the user
        #[arg(long)]
        email: String,

        /// Custom daily request cap (positive; omit for unlimited daily)
        #[arg(long)]
        daily: Option<u32>,

        /// Custom monthly request cap (positive; omit for unlimited monthly)
        #[arg(long)]
        monthly: Option<u32>,

        /// Operator note recorded on the override
        #[arg(long)]
        note: Option<String>,
    },

    /// Clear a per-user rate-limit override (revert to tier defaults)
    ClearRateLimit {
        /// Email of the user
        #[arg(long)]
        email: String,
    },

    /// Set a per-user feature-flag override (wins over the tenant default)
    SetFeature {
        /// Email of the user
        #[arg(long)]
        email: String,

        /// Feature key (e.g. `api_tokens`, `billing_header`)
        #[arg(long)]
        key: String,

        /// true to enable, false to disable
        #[arg(long)]
        enabled: bool,
    },

    /// Clear a per-user feature-flag override (tenant default applies again)
    ClearFeature {
        /// Email of the user
        #[arg(long)]
        email: String,

        /// Feature key
        #[arg(long)]
        key: String,
    },

    /// List a user's per-user feature-flag overrides
    ListFeatures {
        /// Email of the user
        #[arg(long)]
        email: String,
    },
}

#[non_exhaustive]
#[derive(Subcommand)]
enum TenantCommand {
    /// Set a tenant's plan (unlocks plan-gated tools via `tool_catalog.min_plan`)
    SetPlan {
        /// Email of a user in the target tenant
        #[arg(long)]
        email: String,

        /// Plan: starter | professional | enterprise
        #[arg(long)]
        plan: String,

        /// Tenant id (required only if the user belongs to multiple tenants)
        #[arg(long)]
        tenant_id: Option<String>,
    },

    /// Force-enable an MCP tool for the whole tenant (overrides plan gating)
    EnableTool {
        /// Email of a user in the target tenant
        #[arg(long)]
        email: String,

        /// MCP tool name (must exist in the tool catalog)
        #[arg(long)]
        tool: String,

        /// Tenant id (required only if the user belongs to multiple tenants)
        #[arg(long)]
        tenant_id: Option<String>,

        /// Operator note recorded on the override
        #[arg(long)]
        reason: Option<String>,
    },

    /// Force-disable an MCP tool for the whole tenant
    DisableTool {
        /// Email of a user in the target tenant
        #[arg(long)]
        email: String,

        /// MCP tool name (must exist in the tool catalog)
        #[arg(long)]
        tool: String,

        /// Tenant id (required only if the user belongs to multiple tenants)
        #[arg(long)]
        tenant_id: Option<String>,

        /// Operator note recorded on the override
        #[arg(long)]
        reason: Option<String>,
    },

    /// Remove a tenant tool override (revert to plan/catalog default)
    ResetTool {
        /// Email of a user in the target tenant
        #[arg(long)]
        email: String,

        /// MCP tool name
        #[arg(long)]
        tool: String,

        /// Tenant id (required only if the user belongs to multiple tenants)
        #[arg(long)]
        tenant_id: Option<String>,
    },

    /// List the tenant's effective tools with each decision's source
    ListTools {
        /// Email of a user in the target tenant
        #[arg(long)]
        email: String,

        /// Tenant id (required only if the user belongs to multiple tenants)
        #[arg(long)]
        tenant_id: Option<String>,
    },

    /// Set a tenant-default feature flag (per-user overrides still win)
    SetFeature {
        /// Email of a user in the target tenant
        #[arg(long)]
        email: String,

        /// Feature key (e.g. `api_tokens`, `billing_header`)
        #[arg(long)]
        key: String,

        /// true to enable, false to disable
        #[arg(long)]
        enabled: bool,

        /// Tenant id (required only if the user belongs to multiple tenants)
        #[arg(long)]
        tenant_id: Option<String>,
    },

    /// Clear a tenant-default feature flag (built-in default applies again)
    ClearFeature {
        /// Email of a user in the target tenant
        #[arg(long)]
        email: String,

        /// Feature key
        #[arg(long)]
        key: String,

        /// Tenant id (required only if the user belongs to multiple tenants)
        #[arg(long)]
        tenant_id: Option<String>,
    },

    /// List a tenant's explicit feature-flag defaults
    ListFeatures {
        /// Email of a user in the target tenant
        #[arg(long)]
        email: String,

        /// Tenant id (required only if the user belongs to multiple tenants)
        #[arg(long)]
        tenant_id: Option<String>,
    },
}

#[non_exhaustive]
#[derive(Subcommand)]
enum ToolCommand {
    /// Force-enable an MCP tool for a user (overrides plan + tenant gating)
    Enable {
        /// Email of the user
        #[arg(long)]
        email: String,

        /// MCP tool name (must exist in the tool catalog)
        #[arg(long)]
        tool: String,

        /// Operator note recorded on the override
        #[arg(long)]
        reason: Option<String>,
    },

    /// Force-disable an MCP tool for a user
    Disable {
        /// Email of the user
        #[arg(long)]
        email: String,

        /// MCP tool name (must exist in the tool catalog)
        #[arg(long)]
        tool: String,

        /// Operator note recorded on the override
        #[arg(long)]
        reason: Option<String>,
    },

    /// Remove a per-user tool override (revert to plan/tenant/default)
    Reset {
        /// Email of the user
        #[arg(long)]
        email: String,

        /// MCP tool name
        #[arg(long)]
        tool: String,
    },

    /// List a user's per-user tool overrides
    List {
        /// Email of the user
        #[arg(long)]
        email: String,
    },
}

#[non_exhaustive]
#[derive(Subcommand)]
enum TokenCommand {
    /// Generate a new admin token
    Generate {
        /// Service name (e.g., "`pierre_admin_service`")
        #[arg(long)]
        service: String,

        /// Service description
        #[arg(long)]
        description: Option<String>,

        /// Token expiration in days (default: 365, 0 = never expires)
        #[arg(long, default_value = "365")]
        expires_days: u64,

        /// Create super admin token (never expires, all permissions)
        #[arg(long)]
        super_admin: bool,

        /// Custom permissions (comma-separated)
        #[arg(long)]
        permissions: Option<String>,

        /// Scope the token to a specific tenant (UUID)
        #[arg(long)]
        tenant_id: Option<String>,
    },

    /// List all admin tokens
    List {
        /// Include inactive tokens
        #[arg(long)]
        include_inactive: bool,

        /// Show detailed information
        #[arg(long, short = 'd')]
        detailed: bool,
    },

    /// Revoke an admin token
    Revoke {
        /// Token ID to revoke
        token_id: String,
    },

    /// Rotate an admin token (generate new token, revoke old one)
    Rotate {
        /// Token ID to rotate
        token_id: String,

        /// New expiration in days
        #[arg(long)]
        expires_days: Option<u64>,
    },

    /// Show admin token usage statistics
    Stats {
        /// Token ID (optional, shows all if omitted)
        token_id: Option<String>,

        /// Number of days to look back
        #[arg(long, default_value = "30")]
        days: u32,
    },
}

async fn dispatch_auth(action: AuthCommand) -> Result<()> {
    match action {
        AuthCommand::Login { server, no_browser } => {
            commands::auth::login(server, no_browser).await
        }
        AuthCommand::Logout => commands::auth::logout(),
        AuthCommand::Status => commands::auth::status(),
        AuthCommand::Approve {
            user_code,
            server,
            token,
        } => commands::auth::resolve(user_code, server, token, false).await,
        AuthCommand::Deny {
            user_code,
            server,
            token,
        } => commands::auth::resolve(user_code, server, token, true).await,
    }
}

async fn dispatch_strava_pool(action: StravaPoolCommand) -> Result<()> {
    match action {
        StravaPoolCommand::Add {
            client_id,
            client_secret,
            seat_cap,
            label,
        } => commands::strava_pool::add(client_id, client_secret, seat_cap, label).await,
        StravaPoolCommand::List => commands::strava_pool::list().await,
        StravaPoolCommand::Enable { client_id } => {
            commands::strava_pool::set_enabled(client_id, true).await
        }
        StravaPoolCommand::Disable { client_id } => {
            commands::strava_pool::set_enabled(client_id, false).await
        }
        StravaPoolCommand::Delete { client_id } => commands::strava_pool::delete(client_id).await,
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Initialize logging
    let log_level = if cli.verbose { "debug" } else { "info" };
    tracing_subscriber::fmt().with_env_filter(log_level).init();

    info!("Pierre MCP Server CLI");

    // Load configuration
    let database_url = cli
        .database_url
        .or_else(|| env::var("DATABASE_URL").ok())
        .unwrap_or_else(|| "sqlite:./data/users.db".into());

    // Seed commands skip the full KeyManager bootstrap because seeders only touch
    // reference data and use a zero encryption key via `init_for_seeding`.
    if let Command::Seed { action } = cli.command {
        return commands::seed::dispatch(action, &database_url).await;
    }

    // Drift check shares the same posture: only reads the coaches table,
    // never touches encrypted columns. Skip the KeyManager bootstrap so the
    // Cloud Run drift-check Job stays minimal.
    if let Command::CheckDrift { action } = cli.command {
        return commands::drift::dispatch(action, &database_url).await;
    }

    // Key operations bootstrap their own key management (the re-wrap path must
    // initialize with the source KEK), so they dispatch before the standard init.
    if let Command::Key { action } = cli.command {
        return commands::key::dispatch(action, &database_url).await;
    }

    // Auth + StravaPool are gcloud-style *remote* commands: they talk to a
    // pierre-server over HTTP with the cached device-login token and never touch
    // the local database, so they dispatch before the KeyManager/DB bootstrap.
    if let Command::Auth { action } = cli.command {
        return dispatch_auth(action).await;
    }
    if let Command::StravaPool { action } = cli.command {
        return dispatch_strava_pool(action).await;
    }

    // Initialize two-tier key management system
    let (mut key_manager, database_encryption_key) = KeyManager::bootstrap()?;
    info!("Two-tier key management system initialized for pierre-cli");

    // Initialize database
    info!("Connecting to database: {}", redact_url(&database_url));
    let mut database = Database::new(
        &database_url,
        database_encryption_key.to_vec(),
        #[cfg(feature = "postgresql")]
        &PostgresPoolConfig::default(),
    )
    .await?;

    // Complete key manager initialization (updates database's encryption key with loaded DEK)
    key_manager.complete_initialization(&mut database).await?;
    info!("Two-tier key management system fully initialized for pierre-cli");

    // Run database migrations to ensure admin_tokens table exists
    info!("Running database migrations...");
    database.migrate().await?;

    // Build repository registry for trait-object dispatch
    // Arc: the tenant tool-override commands construct a ToolSelectionService,
    // which shares the registry; everything else borrows through the Arc.
    let repos = Arc::new(database.repositories());

    // Initialize JWKS manager - loads RSA keys from database for server compatibility
    let jwks_manager = initialize_jwks_manager(&repos).await?;

    // Execute command
    match cli.command {
        Command::Seed { .. } => unreachable!("Seed is handled in the early return above"),
        Command::CheckDrift { .. } => {
            unreachable!("CheckDrift is handled in the early return above")
        }
        Command::Key { .. } => unreachable!("Key is handled in the early return above"),
        Command::Auth { .. } => unreachable!("Auth is handled in the early return above"),
        Command::StravaPool { .. } => {
            unreachable!("StravaPool is handled in the early return above")
        }
        Command::User { action } => match action {
            UserCommand::Create {
                email,
                password,
                name,
                force,
                super_admin,
            } => {
                commands::user::create(&repos, email, password, name, force, super_admin).await?;
            }
            UserCommand::Promote { email } => {
                commands::user::promote(&repos, email).await?;
            }
            UserCommand::Demote { email } => {
                commands::user::demote(&repos, email).await?;
            }
            UserCommand::ListAdmins => {
                commands::user::list_admins(&repos).await?;
            }
            UserCommand::SetTier { email, tier, note } => {
                commands::user::set_tier(&repos, email, tier, note).await?;
            }
            UserCommand::ClearTier { email } => {
                commands::user::clear_tier(&repos, email).await?;
            }
            UserCommand::Approve { email, reason } => {
                commands::user::approve(&repos, email, reason).await?;
            }
            UserCommand::Suspend { email, reason } => {
                commands::user::suspend(&repos, email, reason).await?;
            }
            UserCommand::ResetPassword { email } => {
                commands::user::reset_password(&repos, email).await?;
            }
            UserCommand::SetRateLimit {
                email,
                daily,
                monthly,
                note,
            } => {
                commands::user::set_rate_limit(&repos, email, daily, monthly, note).await?;
            }
            UserCommand::ClearRateLimit { email } => {
                commands::user::clear_rate_limit(&repos, email).await?;
            }
            UserCommand::SetFeature {
                email,
                key,
                enabled,
            } => {
                commands::user::set_feature(&repos, email, key, enabled).await?;
            }
            UserCommand::ClearFeature { email, key } => {
                commands::user::clear_feature(&repos, email, key).await?;
            }
            UserCommand::ListFeatures { email } => {
                commands::user::list_features(&repos, email).await?;
            }
        },
        Command::Tenant { action } => match action {
            TenantCommand::SetPlan {
                email,
                plan,
                tenant_id,
            } => {
                commands::tenant::set_plan(&repos, email, plan, tenant_id).await?;
            }
            TenantCommand::EnableTool {
                email,
                tool,
                tenant_id,
                reason,
            } => {
                commands::tenant::set_tool(&repos, email, tool, true, tenant_id, reason).await?;
            }
            TenantCommand::DisableTool {
                email,
                tool,
                tenant_id,
                reason,
            } => {
                commands::tenant::set_tool(&repos, email, tool, false, tenant_id, reason).await?;
            }
            TenantCommand::ResetTool {
                email,
                tool,
                tenant_id,
            } => {
                commands::tenant::reset_tool(&repos, email, tool, tenant_id).await?;
            }
            TenantCommand::ListTools { email, tenant_id } => {
                commands::tenant::list_tools(&repos, email, tenant_id).await?;
            }
            TenantCommand::SetFeature {
                email,
                key,
                enabled,
                tenant_id,
            } => {
                commands::tenant::set_feature(&repos, email, key, enabled, tenant_id).await?;
            }
            TenantCommand::ClearFeature {
                email,
                key,
                tenant_id,
            } => {
                commands::tenant::clear_feature(&repos, email, key, tenant_id).await?;
            }
            TenantCommand::ListFeatures { email, tenant_id } => {
                commands::tenant::list_features(&repos, email, tenant_id).await?;
            }
        },
        Command::Tool { action } => match action {
            ToolCommand::Enable {
                email,
                tool,
                reason,
            } => {
                commands::tool::enable(&repos, email, tool, reason).await?;
            }
            ToolCommand::Disable {
                email,
                tool,
                reason,
            } => {
                commands::tool::disable(&repos, email, tool, reason).await?;
            }
            ToolCommand::Reset { email, tool } => {
                commands::tool::reset(&repos, email, tool).await?;
            }
            ToolCommand::List { email } => {
                commands::tool::list(&repos, email).await?;
            }
        },
        Command::Token { action } => match action {
            TokenCommand::Generate {
                service,
                description,
                expires_days,
                super_admin,
                permissions,
                tenant_id,
            } => {
                commands::token::generate(
                    &repos,
                    &jwks_manager,
                    service,
                    description,
                    expires_days,
                    super_admin,
                    commands::token::GenerateTokenOpts {
                        permissions,
                        tenant_id,
                    },
                )
                .await?;
            }
            TokenCommand::List {
                include_inactive,
                detailed,
            } => {
                commands::token::list(&repos, include_inactive, detailed).await?;
            }
            TokenCommand::Revoke { token_id } => {
                commands::token::revoke(&repos, token_id).await?;
            }
            TokenCommand::Rotate {
                token_id,
                expires_days,
            } => {
                commands::token::rotate(&repos, &jwks_manager, token_id, expires_days).await?;
            }
            TokenCommand::Stats { token_id, days } => {
                commands::token::stats(&repos, token_id, days).await?;
            }
        },
        #[cfg(feature = "tools-verification")]
        Command::Harness { action } => {
            commands::harness::dispatch(action, &repos, &database).await?;
        }
    }

    Ok(())
}
