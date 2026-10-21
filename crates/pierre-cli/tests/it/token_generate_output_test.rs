// ABOUTME: `token generate` prints a usage example that names a route and port the server really serves
// ABOUTME: Runs the real binary against a scratch SQLite database and reads its stdout

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::env;
use std::fs;
use std::process::Command;

use uuid::Uuid;

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
    let _ = fs::remove_file(&path);

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
