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
use pierre_database::plugins::{factory::Database, DatabaseProvider};
#[cfg(feature = "postgresql")]
use pierre_mcp_server::config::environment::PostgresPoolConfig;
use pierre_mcp_server::errors::AppResult;

type Result<T> = AppResult<T>;
use std::env;
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

    /// Coaching harness operator commands (Tier 5.5 backfill, etc.)
    #[cfg(feature = "tools-verification")]
    Harness {
        #[command(subcommand)]
        action: commands::harness::HarnessCommand,
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

    // Initialize two-tier key management system
    let (mut key_manager, database_encryption_key) = KeyManager::bootstrap()?;
    info!("Two-tier key management system initialized for pierre-cli");

    // Initialize database
    info!("Connecting to database: {}", database_url);
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
    let repos = database.repositories();

    // Initialize JWKS manager - loads RSA keys from database for server compatibility
    let jwks_manager = initialize_jwks_manager(&repos).await?;

    // Execute command
    match cli.command {
        Command::Seed { .. } => unreachable!("Seed is handled in the early return above"),
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
        },
        Command::Token { action } => match action {
            TokenCommand::Generate {
                service,
                description,
                expires_days,
                super_admin,
                permissions,
            } => {
                commands::token::generate(
                    &repos,
                    &jwks_manager,
                    service,
                    description,
                    expires_days,
                    super_admin,
                    permissions,
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
