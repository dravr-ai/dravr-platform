// ABOUTME: carnet#1201 — a pierre-cli built without e2e-seeded-sessions, as the server image's is, has no `user mint-session`
// ABOUTME: The command signs an account in with no credential, so the shipped binary must not parse it at all

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::env;
use std::process::Command;

#[test]
fn the_shipped_cli_has_no_mint_session_command() {
    let binary = env::var("CARGO_BIN_EXE_pierre-cli")
        .unwrap_or_else(|_| env!("CARGO_BIN_EXE_pierre-cli").to_owned());
    let out = Command::new(binary)
        .env("DATABASE_URL", "sqlite::memory:")
        .args(["user", "mint-session", "--email", "anyone@pierre.dev"])
        .output()
        .unwrap();
    assert!(!out.status.success(), "the command does not exist");
    assert!(out.stdout.is_empty(), "nothing is printed on stdout");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("unrecognized subcommand 'mint-session'"),
        "clap refuses the subcommand itself: {stderr}"
    );
}
