// ABOUTME: Every key-material failure logs at ERROR under a key_material.* event, once per incident, never with a secret
// ABOUTME: Covers stored-secret decrypt failures per kind, DEK read/unwrap failures, boots that refuse to mint a key over ciphertext, and the alert filter
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! # Key-material failures page (carnet#703)
//!
//! On 2026-09-30 the stored DEK was replaced (carnet#696) and every stored
//! secret stopped decrypting. The 133 failures were logged at `WARN`, the DEK
//! minted over a populated database at `INFO`, and `dravr-tronc`'s error
//! notifier, which forwards `ERROR` only, never fired.
//!
//! Each test drives one failure through its production path and asserts the
//! level, the `event` field the Cloud Monitoring metric matches, and that no
//! secret reaches a log field. A decrypt failure that repeats alerts once and
//! repeats at `WARN` under the same event.
//!
//! The stored-secret alert latch is per process and per kind, and every test
//! binary is its own process: each kind is failed by exactly one test here,
//! so the tests stay independent when they run in parallel.

use std::collections::HashMap;
use std::fmt::Debug as FmtDebug;
use std::sync::{Arc, Mutex};

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use chrono::{Duration as ChronoDuration, Utc};
use pierre_auth::admin::jwks::load_or_store_first_keypair;
use pierre_auth::key_management::{
    DatabaseEncryptionKey, KekProvider, KeyManager, LocalKekProvider, MasterEncryptionKey,
};
use pierre_auth::tenant::llm_manager::{LlmProvider, StoreLlmCredentialsRequest, TenantLlmManager};
use pierre_core::errors::ErrorCode;
use pierre_core::models::{Tenant, TenantId, TenantOAuthCredentials, UserOAuthToken};
use pierre_database::backends::factory::{Database, DatabaseBackend};
use pierre_test_support::db::create_test_db_with_key;
use pierre_test_support::server::create_test_user;
use regex::Regex;
use tracing::field::{Field, Visit};
use tracing::subscriber::DefaultGuard;
use tracing::{Level, Subscriber};
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::Layer;
use uuid::Uuid;

/// The key every fixture database is written under.
const WRITE_KEY: [u8; 32] = [3u8; 32];
/// A different key: a database handle holding it cannot open what
/// [`WRITE_KEY`] sealed, the shape of the carnet#696 incident.
const WRONG_KEY: [u8; 32] = [9u8; 32];
/// RSA size for generated signing keys; small to keep debug builds fast.
const TEST_RSA_BITS: usize = 2048;
/// The incident's count of decrypt failures.
const INCIDENT_FAILURES: usize = 133;
/// Plaintext secrets the fixtures store; none may appear in a log field.
const ACCESS_TOKEN: &str = "plaintext-access-token-703";
const CLIENT_SECRET: &str = "plaintext-client-secret-703";
const LLM_API_KEY: &str = "plaintext-llm-api-key-703";

// ── Event capture ────────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
struct CapturedEvent {
    level: Level,
    message: String,
    fields: HashMap<String, String>,
}

impl CapturedEvent {
    fn field(&self, name: &str) -> &str {
        self.fields
            .get(name)
            .unwrap_or_else(|| panic!("event has no {name} field: {self:#?}"))
    }
}

#[derive(Clone, Default)]
struct CaptureLayer {
    events: Arc<Mutex<Vec<CapturedEvent>>>,
}

#[derive(Default)]
struct FieldVisitor {
    message: String,
    fields: HashMap<String, String>,
}

impl Visit for FieldVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn FmtDebug) {
        let rendered = format!("{value:?}");
        if field.name() == "message" {
            self.message.clone_from(&rendered);
        }
        self.fields.insert(field.name().to_owned(), rendered);
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            value.clone_into(&mut self.message);
        }
        self.fields
            .insert(field.name().to_owned(), value.to_owned());
    }
}

impl<S> Layer<S> for CaptureLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let mut visitor = FieldVisitor::default();
        event.record(&mut visitor);
        self.events.lock().unwrap().push(CapturedEvent {
            level: *event.metadata().level(),
            message: visitor.message,
            fields: visitor.fields,
        });
    }
}

/// Every event emitted on this thread while the guard lives. The tests run
/// on tokio's current-thread runtime, so every await stays on this thread.
struct Captured {
    events: Arc<Mutex<Vec<CapturedEvent>>>,
    _guard: DefaultGuard,
}

impl Captured {
    fn start() -> Self {
        let layer = CaptureLayer::default();
        let events = Arc::clone(&layer.events);
        let guard = tracing_subscriber::registry().with(layer).set_default();
        Self {
            events,
            _guard: guard,
        }
    }

    fn all(&self) -> Vec<CapturedEvent> {
        self.events.lock().unwrap().clone()
    }

    /// Events carrying `event = <name>` at `level`.
    fn with_event(&self, name: &str, level: Level) -> Vec<CapturedEvent> {
        self.all()
            .into_iter()
            .filter(|e| e.level == level && e.fields.get("event").map(String::as_str) == Some(name))
            .collect()
    }

    /// The single ERROR carrying `event = <name>`, after checking the event
    /// matches the alert filter.
    fn page(&self, name: &str) -> CapturedEvent {
        assert_alert_filter_matches(name);
        let matching = self.with_event(name, Level::ERROR);
        assert_eq!(
            matching.len(),
            1,
            "expected exactly one ERROR with event {name:?}, got: {:#?}",
            self.all()
        );
        matching.into_iter().next().unwrap()
    }

    /// Fails when anything was logged at ERROR.
    fn assert_nothing_paged(&self) {
        let errors: Vec<_> = self
            .all()
            .into_iter()
            .filter(|e| e.level == Level::ERROR)
            .collect();
        assert!(errors.is_empty(), "nothing may page here: {errors:#?}");
    }

    /// Fails when any captured field or message carries `secret`.
    fn assert_never_logged(&self, secret: &str) {
        for event in self.all() {
            assert!(
                !event.message.contains(secret)
                    && event.fields.values().all(|v| !v.contains(secret)),
                "a secret reached a log line: {event:#?}"
            );
        }
    }
}

// ── The alert contract ───────────────────────────────────────────────────────

/// The `jsonPayload.event` regex the Cloud Monitoring metric filters on.
///
/// This reads the Terraform source on purpose: the log line and the alert
/// filter live on two sides of a boundary no test can cross (the metric runs
/// in Cloud Logging), and a renamed event would otherwise stop paging in
/// silence (docs/coding-standards.md, Tests).
fn alert_filter() -> Regex {
    let terraform = include_str!("../../../infra/environments/dev/key_material_monitoring.tf");
    let pattern = Regex::new(r#"(?m)^\s*jsonPayload\.event=~"([^"]+)"\s*$"#).unwrap();
    let captures = pattern
        .captures(terraform)
        .expect("key_material_monitoring.tf must filter the metric on jsonPayload.event");
    Regex::new(&captures[1]).unwrap()
}

fn assert_alert_filter_matches(event: &str) {
    assert!(
        alert_filter().is_match(event),
        "event {event:?} does not match the key-material alert filter, so it would never page"
    );
}

// ── Fixtures ─────────────────────────────────────────────────────────────────

/// A handle on the same store holding [`WRONG_KEY`]: every ciphertext the
/// fixture wrote fails to open through it.
fn with_wrong_key(database: &Database) -> Database {
    let mut wrong = database.clone();
    wrong.install_dek_versions(1, WRONG_KEY.to_vec(), HashMap::new());
    wrong
}

/// A user who owns a tenant, both stored.
async fn user_with_tenant(database: &Database) -> (Uuid, TenantId) {
    let user = create_test_user(&format!("km-{}@example.com", Uuid::new_v4()), None);
    let user_id = database.repositories().users.create(&user).await.unwrap();
    let tenant_id = TenantId::generate();
    let now = Utc::now();
    database
        .repositories()
        .tenants
        .create(&Tenant {
            id: tenant_id,
            name: "Key material".to_owned(),
            slug: tenant_id.to_string(),
            domain: None,
            plan: "professional".to_owned(),
            owner_user_id: user_id,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
    database
        .repositories()
        .users
        .update_tenant_id(user_id, tenant_id)
        .await
        .unwrap();
    (user_id, tenant_id)
}

/// Store a Strava token for a fresh user; returns the user and tenant.
async fn store_oauth_token(database: &Database) -> (Uuid, TenantId) {
    let (user_id, tenant_id) = user_with_tenant(database).await;
    let token = UserOAuthToken::new(
        user_id,
        tenant_id.to_string(),
        "strava".to_owned(),
        ACCESS_TOKEN.to_owned(),
        Some(format!("refresh-{ACCESS_TOKEN}")),
        Some(Utc::now() + ChronoDuration::hours(6)),
        Some("read".to_owned()),
    );
    database
        .repositories()
        .oauth_tokens
        .upsert_token(&token)
        .await
        .unwrap();
    (user_id, tenant_id)
}

fn kek(byte: u8) -> LocalKekProvider {
    LocalKekProvider::from_mek(MasterEncryptionKey::from_bytes([byte; 32]))
}

/// A key manager as a booting instance builds it.
fn booting_manager(kek_byte: u8) -> KeyManager {
    KeyManager::with_provider(Box::new(kek(kek_byte)), DatabaseEncryptionKey::generate())
}

/// Make reads of the stored `secret_type` row fail while the row stays in
/// place (the technique of `dek_initialization_race_test`).
async fn break_reads(database: &Database, secret_type: &str) {
    match database.backend() {
        DatabaseBackend::SQLite(sqlite) => {
            sqlx::query(
                "UPDATE system_secrets SET secret_value = CAST(secret_value AS BLOB) \
                 WHERE secret_type = $1",
            )
            .bind(secret_type)
            .execute(sqlite.pool())
            .await
            .unwrap();
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(pg) => {
            sqlx::query(
                "ALTER TABLE system_secrets RENAME COLUMN secret_value TO secret_value_hidden",
            )
            .execute(pg.pool())
            .await
            .unwrap();
        }
    }
}

// ── Stored-secret decrypt failures ───────────────────────────────────────────

#[tokio::test]
async fn oauth_token_decrypt_failures_page_once_then_repeat_at_warn() {
    let database = create_test_db_with_key(WRITE_KEY.to_vec()).await.unwrap();
    let (user_id, tenant_id) = store_oauth_token(&database).await;
    let wrong = with_wrong_key(&database);

    let captured = Captured::start();
    for _ in 0..INCIDENT_FAILURES {
        let read = wrong
            .repositories()
            .oauth_tokens
            .get_token(user_id, tenant_id, "strava")
            .await;
        assert!(
            read.is_err(),
            "a token sealed under another key must not open"
        );
    }

    let page = captured.page("key_material.stored_secret_decrypt_failed");
    // The kind leads the message: the error notifier dedups on target plus
    // the message's first 80 characters, so a fixed message would fold every
    // kind into the first one's Slack line.
    assert_eq!(page.message, "Stored oauth_access_token failed to decrypt");
    assert_eq!(page.field("secret_kind"), "oauth_access_token");
    assert_eq!(page.field("table"), "user_oauth_tokens");
    assert_eq!(page.field("column"), "access_token");
    assert_eq!(page.field("dek_version"), "1");
    assert_eq!(page.field("repeats_since_last_alert"), "0");
    let aad = page.field("aad_context");
    assert!(
        aad.contains(&tenant_id.to_string()) && aad.contains(&user_id.to_string()),
        "the event must name the row it failed on: {aad}"
    );
    assert!(aad.contains("strava"));

    let repeats = captured.with_event("key_material.stored_secret_decrypt_failed", Level::WARN);
    assert_eq!(
        repeats.len(),
        INCIDENT_FAILURES - 1,
        "every repeat inside the window logs at WARN under the same event"
    );
    assert!(repeats
        .iter()
        .all(|e| e.field("secret_kind") == "oauth_access_token"));
    captured.assert_never_logged(ACCESS_TOKEN);
}

#[tokio::test]
async fn tenant_oauth_client_secret_decrypt_failure_pages() {
    let database = create_test_db_with_key(WRITE_KEY.to_vec()).await.unwrap();
    let (_, tenant_id) = user_with_tenant(&database).await;
    database
        .repositories()
        .tenants
        .store_oauth_credentials(&TenantOAuthCredentials {
            tenant_id,
            provider: "strava".to_owned(),
            client_id: "703703".to_owned(),
            client_secret: CLIENT_SECRET.to_owned(),
            redirect_uri: "http://localhost:8081/api/oauth/callback/strava".to_owned(),
            scopes: vec!["read".to_owned()],
            rate_limit_per_day: 1000,
        })
        .await
        .unwrap();
    let wrong = with_wrong_key(&database);

    let captured = Captured::start();
    let read = wrong
        .repositories()
        .tenants
        .get_oauth_credentials(tenant_id, "strava")
        .await;
    assert!(read.is_err());

    let page = captured.page("key_material.stored_secret_decrypt_failed");
    assert_eq!(
        page.message,
        "Stored tenant_oauth_client_secret failed to decrypt"
    );
    assert_eq!(page.field("secret_kind"), "tenant_oauth_client_secret");
    assert_eq!(page.field("table"), "tenant_oauth_credentials");
    assert_eq!(page.field("column"), "client_secret_encrypted");
    assert!(page.field("aad_context").contains(&tenant_id.to_string()));
    captured.assert_never_logged(CLIENT_SECRET);
}

#[tokio::test]
async fn strava_pool_client_secret_decrypt_failure_pages() {
    let database = create_test_db_with_key(WRITE_KEY.to_vec()).await.unwrap();
    database
        .repositories()
        .oauth_tokens
        .upsert_strava_pool_app("pool-703", CLIENT_SECRET, 999, Some("pool"))
        .await
        .unwrap();
    let wrong = with_wrong_key(&database);

    let captured = Captured::start();
    let read = wrong
        .repositories()
        .oauth_tokens
        .get_strava_pool_app_secret("pool-703")
        .await;
    assert!(read.is_err());

    let page = captured.page("key_material.stored_secret_decrypt_failed");
    assert_eq!(page.field("secret_kind"), "strava_pool_client_secret");
    assert_eq!(page.field("table"), "strava_oauth_app_pool");
    assert!(page.field("aad_context").contains("pool-703"));
    captured.assert_never_logged(CLIENT_SECRET);
}

#[tokio::test]
async fn llm_api_key_decrypt_failure_pages() {
    let database = create_test_db_with_key(WRITE_KEY.to_vec()).await.unwrap();
    let (user_id, tenant_id) = user_with_tenant(&database).await;
    TenantLlmManager::store_credentials(
        None,
        tenant_id,
        StoreLlmCredentialsRequest {
            provider: LlmProvider::Groq,
            api_key: LLM_API_KEY.to_owned(),
            base_url: None,
            default_model: None,
        },
        user_id,
        &*database.repositories().llm_credentials,
        &*database.repositories().security,
    )
    .await
    .unwrap();
    let wrong = with_wrong_key(&database);

    let captured = Captured::start();
    let resolved = TenantLlmManager::get_credentials(
        None,
        tenant_id,
        LlmProvider::Groq,
        &*wrong.repositories().llm_credentials,
        &*wrong.repositories().security,
    )
    .await;
    assert!(
        resolved.map_or(true, |c| c.api_key != LLM_API_KEY),
        "the stored key must not resolve through the wrong DEK"
    );

    let page = captured.page("key_material.stored_secret_decrypt_failed");
    assert_eq!(page.field("secret_kind"), "llm_api_key");
    assert_eq!(page.field("table"), "user_llm_credentials");
    assert!(page.field("aad_context").contains("groq"));
    captured.assert_never_logged(LLM_API_KEY);
}

#[tokio::test]
async fn rsa_private_key_decrypt_failure_pages_and_refuses_the_keypair() {
    let database = create_test_db_with_key(WRITE_KEY.to_vec()).await.unwrap();
    database
        .repositories()
        .security
        .save_rsa_keypair(
            "kid-703",
            "sealed-signing-key-703",
            "public-half",
            Utc::now(),
            true,
            2048,
        )
        .await
        .unwrap();
    let wrong = with_wrong_key(&database);

    let captured = Captured::start();
    let loaded = load_or_store_first_keypair(wrong.as_security_repository(), TEST_RSA_BITS).await;
    assert!(
        loaded.is_err(),
        "a signing key that will not decrypt must not be replaced by a fresh one"
    );

    let decrypt = captured.page("key_material.stored_secret_decrypt_failed");
    assert_eq!(decrypt.field("secret_kind"), "rsa_private_key");
    assert_eq!(decrypt.field("aad_context"), "kid-703|rsa_keypairs");
    captured.page("key_material.rsa_keypair_load_failed");
    captured.assert_never_logged("sealed-signing-key-703");
}

#[tokio::test]
async fn client_presented_ciphertext_that_fails_does_not_page() {
    // The sealed Google sign-in cookie is opened through the same trait
    // method; a stale or forged cookie is the client's, never key material.
    let database = create_test_db_with_key(WRITE_KEY.to_vec()).await.unwrap();
    let captured = Captured::start();
    let opened = database.as_security_repository().decrypt_data_with_aad(
        "v1:Zm9yZ2VkLWNvb2tpZS1wYXlsb2Fk",
        "oauth2:google-sign-in:v1",
    );
    assert!(opened.is_err());
    captured.assert_nothing_paged();
    assert!(
        captured
            .all()
            .iter()
            .all(|e| !e.fields.contains_key("event")),
        "a client's ciphertext emits no key-material event: {:#?}",
        captured.all()
    );
}

// ── Key-store failures at boot ───────────────────────────────────────────────

#[tokio::test]
async fn dek_read_failure_pages() {
    let mut first = booting_manager(7);
    let mut database = create_test_db_with_key(first.database_key().to_vec())
        .await
        .unwrap();
    first.complete_initialization(&mut database).await.unwrap();
    break_reads(&database, "database_encryption_key").await;

    let captured = Captured::start();
    let mut booting = booting_manager(7);
    let mut booting_database = database.clone();
    assert!(booting
        .complete_initialization(&mut booting_database)
        .await
        .is_err());

    let page = captured.page("key_material.dek_read_failed");
    assert_eq!(
        page.message,
        "Failed to read the stored DEK; refusing to initialize key management"
    );
}

#[tokio::test]
async fn dek_unwrap_failure_pages() {
    let mut first = booting_manager(7);
    let mut database = create_test_db_with_key(first.database_key().to_vec())
        .await
        .unwrap();
    first.complete_initialization(&mut database).await.unwrap();

    let captured = Captured::start();
    // A different key-encryption key: the stored DEK cannot be unwrapped.
    let mut booting = booting_manager(8);
    let mut booting_database = database.clone();
    assert!(booting
        .complete_initialization(&mut booting_database)
        .await
        .is_err());

    let page = captured.page("key_material.dek_unwrap_failed");
    assert_eq!(page.field("dek_version"), "1");
}

/// The stored Strava access token for `user_id`, read through `database`.
async fn read_access_token(database: &Database, user_id: Uuid, tenant_id: TenantId) -> String {
    database
        .repositories()
        .oauth_tokens
        .get_token(user_id, tenant_id, "strava")
        .await
        .unwrap()
        .expect("the stored token is still there")
        .access_token
}

#[tokio::test]
async fn dek_missing_on_a_database_holding_ciphertext_refuses_the_boot() {
    let mut manager = booting_manager(7);
    let mut database = create_test_db_with_key(WRITE_KEY.to_vec()).await.unwrap();
    // The DEK row is gone; the ciphertext it sealed is still there.
    let (user_id, tenant_id) = store_oauth_token(&database).await;

    let captured = Captured::start();
    let booted = manager.complete_initialization(&mut database).await;

    assert!(
        booted.is_err(),
        "an instance must not mint a DEK over ciphertext the new key cannot open"
    );
    let page = captured.page("key_material.dek_minted_over_ciphertext");
    assert_eq!(page.field("encrypted_rows"), "1");
    let stored = database
        .as_security_repository()
        .get_system_secret("database_encryption_key")
        .await;
    assert_eq!(
        stored.unwrap_err().code,
        ErrorCode::ResourceNotFound,
        "a refused boot stores no DEK row"
    );
    assert_eq!(
        database
            .as_security_repository()
            .count_encrypted_rows()
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        read_access_token(&database, user_id, tenant_id).await,
        ACCESS_TOKEN,
        "the ciphertext still opens under the key that sealed it"
    );
}

#[tokio::test]
async fn dek_minted_on_an_empty_database_is_routine() {
    let mut manager = booting_manager(7);
    let mut database = create_test_db_with_key(manager.database_key().to_vec())
        .await
        .unwrap();

    let captured = Captured::start();
    manager
        .complete_initialization(&mut database)
        .await
        .unwrap();
    captured.assert_nothing_paged();
}

#[tokio::test]
async fn signing_keypair_missing_on_a_database_holding_ciphertext_refuses_the_boot() {
    let database = create_test_db_with_key(WRITE_KEY.to_vec()).await.unwrap();
    // The keypair row is gone; the ciphertext stored beside it is still there.
    let (user_id, tenant_id) = store_oauth_token(&database).await;

    let captured = Captured::start();
    let loaded =
        load_or_store_first_keypair(database.as_security_repository(), TEST_RSA_BITS).await;

    assert!(
        loaded.is_err(),
        "an instance must not mint a signing keypair over a database whose keypair was lost"
    );
    let page = captured.page("key_material.rsa_keypair_minted_over_ciphertext");
    assert_eq!(page.field("encrypted_rows"), "1");
    assert!(
        database
            .as_security_repository()
            .load_rsa_keypairs()
            .await
            .unwrap()
            .is_empty(),
        "a refused boot stores no keypair"
    );
    assert_eq!(
        read_access_token(&database, user_id, tenant_id).await,
        ACCESS_TOKEN,
        "the ciphertext is untouched"
    );
}

#[tokio::test]
async fn signing_keypair_minted_on_an_empty_database_is_routine() {
    let database = create_test_db_with_key(WRITE_KEY.to_vec()).await.unwrap();

    let captured = Captured::start();
    load_or_store_first_keypair(database.as_security_repository(), TEST_RSA_BITS)
        .await
        .unwrap();
    captured.assert_nothing_paged();
}

// ── Corrupt stored key rows: every boot abort pages ──────────────────────────
//
// A stored row a hand repair wrote wrong (double-encoded, truncated, a bad
// version number) aborts every boot. The process exits before the in-process
// notifier flushes, so the `key_material.*` event the log metric matches is
// the only page left.

/// Overwrite one stored system secret with `value`.
async fn store_system_secret(database: &Database, secret_type: &str, value: &str) {
    database
        .as_security_repository()
        .update_system_secret(secret_type, value)
        .await
        .unwrap();
}

/// A database whose DEK is stored, then a booting instance on it.
async fn initialized_database() -> Database {
    let mut first = booting_manager(7);
    let mut database = create_test_db_with_key(first.database_key().to_vec())
        .await
        .unwrap();
    first.complete_initialization(&mut database).await.unwrap();
    database
}

/// Boot a fresh instance against `database` and expect it to refuse.
async fn boot_refuses(database: &Database) {
    let mut booting = booting_manager(7);
    let mut booting_database = database.clone();
    assert!(
        booting
            .complete_initialization(&mut booting_database)
            .await
            .is_err(),
        "a corrupt key row must refuse the boot"
    );
}

#[tokio::test]
async fn unparseable_active_dek_version_pages() {
    let database = initialized_database().await;
    store_system_secret(&database, "database_encryption_key_active_version", "one").await;

    let captured = Captured::start();
    boot_refuses(&database).await;

    captured.page("key_material.dek_read_failed");
}

#[tokio::test]
async fn active_dek_version_missing_from_the_store_pages() {
    let database = initialized_database().await;
    // Version 0 parses but names no stored key.
    store_system_secret(&database, "database_encryption_key_active_version", "0").await;

    let captured = Captured::start();
    boot_refuses(&database).await;

    let page = captured.page("key_material.dek_read_failed");
    assert_eq!(page.field("dek_version"), "0");
}

#[tokio::test]
async fn stored_dek_that_is_not_base64_pages() {
    let database = initialized_database().await;
    store_system_secret(&database, "database_encryption_key", "not base64 !!").await;

    let captured = Captured::start();
    boot_refuses(&database).await;

    let page = captured.page("key_material.dek_read_failed");
    assert_eq!(page.field("dek_version"), "1");
}

#[tokio::test]
async fn stored_dek_of_the_wrong_length_pages() {
    let database = initialized_database().await;
    // A truncated key that still unwraps: 16 bytes wrapped under the right KEK.
    let truncated = kek(7).wrap(&[5u8; 16]).await.unwrap();
    store_system_secret(
        &database,
        "database_encryption_key",
        &BASE64_STANDARD.encode(truncated),
    )
    .await;

    let captured = Captured::start();
    boot_refuses(&database).await;

    let page = captured.page("key_material.dek_unwrap_failed");
    assert_eq!(page.field("dek_version"), "1");
}

#[tokio::test]
async fn failed_ciphertext_count_before_a_keypair_mint_pages() {
    let database = create_test_db_with_key(WRITE_KEY.to_vec()).await.unwrap();
    // rsa_keypairs reads empty, but the ciphertext count cannot run.
    execute_raw(
        &database,
        "ALTER TABLE user_llm_credentials RENAME TO user_llm_credentials_hidden",
    )
    .await;

    let captured = Captured::start();
    let loaded =
        load_or_store_first_keypair(database.as_security_repository(), TEST_RSA_BITS).await;

    execute_raw(
        &database,
        "ALTER TABLE user_llm_credentials_hidden RENAME TO user_llm_credentials",
    )
    .await;
    assert!(loaded.is_err(), "an unanswered count must refuse the mint");
    captured.page("key_material.rsa_keypair_load_failed");
}

/// Run one raw statement against whichever backend the factory opened.
async fn execute_raw(database: &Database, sql: &str) {
    match database.backend() {
        DatabaseBackend::SQLite(sqlite) => {
            sqlx::query(sql).execute(sqlite.pool()).await.unwrap();
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(pg) => {
            sqlx::query(sql).execute(pg.pool()).await.unwrap();
        }
    }
}
