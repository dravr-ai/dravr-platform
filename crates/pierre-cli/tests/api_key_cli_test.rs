// ABOUTME: `api-key list` and `api-key usage` read a user's keys and one key's tools through the real binary
// ABOUTME: Pins the most-called-first order and that a key belonging to someone else is refused, not reported

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::env;
use std::fs;
use std::process::Command;

use chrono::Utc;
use pierre_auth::key_management::KeyManager;
#[cfg(feature = "postgresql")]
use pierre_core::config::database::PostgresPoolConfig;
use pierre_core::models::{ApiKey, ApiKeyTier, ApiKeyUsage, User};
use pierre_database::backends::{factory::Database, DatabaseProvider};
use uuid::Uuid;

/// Base64 of 32 fixed bytes — the local KEK both this process and the binary use.
const MASTER_KEY: &str = "MDEyMzQ1Njc4OWFiY2RlZjAxMjM0NTY3ODlhYmNkZWY=";

fn cli_binary() -> String {
    env::var("CARGO_BIN_EXE_pierre-cli")
        .unwrap_or_else(|_| env!("CARGO_BIN_EXE_pierre-cli").to_owned())
}

/// Open the database the binary will read, with the same key hierarchy.
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

fn run(url: &str, args: &[&str]) -> (i32, String, String) {
    let out = Command::new(cli_binary())
        .env("DATABASE_URL", url)
        .env("PIERRE_MASTER_ENCRYPTION_KEY", MASTER_KEY)
        .args(args)
        .output()
        .unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

async fn create_user(database: &Database) -> (Uuid, String) {
    let email = format!("{}@test.local", Uuid::new_v4());
    let user = User::new(email.clone(), "hash".to_owned(), None);
    database.repositories().users.create(&user).await.unwrap();
    (user.id, email)
}

async fn create_key(database: &Database, user_id: Uuid, name: &str) -> ApiKey {
    let id = Uuid::new_v4().to_string();
    let key = ApiKey {
        key_prefix: format!("pk_live_{}", &id[..8]),
        key_hash: format!("hash-{id}"),
        id,
        user_id,
        name: name.to_owned(),
        description: None,
        tier: ApiKeyTier::Starter,
        rate_limit_requests: 10_000,
        rate_limit_window_seconds: 2_592_000,
        is_active: true,
        last_used_at: None,
        expires_at: None,
        created_at: Utc::now(),
    };
    database.repositories().api_keys.create(&key).await.unwrap();
    key
}

async fn record_call(database: &Database, key: &ApiKey, tool_name: &str, status_code: u16) {
    database
        .repositories()
        .usage
        .record_api_key(&ApiKeyUsage {
            id: None,
            api_key_id: key.id.clone(),
            timestamp: Utc::now(),
            tool_name: tool_name.to_owned(),
            response_time_ms: Some(20),
            status_code,
            error_message: None,
            request_size_bytes: None,
            response_size_bytes: None,
            ip_address: None,
            user_agent: None,
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn api_key_list_and_usage_show_a_keys_tools_most_called_first() {
    let path = env::temp_dir().join(format!("pierre-cli-api-key-{}.db", Uuid::new_v4()));
    let url = format!("sqlite:{}?mode=rwc", path.display());
    let database = open_database(&url).await;

    let (user_id, email) = create_user(&database).await;
    let key = create_key(&database, user_id, "Export script").await;
    record_call(&database, &key, "get_activities", 200).await;
    record_call(&database, &key, "get_athlete", 200).await;
    record_call(&database, &key, "get_athlete", 200).await;
    record_call(&database, &key, "get_athlete", 500).await;

    let (code, stdout, stderr) = run(&url, &["api-key", "list", "--email", &email]);
    assert_eq!(code, 0, "stdout: {stdout}\nstderr: {stderr}");
    assert!(
        stdout.contains(&key.id) && stdout.contains("Export script"),
        "{stdout}"
    );
    assert!(stdout.contains("active"), "{stdout}");

    let (code, stdout, stderr) = run(&url, &["api-key", "usage", "--email", &email, &key.id]);
    assert_eq!(code, 0, "stdout: {stdout}\nstderr: {stderr}");
    assert!(
        stdout.contains("4 requests, 3 succeeded, 1 failed"),
        "{stdout}"
    );
    let athlete = stdout.find("get_athlete").expect("get_athlete listed");
    let activities = stdout
        .find("get_activities")
        .expect("get_activities listed");
    assert!(athlete < activities, "most-called first:\n{stdout}");

    let _ = fs::remove_file(&path);
}

#[tokio::test]
async fn api_key_usage_refuses_a_key_that_belongs_to_someone_else() {
    let path = env::temp_dir().join(format!("pierre-cli-api-key-{}.db", Uuid::new_v4()));
    let url = format!("sqlite:{}?mode=rwc", path.display());
    let database = open_database(&url).await;

    let (owner_id, _owner_email) = create_user(&database).await;
    let (_other_id, other_email) = create_user(&database).await;
    let key = create_key(&database, owner_id, "Owner key").await;
    record_call(&database, &key, "get_activities", 200).await;

    let (code, stdout, stderr) = run(
        &url,
        &["api-key", "usage", "--email", &other_email, &key.id],
    );
    assert_ne!(code, 0, "another user's key must be refused");
    assert!(
        !stdout.contains("get_activities"),
        "no traffic of the owner's leaks: {stdout}"
    );
    assert!(
        stderr.contains(&key.id),
        "the refusal names the key: {stderr}"
    );

    let _ = fs::remove_file(&path);
}

#[tokio::test]
async fn api_key_usage_refuses_an_empty_window() {
    let path = env::temp_dir().join(format!("pierre-cli-api-key-{}.db", Uuid::new_v4()));
    let url = format!("sqlite:{}?mode=rwc", path.display());
    let database = open_database(&url).await;

    let (user_id, email) = create_user(&database).await;
    let key = create_key(&database, user_id, "Export script").await;

    let (code, _stdout, stderr) = run(
        &url,
        &[
            "api-key", "usage", "--email", &email, "--days", "0", &key.id,
        ],
    );
    assert_ne!(code, 0, "a zero-day window must be refused");
    assert!(stderr.contains("--days must be at least 1"), "{stderr}");

    let _ = fs::remove_file(&path);
}
