// ABOUTME: `oauth2-client create` provisions the confidential client a resource server introspects tokens with
// ABOUTME: Runs the real binary on a scratch SQLite database: the row's shape, the secret printed once, and what it may mint

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::env;
use std::fs;
use std::process::Command;
use std::sync::Arc;

use pierre_auth::oauth2_server::client_registration::ClientRegistrationManager;
use uuid::Uuid;

use crate::scratch_db::open;

/// Base64 of 32 fixed bytes — the local KEK the binary bootstraps with.
const MASTER_KEY: &str = "MDEyMzQ1Njc4OWFiY2RlZjAxMjM0NTY3ODlhYmNkZWY=";

fn cli_binary() -> String {
    env::var("CARGO_BIN_EXE_pierre-cli")
        .unwrap_or_else(|_| env!("CARGO_BIN_EXE_pierre-cli").to_owned())
}

/// The value printed after `label` on its own line of `stdout`.
fn printed_value<'a>(stdout: &'a str, label: &str) -> &'a str {
    stdout
        .lines()
        .find_map(|line| line.strip_prefix(label))
        .map_or_else(|| panic!("{label} is printed: {stdout}"), str::trim)
}

/// The command stores a confidential `client_credentials`-only client with
/// no redirect URI, no scope and no expiry; prints its `client_id` and its
/// secret once, to stdout and never to the log on stderr; tells the admin
/// to name it in `OAUTH2_INTROSPECTION_CLIENTS`; and the secret
/// authenticates it. What it may mint is `oauth2_introspection_test`'s.
#[tokio::test]
async fn create_stores_a_confidential_introspection_client_and_prints_its_secret_once() {
    let path = env::temp_dir().join(format!("pierre-cli-oauth2-client-{}.db", Uuid::new_v4()));
    let url = format!("sqlite:{}?mode=rwc", path.display());
    let output = Command::new(cli_binary())
        .env("DATABASE_URL", &url)
        .env("PIERRE_MASTER_ENCRYPTION_KEY", MASTER_KEY)
        .args(["oauth2-client", "create", "--name", "barrage-mcp"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(output.status.success(), "{stdout}{stderr}");

    let client_id = printed_value(&stdout, "client_id:");
    let secret = printed_value(&stdout, "client_secret:");
    assert!(!secret.is_empty(), "{stdout}");
    assert_eq!(stdout.matches(secret).count(), 1, "printed once: {stdout}");
    assert!(
        !stderr.contains(secret),
        "bug: the secret reached the log: {stderr}"
    );
    assert!(stdout.contains("OAUTH2_INTROSPECTION_CLIENTS"), "{stdout}");

    let database = open(&url).await;
    let repository = Arc::clone(&database.repositories().oauth2_server);
    let stored = repository.get_client(client_id).await.unwrap().unwrap();
    assert_eq!(
        stored.client_name.as_deref(),
        Some("barrage-mcp"),
        "bug: the client was stored under another name"
    );
    assert_eq!(
        stored.grant_types,
        ["client_credentials"],
        "bug: the introspection client may use another grant type"
    );
    assert_eq!(
        stored.redirect_uris,
        Vec::<String>::new(),
        "no redirect URI"
    );
    assert_eq!(
        stored.response_types,
        Vec::<String>::new(),
        "no response type"
    );
    assert_eq!(
        stored.scope.as_deref(),
        Some(""),
        "no scope, never the default"
    );
    assert_eq!(stored.expires_at, None, "it never expires");
    assert_ne!(stored.client_secret_hash, secret, "only the hash is stored");

    let manager = ClientRegistrationManager::new(repository);
    let authenticated = manager.authenticate_client(client_id, secret).await;
    let wrong = manager
        .authenticate_client(client_id, "not-the-secret")
        .await;
    drop(database);
    // Best-effort cleanup of the temp file.
    drop(fs::remove_file(&path));

    assert!(authenticated.is_ok(), "the printed secret authenticates it");
    assert!(wrong.is_err(), "bug: another secret authenticated it");
}
