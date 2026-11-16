// ABOUTME: `token generate` prints a usage example that names a route and port the server really serves, and binds a token to a real tenant id only
// ABOUTME: Runs the real binary against a scratch SQLite database and reads what it prints

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::env;
use std::fs;
use std::process::Command;

use uuid::Uuid;

use crate::scratch_db::open;

/// Base64 of 32 fixed bytes — the local KEK the binary bootstraps with.
const MASTER_KEY: &str = "MDEyMzQ1Njc4OWFiY2RlZjAxMjM0NTY3ODlhYmNkZWY=";

fn cli_binary() -> String {
    env::var("CARGO_BIN_EXE_pierre-cli")
        .unwrap_or_else(|_| env!("CARGO_BIN_EXE_pierre-cli").to_owned())
}

#[test]
fn the_usage_example_names_the_provision_route_on_the_server_port() {
    let path = env::temp_dir().join(format!("pierre-cli-token-{}.db", Uuid::new_v4()));
    let url = format!("sqlite:{}?mode=rwc", path.display());
    let run = |args: &[&str]| {
        Command::new(cli_binary())
            .env("DATABASE_URL", &url)
            .env("PIERRE_MASTER_ENCRYPTION_KEY", MASTER_KEY)
            .args(args)
            .output()
            .unwrap()
    };
    // Minting a token needs the admin JWT secret a first admin user creates.
    let created = run(&[
        "user",
        "create",
        "--email",
        "operator@example.com",
        "--password",
        "correct-horse-battery-staple",
    ]);
    assert!(
        created.status.success(),
        "{}",
        String::from_utf8_lossy(&created.stderr)
    );
    let output = run(&["token", "generate", "--service", "usage-example-check"]);
    // Best-effort cleanup of the temp files.
    drop(fs::remove_file(&path));

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "stdout: {stdout}\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains("-X POST http://localhost:8081/admin/provision \\"),
        "the example must name the served route on the server's port: {stdout}"
    );
    assert!(!stdout.contains("provision-api-key"), "{stdout}");
    assert!(!stdout.contains("localhost:8080"), "{stdout}");
}

/// `--tenant-id` binds a token to a tenant id and to nothing else
/// (carnet#1589). The value was stored as typed, and the server refuses a
/// token whose tenant it cannot read on every tenant-scoped request and
/// every token write: a mistyped id minted a token that could do nothing it
/// was bound for. It is refused before anything is minted, and a real id
/// is stored in the form the server compares.
#[tokio::test]
async fn a_tenant_id_that_is_not_one_mints_nothing() {
    let path = env::temp_dir().join(format!("pierre-cli-token-tenant-{}.db", Uuid::new_v4()));
    let url = format!("sqlite:{}?mode=rwc", path.display());
    let run = |args: &[&str]| {
        let output = Command::new(cli_binary())
            .env("DATABASE_URL", &url)
            .env("PIERRE_MASTER_ENCRYPTION_KEY", MASTER_KEY)
            .args(args)
            .output()
            .unwrap();
        let printed = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        (output.status.success(), printed)
    };
    let (created, said) = run(&[
        "user",
        "create",
        "--email",
        "admin@example.com",
        "--password",
        "correct-horse-battery-staple",
    ]);
    assert!(created, "{said}");

    let tenant = Uuid::new_v4();
    let generate = |service: &str, tenant_id: &str| {
        run(&[
            "token",
            "generate",
            "--service",
            service,
            "--tenant-id",
            tenant_id,
        ])
    };
    let (mistyped, refusal) = generate("mistyped-desk", "club-montreal");
    let (bound, _) = generate("club-desk", &tenant.to_string().to_uppercase());
    let database = open(&url).await;
    let tokens = database.repositories().admin.list_tokens(true).await;
    let tokens = tokens.unwrap();
    drop(database);
    // Best-effort cleanup of the temp files.
    drop(fs::remove_file(&path));

    // What a mint prints is the token, so a wrongly minted one is not quoted.
    assert!(!mistyped, "a tenant that is no tenant id is refused");
    assert!(
        refusal
            .contains("--tenant-id must be a tenant id (a UUID), and 'club-montreal' is not one"),
        "{refusal}"
    );
    assert!(bound, "a tenant id binds a token");
    let minted: Vec<_> = tokens
        .iter()
        .map(|token| (token.service_name.as_str(), token.tenant_id.clone()))
        .collect();
    assert_eq!(
        minted,
        [("club-desk", Some(tenant.to_string()))],
        "the refused request minted nothing, and the id is stored as the server compares it"
    );
}
