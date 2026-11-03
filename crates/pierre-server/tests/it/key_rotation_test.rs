// ABOUTME: Key rotation end to end — fresh boot, legacy adoption, sweep, drain, disable, index rotation, the guards
// ABOUTME: Drives the controller's tick with a moved clock; runs on SQLite and, on the PG lane, on Postgres

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Integration tests for the keyset (ADR-017 Phase 4, carnet#1347).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use pierre_auth::key_management::{DatabaseEncryptionKey, KekProvider, KeyManager};
use pierre_auth::keyset::secrets::{insert_key_if_absent, ACTIVE_VERSION_SECRET};
use pierre_auth::keyset::{
    read_keyset, CiphertextHolder, KeyState, KeysetController, SweepState, KEYSET_SECRET,
};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::SessionRefreshToken;
use pierre_database::backends::factory::{Database, DatabaseBackend};
use pierre_database::backends::shared::encryption::create_rsa_key_aad_context;
use pierre_database::backends::shared::key_material::StoredSecretKind;
use pierre_database::backends::shared::keyring::{KeyFamily, SealDomain};
use pierre_database::backends::SecurityRepository;
use pierre_test_support::db::create_test_db_with_key;
use serial_test::serial;

use crate::common::{self, bootstrap_keys};

const TEST_MEK: [u8; 32] = [b'r'; 32];
const POOL_APP: &str = "keyset-pool-app";
const POOL_SECRET: &str = "pool-client-secret";

/// A booted database and the manager that loaded its keyset.
struct Booted {
    database: Database,
    key_manager: KeyManager,
}

impl Booted {
    fn security(&self) -> &dyn SecurityRepository {
        self.database.as_security_repository()
    }

    fn controller(&self, holders: Vec<Arc<dyn CiphertextHolder>>) -> KeysetController {
        KeysetController::new(
            self.key_manager.keyset_loader().unwrap(),
            self.key_manager.kek(),
            holders,
            0,
        )
    }

    async fn state(&self, family: KeyFamily, version: u32) -> KeyState {
        let (_, doc) = read_keyset(self.security()).await.unwrap().unwrap();
        doc.versions(family)[&version].state
    }
}

async fn boot() -> Booted {
    let (mut key_manager, database_key) = bootstrap_keys(TEST_MEK);
    let database = create_test_db_with_key(database_key.to_vec())
        .await
        .unwrap();
    key_manager
        .complete_initialization(&database)
        .await
        .unwrap();
    Booted {
        database,
        key_manager,
    }
}

/// Boot again over the same database, as a restarted instance does.
async fn reboot(booted: Booted) -> Booted {
    let (mut key_manager, _) = bootstrap_keys(TEST_MEK);
    key_manager
        .complete_initialization(&booted.database)
        .await
        .unwrap();
    Booted {
        database: booted.database,
        key_manager,
    }
}

/// Turn a fresh database into the legacy layout at `active` versions: no
/// keyset document, `active - 1` extra data keys, the active-version secret.
async fn make_legacy(booted: &Booted, active: u32) {
    let security = booted.security();
    security.delete_system_secret(KEYSET_SECRET).await.unwrap();
    let kek: Arc<dyn KekProvider> = booted.key_manager.kek();
    for version in 2..=active {
        let key = DatabaseEncryptionKey::generate();
        let wrapped = kek.wrap(key.as_bytes()).await.unwrap();
        assert!(
            insert_key_if_absent(security, KeyFamily::Data, version, &wrapped)
                .await
                .unwrap()
        );
    }
    security
        .update_system_secret(ACTIVE_VERSION_SECRET, &active.to_string())
        .await
        .unwrap();
}

async fn stored_pool_secret(database: &Database) -> String {
    sql_text(
        database,
        "SELECT client_secret_encrypted AS v FROM strava_oauth_app_pool WHERE client_id = $1",
        POOL_APP,
    )
    .await
}

async fn stored_private_key(database: &Database, kid: &str) -> String {
    sql_text(
        database,
        "SELECT private_key_pem AS v FROM rsa_keypairs WHERE kid = $1",
        kid,
    )
    .await
}

/// One text column of one row, by a key, on whichever backend is open.
async fn sql_text(database: &Database, sql: &str, key: &str) -> String {
    use sqlx::Row;
    match database.backend() {
        DatabaseBackend::SQLite(db) => sqlx::query(sql)
            .bind(key)
            .fetch_one(db.pool())
            .await
            .unwrap()
            .get("v"),
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(db) => sqlx::query(sql)
            .bind(key)
            .fetch_one(db.pool())
            .await
            .unwrap()
            .get("v"),
    }
}

/// Run a two-bind statement on whichever backend is open.
async fn exec(database: &Database, sql: &str, value: &str, key: &str) {
    match database.backend() {
        DatabaseBackend::SQLite(db) => {
            sqlx::query(sql)
                .bind(value)
                .bind(key)
                .execute(db.pool())
                .await
                .unwrap();
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(db) => {
            sqlx::query(sql)
                .bind(value)
                .bind(key)
                .execute(db.pool())
                .await
                .unwrap();
        }
    }
}

/// Tick the controller at `now` and return the report.
async fn tick(booted: &Booted, controller: &KeysetController, now: DateTime<Utc>) {
    controller.tick(booted.security(), now).await.unwrap();
}

#[tokio::test]
#[serial]
async fn a_fresh_database_writes_its_keyset_at_boot_and_reboots_from_it() {
    let booted = boot().await;
    let (_, doc) = read_keyset(booted.security()).await.unwrap().unwrap();
    assert_eq!(doc.primary(KeyFamily::Data).unwrap(), 1);
    assert_eq!(doc.primary(KeyFamily::Index).unwrap(), 1);
    assert!(booted.database.keyring().current().adopted());

    let sealed = booted
        .security()
        .encrypt_data_with_aad(
            SealDomain::Stored(StoredSecretKind::StravaPoolClientSecret),
            "secret",
            "aad",
        )
        .unwrap();
    let rebooted = reboot(booted).await;
    assert_eq!(
        rebooted
            .security()
            .decrypt_data_with_aad(
                SealDomain::Stored(StoredSecretKind::StravaPoolClientSecret),
                &sealed,
                "aad"
            )
            .unwrap(),
        "secret"
    );
}

#[tokio::test]
#[serial]
async fn a_legacy_database_runs_as_before_until_adopted() {
    let booted = boot().await;
    make_legacy(&booted, 2).await;
    let legacy = reboot(booted).await;
    let ring = legacy.database.keyring().current();
    assert!(!ring.adopted(), "no document means the legacy layout");
    assert_eq!(ring.data_primary(), 2);
    assert_eq!(ring.versions(KeyFamily::Data), vec![1, 2]);

    // The controller does nothing before adoption.
    let controller = legacy.controller(Vec::new());
    let report = controller
        .tick(legacy.security(), Utc::now() + Duration::days(400))
        .await
        .unwrap();
    assert_eq!(report.transitions, Vec::new());
    assert_eq!(report.resealed, 0);
    assert!(read_keyset(legacy.security()).await.unwrap().is_none());

    // Adoption is the operator's, and happens once.
    let doc = legacy
        .key_manager
        .adopt(&legacy.database)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(doc.primary(KeyFamily::Data).unwrap(), 2);
    assert_eq!(
        legacy.state(KeyFamily::Data, 1).await,
        KeyState::DecryptOnly
    );
    assert!(legacy
        .key_manager
        .adopt(&legacy.database)
        .await
        .unwrap()
        .is_none());
    let gone = legacy
        .security()
        .get_system_secret(ACTIVE_VERSION_SECRET)
        .await;
    assert!(
        gone.is_err(),
        "adoption deletes the legacy active-version secret"
    );
    assert!(legacy.database.keyring().current().adopted());
}

#[tokio::test]
#[serial]
async fn adoption_sweeps_every_row_to_the_primary_then_disables_the_old_key() {
    let booted = boot().await;
    // Rows written under v1: a pool secret, a sealed RSA key, a plaintext one.
    let repos = Arc::clone(booted.database.repositories());
    repos
        .oauth_tokens
        .upsert_strava_pool_app(POOL_APP, POOL_SECRET, 10, Some("keyset"))
        .await
        .unwrap();
    let security = booted.security();
    security
        .save_rsa_keypair(
            "kid-sealed",
            "-----BEGIN PRIVATE KEY-----\nAAAA",
            "pub",
            Utc::now(),
            false,
            2048,
        )
        .await
        .unwrap();
    security
        .save_rsa_keypair("kid-plain", "placeholder", "pub", Utc::now(), false, 2048)
        .await
        .unwrap();
    exec(
        &booted.database,
        "UPDATE rsa_keypairs SET private_key_pem = $1 WHERE kid = $2",
        "-----BEGIN PRIVATE KEY-----\nBBBB",
        "kid-plain",
    )
    .await;
    assert!(stored_pool_secret(&booted.database)
        .await
        .starts_with("v1:"));

    make_legacy(&booted, 2).await;
    let legacy = reboot(booted).await;
    legacy
        .key_manager
        .adopt(&legacy.database)
        .await
        .unwrap()
        .unwrap();
    let controller = legacy.controller(Vec::new());
    let now = Utc::now();

    tick(&legacy, &controller, now).await;
    assert!(stored_pool_secret(&legacy.database)
        .await
        .starts_with("v2:"));
    assert!(stored_private_key(&legacy.database, "kid-sealed")
        .await
        .starts_with("v2:"));
    assert!(
        stored_private_key(&legacy.database, "kid-plain")
            .await
            .starts_with("v2:"),
        "a legacy plaintext key is sealed by the sweep"
    );
    assert_eq!(
        legacy
            .security()
            .count_sealed_under(StoredSecretKind::RsaPrivateKey, 1)
            .await
            .unwrap(),
        0
    );
    let pem = legacy
        .security()
        .decrypt_data_with_aad(
            SealDomain::Stored(StoredSecretKind::RsaPrivateKey),
            &stored_private_key(&legacy.database, "kid-plain").await,
            &create_rsa_key_aad_context("kid-plain"),
        )
        .unwrap();
    assert_eq!(pem, "-----BEGIN PRIVATE KEY-----\nBBBB");

    // Disabling waits L + the sign-in cookie's TTL, then two zero counts.
    tick(&legacy, &controller, now + Duration::minutes(20)).await;
    assert_eq!(
        legacy.state(KeyFamily::Data, 1).await,
        KeyState::DecryptOnly
    );
    tick(&legacy, &controller, now + Duration::minutes(26)).await;
    assert_eq!(
        legacy.state(KeyFamily::Data, 1).await,
        KeyState::DecryptOnly,
        "one zero is not enough"
    );
    tick(&legacy, &controller, now + Duration::minutes(31)).await;
    assert_eq!(legacy.state(KeyFamily::Data, 1).await, KeyState::Disabled);
    assert_eq!(
        legacy
            .database
            .keyring()
            .current()
            .versions(KeyFamily::Data),
        vec![2]
    );

    // Everything still opens, and index v1 (the same row) still hashes.
    assert_eq!(
        repos_of(&legacy)
            .oauth_tokens
            .get_strava_pool_app_secret(POOL_APP)
            .await
            .unwrap()
            .as_deref(),
        Some(POOL_SECRET)
    );
    assert_eq!(legacy.state(KeyFamily::Index, 1).await, KeyState::Primary);

    // Destroy is refused before 14 days; enable undoes the disable.
    assert!(legacy
        .key_manager
        .destroy(&legacy.database, KeyFamily::Data, 1)
        .await
        .is_err());
    legacy
        .key_manager
        .enable(&legacy.database, KeyFamily::Data, 1)
        .await
        .unwrap();
    assert_eq!(
        legacy.state(KeyFamily::Data, 1).await,
        KeyState::DecryptOnly
    );

    let history = legacy
        .key_manager
        .history(&legacy.database, 50)
        .await
        .unwrap();
    let messages: Vec<&str> = history.iter().map(|e| e.message.as_str()).collect();
    assert!(messages.contains(&"data v1 disabled"), "{messages:?}");
    assert!(messages.contains(&"data v1 enabled"), "{messages:?}");
    assert!(
        messages.contains(&"data v2 adopted as primary"),
        "{messages:?}"
    );
}

fn repos_of(booted: &Booted) -> Arc<pierre_database::RepositoryRegistry> {
    Arc::clone(booted.database.repositories())
}

#[tokio::test]
#[serial]
async fn a_row_that_does_not_open_is_reported_and_holds_its_key() {
    let booted = boot().await;
    repos_of(&booted)
        .oauth_tokens
        .upsert_strava_pool_app(POOL_APP, POOL_SECRET, 10, None)
        .await
        .unwrap();
    // Tampered under v1: it no longer authenticates.
    exec(
        &booted.database,
        "UPDATE strava_oauth_app_pool SET client_secret_encrypted = $1 WHERE client_id = $2",
        "v1:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        POOL_APP,
    )
    .await;
    booted
        .key_manager
        .rotate(&booted.database, KeyFamily::Data)
        .await
        .unwrap();
    let controller = booted.controller(Vec::new());
    let now = Utc::now() + Duration::minutes(16);
    tick(&booted, &controller, now).await;
    assert_eq!(booted.state(KeyFamily::Data, 2).await, KeyState::Primary);
    let state = SweepState::read(booted.security()).await.unwrap();
    assert_eq!(state.unreadable_under(1), 1);

    for minutes in [30, 40, 50] {
        tick(&booted, &controller, now + Duration::minutes(minutes)).await;
    }
    assert_eq!(
        booted.state(KeyFamily::Data, 1).await,
        KeyState::DecryptOnly
    );

    // The operator deletes the row; the next complete pass clears it.
    exec(
        &booted.database,
        "DELETE FROM strava_oauth_app_pool WHERE client_secret_encrypted = $1 AND client_id = $2",
        "v1:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        POOL_APP,
    )
    .await;
    for minutes in [60, 65] {
        tick(&booted, &controller, now + Duration::minutes(minutes)).await;
    }
    assert_eq!(booted.state(KeyFamily::Data, 1).await, KeyState::Disabled);
}

struct FailingHolder;

#[async_trait]
impl CiphertextHolder for FailingHolder {
    fn name(&self) -> &'static str {
        "failing"
    }
    async fn count_sealed_before(&self, _before: DateTime<Utc>) -> AppResult<u64> {
        Err(AppError::internal("store unreachable"))
    }
}

#[tokio::test]
#[serial]
async fn an_external_holder_whose_count_fails_holds_the_key() {
    let booted = boot().await;
    booted
        .key_manager
        .rotate(&booted.database, KeyFamily::Data)
        .await
        .unwrap();
    let controller = booted.controller(vec![Arc::new(FailingHolder)]);
    let now = Utc::now() + Duration::minutes(16);
    for minutes in [0, 30, 40, 50] {
        tick(&booted, &controller, now + Duration::minutes(minutes)).await;
    }
    assert_eq!(
        booted.state(KeyFamily::Data, 1).await,
        KeyState::DecryptOnly
    );
    let state = SweepState::read(booted.security()).await.unwrap();
    assert_eq!(state.holders.get("failing"), Some(&None));
}

#[tokio::test]
#[serial]
async fn an_index_rotation_signs_no_one_out() {
    let booted = boot().await;
    let shared = Arc::new(booted.database.clone());
    let (user_id, _) = common::create_test_user(&shared).await.unwrap();
    let repos = repos_of(&booted);
    let record = |family: &str| SessionRefreshToken {
        family_id: family.to_owned(),
        user_id,
        tenant_id: None,
        client_id: None,
        created_at: Utc::now(),
        expires_at: Utc::now() + Duration::days(30),
    };
    repos
        .session_refresh_tokens
        .store_token("before-staging", &record("f1"))
        .await
        .unwrap();
    repos
        .session_refresh_tokens
        .store_token("before-promotion", &record("f2"))
        .await
        .unwrap();

    booted
        .key_manager
        .rotate(&booted.database, KeyFamily::Index)
        .await
        .unwrap();
    assert!(
        booted
            .key_manager
            .rotate(&booted.database, KeyFamily::Index)
            .await
            .is_err(),
        "one rotation per family at a time"
    );
    // Staged: writes stay on k1, lookups try both.
    assert!(repos
        .session_refresh_tokens
        .consume_token("before-staging", Utc::now())
        .await
        .unwrap()
        .is_some());

    let controller = booted.controller(Vec::new());
    tick(&booted, &controller, Utc::now() + Duration::minutes(16)).await;
    assert_eq!(booted.state(KeyFamily::Index, 2).await, KeyState::Primary);

    // A token minted under k1 still exchanges after k2 is primary, and its
    // successor is hashed under k2.
    assert!(repos
        .session_refresh_tokens
        .consume_token("before-promotion", Utc::now())
        .await
        .unwrap()
        .is_some());
    repos
        .session_refresh_tokens
        .store_token("after", &record("f3"))
        .await
        .unwrap();
    let hash = booted.database.keyring().hash_for_storage("after").unwrap();
    assert!(hash.starts_with("k2:"));
    assert!(repos
        .session_refresh_tokens
        .consume_token("after", Utc::now())
        .await
        .unwrap()
        .is_some());

    // k1 keeps live rows (revoked ones included) until they expire.
    assert!(
        booted
            .security()
            .count_live_index_hashes(1, Utc::now())
            .await
            .unwrap()
            >= 2
    );
    for minutes in [45, 50, 55] {
        tick(
            &booted,
            &controller,
            Utc::now() + Duration::minutes(minutes),
        )
        .await;
    }
    assert_eq!(
        booted.state(KeyFamily::Index, 1).await,
        KeyState::DecryptOnly
    );
}
