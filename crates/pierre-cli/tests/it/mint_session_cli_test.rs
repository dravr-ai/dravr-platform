// ABOUTME: carnet#1201 — `pierre-cli user mint-session` mints a mobile session the server honours, and its refresh token renews it
// ABOUTME: Runs the binary against a fresh SQLite database; also pins its user.login event, its masked refusal and its SQLite-only policy

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The mobile E2E flows that do not test sign-in start signed in with a pair
//! this command mints (carnet#1201). The pair is only worth anything if the
//! server treats it as the code exchange's own: the access token admitted by
//! the auth middleware every `/api/` route authenticates with, as the
//! `dravr-mobile` client and as that account, and the refresh token renewing
//! the session through the refresh grant's service call exactly once.
//!
//! The binary writes the database; this process opens the same file with the
//! same master key, handed over explicitly rather than through the
//! environment, so the suite stays a module of the one test binary.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::env;
use std::fs;
use std::process::Command;
use std::sync::Arc;

use pierre_auth::admin::jwks::{load_or_store_first_keypair, RSA_KEY_SIZE};
use pierre_auth::auth::AuthManager;
use pierre_auth::key_management::{KeyManager, LocalKekProvider, MasterEncryptionKey};
use pierre_auth::oauth2_server::first_party::MOBILE_CLIENT_ID;
use pierre_cache::{Cache, CacheConfig};
use pierre_config::environment::ServerConfig;
#[cfg(feature = "postgresql")]
use pierre_core::config::database::PostgresPoolConfig;
use pierre_database::backends::factory::Database;
use pierre_middleware::{mask_email, McpAuthMiddleware};
use pierre_providers::ProviderRegistry;
use pierre_runtime_context::DataContext;
use pierre_services::auth::AuthService;
use secrecy::ExposeSecret;
use serde_json::Value;
use uuid::Uuid;

/// The local KEK, as the binary reads it (base64) and as this process holds it (bytes).
const MASTER_KEY_BASE64: &str = "MDEyMzQ1Njc4OWFiY2RlZjAxMjM0NTY3ODlhYmNkZWY=";
const MASTER_KEY: [u8; 32] = *b"0123456789abcdef0123456789abcdef";

fn cli_binary() -> String {
    env::var("CARGO_BIN_EXE_pierre-cli")
        .unwrap_or_else(|_| env!("CARGO_BIN_EXE_pierre-cli").to_owned())
}

/// What a successful run printed: stdout, and the log lines on stderr.
struct CliRun {
    stdout: String,
    stderr: String,
}

/// Run the binary against `url`, failing the test with its output when it fails.
fn run_cli(url: &str, args: &[&str]) -> CliRun {
    let out = Command::new(cli_binary())
        .env("DATABASE_URL", url)
        .env("PIERRE_MASTER_ENCRYPTION_KEY", MASTER_KEY_BASE64)
        .env("RUST_LOG", "warn")
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "pierre-cli {args:?} failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    CliRun {
        stdout: String::from_utf8(out.stdout).unwrap(),
        stderr: String::from_utf8(out.stderr).unwrap(),
    }
}

/// The database the binary wrote, opened with the key hierarchy it used.
async fn open_database(url: &str) -> Database {
    let (mut key_manager, key) = KeyManager::bootstrap_with(Box::new(LocalKekProvider::from_mek(
        MasterEncryptionKey::from_bytes(MASTER_KEY),
    )));
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
    database
}

#[tokio::test]
async fn a_minted_session_signs_the_account_in_as_the_mobile_app_and_refreshes_once() {
    let path = env::temp_dir().join(format!("pierre-cli-mint-session-{}.db", Uuid::new_v4()));
    let url = format!("sqlite:{}?mode=rwc", path.display());
    let email = format!("mint-{}@pierre.dev", Uuid::new_v4().simple());
    run_cli(
        &url,
        &[
            "user",
            "create",
            "--email",
            &email,
            "--password",
            "MintSession1234",
        ],
    );

    let run = run_cli(&url, &["user", "mint-session", "--email", &email]);
    let stdout = run.stdout;
    let minted: Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("mint-session must print one JSON object, got {stdout:?}: {e}"));
    assert_eq!(minted["client_id"], MOBILE_CLIENT_ID);
    // Wire values the app parses, pinned on purpose: RFC 6750's token type,
    // and OpenID Connect Core §11's scope the app's code exchange sends.
    assert_eq!(minted["token_type"], "Bearer");
    assert_eq!(minted["scope"], "offline_access");
    let access_token = minted["access_token"].as_str().unwrap().to_owned();
    let refresh_token = minted["refresh_token"].as_str().unwrap().to_owned();
    assert!(
        !access_token.is_empty(),
        "the session carries an access token"
    );
    assert!(
        !refresh_token.is_empty(),
        "the session carries a refresh token"
    );

    let database = open_database(&url).await;
    let repos = Arc::clone(database.repositories());
    let user = repos.users.get_by_email(&email).await.unwrap().unwrap();
    assert_eq!(minted["user_id"], user.id.to_string());

    // The sign-in is announced as every sign-in is, naming this command as its
    // source and the account it opened, and logged as the CLI's, not as a
    // redeemed authorization code.
    let login_event = run
        .stderr
        .lines()
        .find(|line| line.contains("user.login"))
        .unwrap_or_else(|| panic!("mint-session raises user.login; stderr: {}", run.stderr));
    assert!(
        login_event.contains("pierre_cli_mint_session"),
        "the event names the CLI mint as its source: {login_event}"
    );
    assert!(
        login_event.contains(&user.id.to_string()),
        "the event names the account: {login_event}"
    );
    assert!(
        run.stderr
            .contains("User signed in with a session pierre-cli minted"),
        "the sign-in is logged as the CLI's: {}",
        run.stderr
    );
    assert!(
        !run.stderr.contains("authorization code"),
        "the CLI mint is not logged as a code exchange: {}",
        run.stderr
    );
    assert!(
        !run.stderr.contains(&access_token) && !run.stderr.contains(&refresh_token),
        "no token reaches the log"
    );

    // The middleware every /api route authenticates a bearer with, over the
    // signing keys the server loads at boot.
    let jwks = Arc::new(
        load_or_store_first_keypair(repos.security.as_ref(), RSA_KEY_SIZE)
            .await
            .unwrap(),
    );
    let middleware =
        McpAuthMiddleware::new(AuthManager::new(24), Arc::clone(&repos), Arc::clone(&jwks));
    let admitted = middleware
        .authenticate_request(Some(&format!("Bearer {access_token}")))
        .await
        .expect("the minted access token must authenticate an /api request");
    assert_eq!(
        admitted.user_id, user.id,
        "the token signs in the account it was minted for"
    );
    assert_eq!(
        admitted.client_id.as_deref(),
        Some(MOBILE_CLIENT_ID),
        "the token names the mobile app, as the code exchange's does"
    );

    // The refresh grant's service call: a fresh JWT for the same account,
    // admitted the same way, and the presented token spent.
    let data = DataContext::new(
        Arc::new(database),
        Arc::clone(&repos),
        Arc::new(
            Cache::new(CacheConfig {
                enable_background_cleanup: false,
                ..CacheConfig::default()
            })
            .await
            .unwrap(),
        ),
        Arc::new(ProviderRegistry::new()),
    );
    let auth_service = AuthService::new(
        Arc::new(AuthManager::new(24)),
        Arc::clone(&jwks),
        Arc::new(ServerConfig::from_env().unwrap()),
        data,
    );
    let refreshed = auth_service
        .refresh_session(&refresh_token)
        .await
        .expect("the minted refresh token must renew the session");
    assert_eq!(refreshed.login.user.user_id, user.id.to_string());
    let renewed = refreshed.login.jwt_token.as_ref().unwrap().expose_secret();
    let readmitted = middleware
        .authenticate_request(Some(&format!("Bearer {renewed}")))
        .await
        .expect("the renewed access token must authenticate too");
    assert_eq!(readmitted.user_id, user.id);
    assert_eq!(readmitted.client_id.as_deref(), Some(MOBILE_CLIENT_ID));
    assert!(
        auth_service.refresh_session(&refresh_token).await.is_err(),
        "a minted refresh token rotates like any other: it works exactly once"
    );

    drop(auth_service);
    // Best-effort cleanup of the temp file.
    drop(fs::remove_file(&path));
}

#[test]
fn minting_for_an_unknown_account_fails_naming_it_masked() {
    let path = env::temp_dir().join(format!("pierre-cli-mint-unknown-{}.db", Uuid::new_v4()));
    let url = format!("sqlite:{}?mode=rwc", path.display());
    let out = Command::new(cli_binary())
        .env("DATABASE_URL", &url)
        .env("PIERRE_MASTER_ENCRYPTION_KEY", MASTER_KEY_BASE64)
        .env("RUST_LOG", "warn")
        .args(["user", "mint-session", "--email", "nobody@pierre.dev"])
        .output()
        .unwrap();
    assert!(!out.status.success(), "an unknown account mints nothing");
    assert!(
        out.stdout.is_empty(),
        "no token is printed: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains(&mask_email("nobody@pierre.dev")),
        "the failure names the account, masked: {stderr}"
    );
    assert!(
        !stderr.contains("nobody@pierre.dev"),
        "the full address stays out of the error: {stderr}"
    );
    drop(fs::remove_file(&path));
}

#[test]
fn minting_refuses_any_database_but_sqlite_before_connecting() {
    // Nothing listens there: a refusal that came from a connection attempt
    // would name the connection, not the policy.
    let out = Command::new(cli_binary())
        .env(
            "DATABASE_URL",
            "postgresql://pierre:pierre@127.0.0.1:1/pierre",
        )
        .env("PIERRE_MASTER_ENCRYPTION_KEY", MASTER_KEY_BASE64)
        .env("RUST_LOG", "warn")
        .args(["user", "mint-session", "--email", "anyone@pierre.dev"])
        .output()
        .unwrap();
    assert!(!out.status.success(), "a Postgres URL mints nothing");
    assert!(
        out.stdout.is_empty(),
        "no token is printed: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("only against a local SQLite database"),
        "the refusal names the policy: {stderr}"
    );
    assert!(
        !stderr.contains("Connecting to database"),
        "the refusal comes before any connection: {stderr}"
    );
}
