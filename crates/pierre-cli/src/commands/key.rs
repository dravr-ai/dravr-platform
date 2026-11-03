// ABOUTME: Key-rotation operator commands for pierre-cli — adopt, rotate a family, status, enable, destroy, re-wrap under a new KEK
// ABOUTME: Wires the keyset operator steps (pierre_auth::keyset) to the CLI (ADR-017 Phase 4, carnet#1347)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Encryption Key Sub-Command
//!
//! ```bash
//! # Write the keyset document from the legacy key layout (one-way).
//! pierre-cli key adopt
//!
//! # Stage a new key; the server's controller promotes it after 15 minutes,
//! # re-seals older rows, and disables the old key once nothing uses it.
//! pierre-cli key rotate --family index
//! pierre-cli key rotate --family data
//!
//! # States, ages, rows left; --history lists the lifecycle record.
//! pierre-cli key status [--history]
//!
//! # Undo a disable; destroy a key disabled for 14 days (one-way).
//! pierre-cli key enable data 1
//! pierre-cli key destroy data 1
//!
//! # Re-wrap every key version from the local env KEK to the configured KEK.
//! pierre-cli key rewrap-kek
//! ```
//!
//! On a deployed environment the database is private: run these through a
//! server-image Cloud Run job (`/app/seed-entrypoint.sh key …`), as the
//! vault runbook "Key rotation" describes.

use std::sync::Arc;

use clap::{Subcommand, ValueEnum};
use pierre_auth::key_management::{DatabaseEncryptionKey, KeyManager, LocalKekProvider};
use pierre_auth::keyset::operator::KeysetStatus;
use pierre_auth::keyset::RELOAD_PERIOD;
#[cfg(feature = "postgresql")]
use pierre_core::config::database::PostgresPoolConfig;
use pierre_core::errors::AppResult;
use pierre_core::redaction::redact_url;
use pierre_database::backends::factory::Database;
use pierre_database::backends::shared::keyring::KeyFamily;
use pierre_database::DatabaseProvider;
use pierre_services::supervised::spawn_supervised_loop;
use tracing::info;

/// A key family on the command line.
#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum FamilyArg {
    /// The AES-256-GCM keys that seal stored secrets.
    Data,
    /// The HMAC keys of the refresh-token blind index.
    Index,
    /// Both, data first.
    All,
}

impl FamilyArg {
    const fn families(self) -> &'static [KeyFamily] {
        match self {
            Self::Data => &[KeyFamily::Data],
            Self::Index => &[KeyFamily::Index],
            Self::All => &[KeyFamily::Data, KeyFamily::Index],
        }
    }
}

/// One family, for the commands that act on a single version.
#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum OneFamily {
    /// The data family.
    Data,
    /// The index family.
    Index,
}

impl From<OneFamily> for KeyFamily {
    fn from(family: OneFamily) -> Self {
        match family {
            OneFamily::Data => Self::Data,
            OneFamily::Index => Self::Index,
        }
    }
}

/// `pierre-cli key <action>` subcommand.
#[non_exhaustive]
#[derive(Subcommand)]
pub(crate) enum KeyCommand {
    /// Write the keyset document from the legacy key layout (one-way)
    Adopt,

    /// Stage a new key for a family; the server promotes it after the fleet window
    Rotate {
        /// Which family to rotate
        #[arg(long, value_enum)]
        family: FamilyArg,
    },

    /// Show every key version's state, age and the rows still under it
    Status {
        /// Also list the lifecycle record, newest first
        #[arg(long)]
        history: bool,
    },

    /// Move a disabled key back to `decrypt_only`
    Enable {
        /// The key's family
        #[arg(value_enum)]
        family: OneFamily,
        /// The key's version
        version: u32,
    },

    /// Destroy a key disabled for 14 days whose rows are all gone (one-way)
    Destroy {
        /// The key's family
        #[arg(value_enum)]
        family: OneFamily,
        /// The key's version
        version: u32,
    },

    /// Re-wrap every key version from the local env KEK to the configured KEK
    RewrapKek,
}

/// Dispatch a key command. Self-contained: it performs its own key-management
/// bootstrap because the re-wrap path must initialize with the *source* KEK.
pub(crate) async fn dispatch(action: KeyCommand, database_url: &str) -> AppResult<()> {
    if matches!(action, KeyCommand::RewrapKek) {
        return rewrap_kek(database_url).await;
    }
    let (key_manager, database) = open(database_url).await?;
    match action {
        KeyCommand::Adopt => adopt(&key_manager, &database).await,
        KeyCommand::Rotate { family } => {
            for family in family.families() {
                let version = key_manager.rotate(&database, *family).await?;
                println!(
                    "Staged {family} v{version}. The server's rotation controller promotes it after 15 minutes, then re-seals and drains the old key."
                );
            }
            Ok(())
        }
        KeyCommand::Status { history } => status(&key_manager, &database, history).await,
        KeyCommand::Enable { family, version } => {
            key_manager
                .enable(&database, family.into(), version)
                .await?;
            println!(
                "{} v{version} is decrypt_only again.",
                KeyFamily::from(family)
            );
            Ok(())
        }
        KeyCommand::Destroy { family, version } => {
            key_manager
                .destroy(&database, family.into(), version)
                .await?;
            println!("{} v{version} is destroyed.", KeyFamily::from(family));
            Ok(())
        }
        KeyCommand::RewrapKek => Ok(()),
    }
}

async fn open_database(database_url: &str, encryption_key: Vec<u8>) -> AppResult<Database> {
    info!("Connecting to database: {}", redact_url(database_url));
    Database::new(
        database_url,
        encryption_key,
        #[cfg(feature = "postgresql")]
        &PostgresPoolConfig::default(),
    )
    .await
}

/// Open the database with the configured KEK, load the keyset, and keep this
/// process's keyring current for as long as the command runs.
async fn open(database_url: &str) -> AppResult<(KeyManager, Database)> {
    let (mut key_manager, database_key) = KeyManager::bootstrap()?;
    let database = open_database(database_url, database_key.to_vec()).await?;
    database.migrate().await?;
    key_manager.complete_initialization(&database).await?;
    keep_keyring_current(&key_manager, &database);
    Ok((key_manager, database))
}

/// Re-read the keyset every [`RELOAD_PERIOD`] while the command runs, so a
/// long command keeps sealing under the current primary.
pub(crate) fn keep_keyring_current(key_manager: &KeyManager, database: &Database) {
    if let Some(loader) = key_manager.keyset_loader() {
        let database = Arc::new(database.clone());
        spawn_supervised_loop("keyset reload", RELOAD_PERIOD, move || {
            let (loader, database) = (Arc::clone(&loader), Arc::clone(&database));
            async move {
                loader
                    .refresh_or_report(database.as_security_repository())
                    .await;
            }
        });
    }
}

async fn adopt(key_manager: &KeyManager, database: &Database) -> AppResult<()> {
    match key_manager.adopt(database).await? {
        Some(doc) => {
            let primary = doc.primary(KeyFamily::Data)?;
            let older = doc.loaded(KeyFamily::Data).len().saturating_sub(1);
            println!("Adopted the legacy key layout: data v{primary} is primary, {older} older version(s) decrypt_only, index v1 primary.");
            println!("The server's rotation controller now re-seals every row under v{primary} and disables the older versions once drained.");
        }
        None => println!("The keyset is already adopted; nothing was written."),
    }
    Ok(())
}

async fn status(key_manager: &KeyManager, database: &Database, history: bool) -> AppResult<()> {
    print_status(&key_manager.status(database).await?);
    if history {
        println!();
        println!("History (newest first):");
        for event in key_manager.history(database, 100).await? {
            println!("  {}  {}", event.at.to_rfc3339(), event.message);
        }
    }
    Ok(())
}

fn print_status(status: &KeysetStatus) {
    if !status.adopted {
        println!(
            "Legacy layout (not adopted): data v1..=v{} loaded, v{} primary; index v1. Run `pierre-cli key adopt`.",
            status.legacy_active_version.unwrap_or(1),
            status.legacy_active_version.unwrap_or(1)
        );
        return;
    }
    println!("Keyset revision {}", status.revision);
    for version in &status.versions {
        let key = &version.key;
        let since = key
            .disabled_at
            .or(key.demoted_at)
            .or(key.promoted_at)
            .unwrap_or(key.created_at);
        let remaining = version
            .remaining
            .map_or_else(String::new, |n| format!(", {n} rows left"));
        let drained = key.drained_at.map_or_else(String::new, |at| {
            format!(", drained once at {}", at.to_rfc3339())
        });
        println!(
            "  {} v{}: {} since {}{remaining}{drained}",
            version.family,
            version.version,
            key.state.as_str(),
            since.to_rfc3339()
        );
    }
    println!("Rows not under the data primary:");
    for (kind, left) in &status.rows_not_under_primary {
        println!("  {}: {left}", kind.as_str());
    }
    for (holder, count) in &status.sweep.holders {
        let count = count.map_or_else(|| "count failed".to_owned(), |n| n.to_string());
        println!("External holder {holder}: {count} (as of the controller's last disable check)");
    }
    if !status.sweep.unreadable.is_empty() {
        println!("Unreadable rows (they hold their key; fix or delete each):");
        for row in &status.sweep.unreadable {
            println!("  {} {} under v{}", row.kind, row.row_key, row.version);
        }
    }
    if let Some(at) = status.sweep.last_tick_at {
        println!("Controller last ticked at {}", at.to_rfc3339());
    }
}

async fn rewrap_kek(database_url: &str) -> AppResult<()> {
    // Source: the historical local env KEK — stored keys are wrapped with it today.
    let source_kek = Box::new(LocalKekProvider::from_env()?);
    let temp_dek = DatabaseEncryptionKey::generate();
    let database_key = *temp_dek.as_bytes();
    let mut key_manager = KeyManager::with_provider(source_kek, temp_dek);

    let database = open_database(database_url, database_key.to_vec()).await?;
    database.migrate().await?;
    key_manager.complete_initialization(&database).await?;

    // Target: the configured KEK (GCP Cloud KMS when enabled + set, else local).
    let target_kek = KeyManager::configured_kek_provider()?;
    key_manager.rewrap_deks(target_kek, &database).await?;
    println!("Re-wrapped every key version under the configured KEK.");
    Ok(())
}
