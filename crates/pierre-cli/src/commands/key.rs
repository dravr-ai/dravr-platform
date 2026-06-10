// ABOUTME: Encryption-key operator commands for pierre-cli — rotate the DEK, re-wrap under a new KEK
// ABOUTME: Wires KeyManager::rotate_dek and rewrap_deks to the CLI (ADR-017 Phase 6)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Encryption Key Sub-Command
//!
//! ```bash
//! # Rotate the Database Encryption Key (DEK) to a fresh active version.
//! # Existing data stays decryptable; new writes use the new version.
//! pierre-cli key rotate-dek
//!
//! # Re-wrap all DEK versions from the local env KEK to the configured KEK
//! # (e.g. after enabling GCP Cloud KMS via the gcp-kms feature + PIERRE_KMS_KEY_RESOURCE).
//! pierre-cli key rewrap-kek
//! ```

use clap::Subcommand;
use pierre_auth::key_management::{DatabaseEncryptionKey, KeyManager, LocalKekProvider};
#[cfg(feature = "postgresql")]
use pierre_core::config::database::PostgresPoolConfig;
use pierre_core::errors::AppResult;
use pierre_core::redaction::redact_url;
use pierre_database::backends::factory::Database;
use pierre_database::DatabaseProvider;
use tracing::info;

/// `pierre-cli key <action>` subcommand.
#[non_exhaustive]
#[derive(Subcommand)]
pub enum KeyCommand {
    /// Rotate the Database Encryption Key (DEK) to a fresh active version
    RotateDek,

    /// Re-wrap all DEK versions from the local env KEK to the configured KEK
    RewrapKek,
}

/// Dispatch a key command. Self-contained: it performs its own key-management
/// bootstrap because the re-wrap path must initialize with the *source* KEK.
pub async fn dispatch(action: KeyCommand, database_url: &str) -> AppResult<()> {
    match action {
        KeyCommand::RotateDek => rotate_dek(database_url).await,
        KeyCommand::RewrapKek => rewrap_kek(database_url).await,
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

async fn rotate_dek(database_url: &str) -> AppResult<()> {
    let (mut key_manager, database_key) = KeyManager::bootstrap()?;
    let mut database = open_database(database_url, database_key.to_vec()).await?;
    database.migrate().await?;
    key_manager.complete_initialization(&mut database).await?;

    let version = key_manager.rotate_dek(&mut database).await?;
    println!("Rotated Database Encryption Key to version {version}.");
    println!("Existing data stays decryptable; new writes use version {version}.");
    Ok(())
}

async fn rewrap_kek(database_url: &str) -> AppResult<()> {
    // Source: the historical local env KEK — stored DEKs are wrapped with it today.
    let source_kek = Box::new(LocalKekProvider::from_env()?);
    let temp_dek = DatabaseEncryptionKey::generate();
    let database_key = *temp_dek.as_bytes();
    let mut key_manager = KeyManager::with_provider(source_kek, temp_dek);

    let mut database = open_database(database_url, database_key.to_vec()).await?;
    database.migrate().await?;
    key_manager.complete_initialization(&mut database).await?;

    // Target: the configured KEK (GCP Cloud KMS when enabled + set, else local).
    let target_kek = KeyManager::configured_kek_provider()?;
    key_manager.rewrap_deks(target_kek, &mut database).await?;
    println!("Re-wrapped all DEK versions under the configured KEK.");
    Ok(())
}
