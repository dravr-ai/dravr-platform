// ABOUTME: A database seeded with ciphertext before any server boot still boots: the seeder stores the signing keypair first
// ABOUTME: Runs `pierre-cli seed synthetic-activities` on a fresh database, then loads the keypair the way a booting server does

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A server refuses to mint a JWT signing keypair on a database that already
//! holds ciphertext (carnet#703): an empty `rsa_keypairs` table beside
//! encrypted rows is a lost keypair. The seeders that write an encrypted
//! token therefore load or store the keypair before seeding, as a booting
//! server does, so a database seeded before its first boot is not mistaken
//! for one that lost its key.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::env;
use std::fs;
use std::process::Command;

use pierre_auth::admin::jwks::{load_or_store_first_keypair, RSA_KEY_SIZE};
use pierre_auth::key_management::KeyManager;
#[cfg(feature = "postgresql")]
use pierre_core::config::database::PostgresPoolConfig;
use pierre_core::models::{Tenant, User};
use pierre_database::backends::{factory::Database, DatabaseProvider};
use uuid::Uuid;

/// Base64 of 32 fixed bytes — the local KEK both this process and the binary use.
const MASTER_KEY: &str = "MDEyMzQ1Njc4OWFiY2RlZjAxMjM0NTY3ODlhYmNkZWY=";

fn cli_binary() -> String {
    env::var("CARGO_BIN_EXE_pierre-cli")
        .unwrap_or_else(|_| env!("CARGO_BIN_EXE_pierre-cli").to_owned())
}

/// Open the database with the key hierarchy the binary uses.
async fn open_database(url: &str) -> Database {
    env::set_var("PIERRE_MASTER_ENCRYPTION_KEY", MASTER_KEY);
    let (mut key_manager, key) = KeyManager::bootstrap().unwrap();
    let mut database = Database::new(
        url,
        key.to_vec(),
        #[cfg(feature = "postgresql")]
        &PostgresPoolConfig::default(),
    )
    .await
    .unwrap();
    key_manager
        .complete_initialization(&mut database)
        .await
        .unwrap();
    database.migrate().await.unwrap();
    database
}

#[tokio::test]
async fn a_database_seeded_with_ciphertext_before_its_first_boot_boots() {
    let path = env::temp_dir().join(format!("pierre-cli-seed-boot-{}.db", Uuid::new_v4()));
    let url = format!("sqlite:{}?mode=rwc", path.display());
    let database = open_database(&url).await;
    let repos = database.repositories();

    // A user with a tenant, and no signing keypair yet: nothing has booted.
    let email = format!("{}@test.local", Uuid::new_v4());
    let user = User::new(email.clone(), "hash".to_owned(), None);
    repos.users.create(&user).await.unwrap();
    let tenant = Tenant::new(
        "Seed Tenant".to_owned(),
        format!("tenant-{}", Uuid::new_v4()),
        None,
        "starter".to_owned(),
        user.id,
    );
    repos.tenants.create(&tenant).await.unwrap();
    repos
        .users
        .update_tenant_id(user.id, tenant.id)
        .await
        .unwrap();
    let security = database.as_security_repository();
    assert!(security.load_rsa_keypairs().await.unwrap().is_empty());
    assert_eq!(security.count_encrypted_rows().await.unwrap(), 0);

    let out = Command::new(cli_binary())
        .env("DATABASE_URL", &url)
        .env("PIERRE_MASTER_ENCRYPTION_KEY", MASTER_KEY)
        .args([
            "seed",
            "synthetic-activities",
            "--email",
            &email,
            "--provider",
            "strava",
            "--count",
            "2",
            "--days",
            "2",
            "--seed",
            "703",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "seed failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    // The seeder wrote an encrypted token and, before it, the keypair.
    let stored = security.load_rsa_keypairs().await.unwrap();
    assert_eq!(stored.len(), 1, "the seeder stores the first keypair");
    assert!(
        repos
            .oauth_tokens
            .get_token(user.id, tenant.id, "strava")
            .await
            .unwrap()
            .is_some(),
        "the seeder stored the encrypted provider token"
    );

    // A server booting now loads the seeder's keypair instead of refusing.
    let jwks = load_or_store_first_keypair(security, RSA_KEY_SIZE)
        .await
        .expect("a database seeded before its first boot must boot");
    assert_eq!(jwks.get_active_key().unwrap().kid, stored[0].0);

    drop(database);
    let _ = fs::remove_file(&path);
}
