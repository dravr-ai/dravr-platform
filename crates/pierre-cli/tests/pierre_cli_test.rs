// ABOUTME: Integration tests for pierre-cli binary
// ABOUTME: Tests CLI commands for user and token management
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Integration tests for the pierre-cli binary.
//!
//! These tests verify CLI command structure, help output, and error handling.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::{env, process::Command};

/// Get the path to the `pierre-cli` binary
fn cli_binary() -> String {
    // Use runtime env var for nextest archive compatibility (env! bakes absolute path at compile time)
    env::var("CARGO_BIN_EXE_pierre-cli")
        .unwrap_or_else(|_| env!("CARGO_BIN_EXE_pierre-cli").to_owned())
}

/// Helper to run CLI command and capture output
fn run_cli(args: &[&str]) -> (i32, String, String) {
    let output = Command::new(cli_binary()).args(args).output().unwrap();

    let exit_code = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    (exit_code, stdout, stderr)
}

#[test]
fn test_cli_help_shows_commands() {
    let (exit_code, stdout, _stderr) = run_cli(&["--help"]);

    assert_eq!(exit_code, 0, "CLI help should exit with 0");
    assert!(
        stdout.contains("user"),
        "Help should mention 'user' command"
    );
    assert!(
        stdout.contains("token"),
        "Help should mention 'token' command"
    );
    assert!(
        stdout.contains("Pierre MCP Server"),
        "Help should mention Pierre MCP Server"
    );
}

#[test]
fn test_cli_user_help() {
    let (exit_code, stdout, _stderr) = run_cli(&["user", "--help"]);

    assert_eq!(exit_code, 0, "User help should exit with 0");
    assert!(
        stdout.contains("create"),
        "User help should mention 'create' command"
    );
    assert!(
        stdout.contains("User management"),
        "User help should describe user management"
    );
}

#[test]
fn test_cli_user_create_help() {
    let (exit_code, stdout, _stderr) = run_cli(&["user", "create", "--help"]);

    assert_eq!(exit_code, 0, "User create help should exit with 0");
    assert!(stdout.contains("--email"), "Should show --email option");
    assert!(
        stdout.contains("--password"),
        "Should show --password option"
    );
    assert!(
        stdout.contains("--super-admin"),
        "Should show --super-admin option"
    );
    assert!(stdout.contains("--force"), "Should show --force option");
    // The dev-setup script creates jf_test / phil_test / the operator through this
    // command and passes `--locale en`; a dropped flag would silently seed them French.
    assert!(stdout.contains("--locale"), "Should show --locale option");
}

#[test]
fn test_cli_token_help() {
    let (exit_code, stdout, _stderr) = run_cli(&["token", "--help"]);

    assert_eq!(exit_code, 0, "Token help should exit with 0");
    assert!(
        stdout.contains("generate"),
        "Token help should mention 'generate' command"
    );
    assert!(
        stdout.contains("list"),
        "Token help should mention 'list' command"
    );
    assert!(
        stdout.contains("revoke"),
        "Token help should mention 'revoke' command"
    );
    assert!(
        stdout.contains("rotate"),
        "Token help should mention 'rotate' command"
    );
    assert!(
        stdout.contains("stats"),
        "Token help should mention 'stats' command"
    );
}

#[test]
fn test_cli_token_generate_help() {
    let (exit_code, stdout, _stderr) = run_cli(&["token", "generate", "--help"]);

    assert_eq!(exit_code, 0, "Token generate help should exit with 0");
    assert!(stdout.contains("--service"), "Should show --service option");
    assert!(
        stdout.contains("--expires-days"),
        "Should show --expires-days option"
    );
    assert!(
        stdout.contains("--super-admin"),
        "Should show --super-admin option"
    );
    assert!(
        stdout.contains("--permissions"),
        "Should show --permissions option"
    );
}

#[test]
fn test_cli_token_list_help() {
    let (exit_code, stdout, _stderr) = run_cli(&["token", "list", "--help"]);

    assert_eq!(exit_code, 0, "Token list help should exit with 0");
    assert!(
        stdout.contains("--include-inactive"),
        "Should show --include-inactive option"
    );
    assert!(
        stdout.contains("--detailed"),
        "Should show --detailed option"
    );
}

#[test]
fn test_cli_token_revoke_help() {
    let (exit_code, stdout, _stderr) = run_cli(&["token", "revoke", "--help"]);

    assert_eq!(exit_code, 0, "Token revoke help should exit with 0");
    assert!(
        stdout.contains("token_id") || stdout.contains("TOKEN_ID"),
        "Should show token_id argument"
    );
}

#[test]
fn test_cli_token_rotate_help() {
    let (exit_code, stdout, _stderr) = run_cli(&["token", "rotate", "--help"]);

    assert_eq!(exit_code, 0, "Token rotate help should exit with 0");
    assert!(
        stdout.contains("token_id") || stdout.contains("TOKEN_ID"),
        "Should show token_id argument"
    );
    assert!(
        stdout.contains("--expires-days"),
        "Should show --expires-days option"
    );
}

#[test]
fn test_cli_token_stats_help() {
    let (exit_code, stdout, _stderr) = run_cli(&["token", "stats", "--help"]);

    assert_eq!(exit_code, 0, "Token stats help should exit with 0");
    assert!(stdout.contains("--days"), "Should show --days option");
}

#[test]
fn test_cli_verbose_flag() {
    let (exit_code, stdout, _stderr) = run_cli(&["-v", "--help"]);

    assert_eq!(exit_code, 0, "Verbose flag with help should exit with 0");
    assert!(
        stdout.contains("user"),
        "Help should still work with -v flag"
    );
}

#[test]
fn test_cli_invalid_command() {
    let (exit_code, _stdout, stderr) = run_cli(&["invalid-command"]);

    assert_ne!(exit_code, 0, "Invalid command should exit with non-zero");
    assert!(
        stderr.contains("error") || stderr.contains("invalid"),
        "Should show error for invalid command"
    );
}

#[test]
fn test_cli_user_create_missing_required_args() {
    let (exit_code, _stdout, stderr) = run_cli(&["user", "create"]);

    assert_ne!(
        exit_code, 0,
        "User create without args should exit with non-zero"
    );
    assert!(
        stderr.contains("--email") || stderr.contains("required"),
        "Should indicate missing required args"
    );
}

#[test]
fn test_cli_token_generate_missing_required_args() {
    let (exit_code, _stdout, stderr) = run_cli(&["token", "generate"]);

    assert_ne!(
        exit_code, 0,
        "Token generate without args should exit with non-zero"
    );
    assert!(
        stderr.contains("--service") || stderr.contains("required"),
        "Should indicate missing required args"
    );
}

#[test]
fn test_cli_token_revoke_missing_token_id() {
    let (exit_code, _stdout, stderr) = run_cli(&["token", "revoke"]);

    assert_ne!(
        exit_code, 0,
        "Token revoke without token_id should exit with non-zero"
    );
    assert!(
        stderr.contains("token_id") || stderr.contains("required") || stderr.contains("TOKEN_ID"),
        "Should indicate missing token_id"
    );
}

#[test]
fn test_cli_database_url_option() {
    let (exit_code, stdout, _stderr) = run_cli(&["--database-url", "sqlite::memory:", "--help"]);

    assert_eq!(
        exit_code, 0,
        "Database URL option with help should exit with 0"
    );
    assert!(
        stdout.contains("user"),
        "Help should work with database-url option"
    );
}

#[test]
fn test_settings_help_shows_both_surfaces() {
    let (exit_code, stdout, _stderr) = run_cli(&["settings", "--help"]);

    assert_eq!(exit_code, 0, "Settings help should exit with 0");
    assert!(
        stdout.contains("guardian"),
        "Settings help should mention 'guardian'"
    );
    assert!(
        stdout.contains("harness"),
        "Settings help should mention 'harness'"
    );
}

#[test]
fn test_settings_guardian_help_shows_verbs() {
    let (exit_code, stdout, _stderr) = run_cli(&["settings", "guardian", "--help"]);

    assert_eq!(exit_code, 0, "Guardian settings help should exit with 0");
    assert!(stdout.contains("show"), "should list the 'show' verb");
    assert!(stdout.contains("set"), "should list the 'set' verb");
}

#[test]
fn test_settings_guardian_set_help_lists_every_policy_flag() {
    let (exit_code, stdout, _stderr) = run_cli(&["settings", "guardian", "set", "--help"]);

    assert_eq!(exit_code, 0, "Guardian set help should exit with 0");
    for flag in [
        "--mode",
        "--tainted-destructive",
        "--plan-mode",
        "--max-destructive-per-turn",
        "--max-writes-per-turn",
        "--external-send",
        "--clear",
    ] {
        assert!(stdout.contains(flag), "set help should list {flag}");
    }
}

#[test]
fn test_settings_harness_set_requires_the_json_argument() {
    let (exit_code, _stdout, stderr) = run_cli(&["settings", "harness", "set"]);

    assert_ne!(exit_code, 0, "harness set without JSON must fail");
    assert!(
        stderr.contains("JSON") || stderr.contains("json") || stderr.contains("required"),
        "error should point at the missing document argument, got: {stderr}"
    );
}

// ===========================================================================
// Pre-approval verbs — remote, like `user get` / `user set`
//
// `user allow / disallow / list-allowed` used to hold a repository handle, so
// they opened DATABASE_URL and could only ever write the local database: an
// operator with a valid device login could not pre-approve an address on a
// deployed environment at all (registre#110). These pin the two things a
// regression would break — the `--server` / `--token` arguments clap must
// accept, and dispatch happening *before* the KeyManager/database bootstrap.
// ===========================================================================

#[test]
fn test_cli_user_allow_help_offers_remote_arguments() {
    let (exit_code, stdout, _stderr) = run_cli(&["user", "allow", "--help"]);

    assert_eq!(exit_code, 0, "User allow help should exit with 0");
    assert!(stdout.contains("--email"), "Should show --email option");
    assert!(stdout.contains("--note"), "Should show --note option");
    assert!(
        stdout.contains("--server"),
        "allow must take --server, or it cannot reach a deployed environment"
    );
    assert!(
        stdout.contains("--token"),
        "allow must take --token for a headless/CI operator"
    );
}

#[test]
fn test_cli_user_disallow_and_list_allowed_help_offer_remote_arguments() {
    let (exit_code, stdout, _stderr) = run_cli(&["user", "disallow", "--help"]);
    assert_eq!(exit_code, 0, "User disallow help should exit with 0");
    assert!(stdout.contains("--email"), "Should show --email option");
    assert!(stdout.contains("--server"), "disallow must take --server");
    assert!(stdout.contains("--token"), "disallow must take --token");

    let (exit_code, stdout, _stderr) = run_cli(&["user", "list-allowed", "--help"]);
    assert_eq!(exit_code, 0, "User list-allowed help should exit with 0");
    assert!(
        stdout.contains("--server"),
        "list-allowed must take --server"
    );
    assert!(stdout.contains("--token"), "list-allowed must take --token");
    assert!(
        stdout.contains("--format"),
        "list-allowed must take --format, like the other remote listing"
    );
}

/// Every remote user verb must dispatch before the KeyManager/DB bootstrap, so
/// one binary serves a laptop and a deployed environment alike.
///
/// A closed port is the probe: the command must fail trying to *reach the
/// server*. If a verb ever regresses to the database path it announces
/// "Connecting to database" and fails on the local `SQLite` file instead, which
/// is exactly the bug that made `user allow` useless against dev.
#[test]
fn test_remote_user_verbs_never_open_the_local_database() {
    const CLOSED_PORT: &str = "http://127.0.0.1:1";
    let invocations: [&[&str]; 4] = [
        &[
            "user",
            "allow",
            "--email",
            "someone@example.com",
            "--server",
            CLOSED_PORT,
            "--token",
            "not-a-real-token",
        ],
        &[
            "user",
            "disallow",
            "--email",
            "someone@example.com",
            "--server",
            CLOSED_PORT,
            "--token",
            "not-a-real-token",
        ],
        &[
            "user",
            "get",
            "--server",
            CLOSED_PORT,
            "--token",
            "not-a-real-token",
        ],
        &[
            "user",
            "list-allowed",
            "--server",
            CLOSED_PORT,
            "--token",
            "not-a-real-token",
        ],
    ];

    for args in invocations {
        let (exit_code, stdout, stderr) = run_cli(args);

        assert_ne!(
            exit_code,
            0,
            "an unreachable server must fail `{}`",
            args.join(" ")
        );
        let output = format!("{stdout}{stderr}");
        assert!(
            !output.contains("Connecting to database"),
            "`{}` must not open a database: {output}",
            args.join(" ")
        );
        assert!(
            !output.contains("key management"),
            "`{}` must dispatch before the KeyManager bootstrap: {output}",
            args.join(" ")
        );
    }
}

/// A typo'd `--format` must name the formats that exist rather than silently
/// printing a table, and must be rejected before the command spends a round
/// trip on the server.
#[test]
fn test_unknown_format_is_rejected_by_name() {
    for verb in ["get", "list-allowed"] {
        let (exit_code, stdout, stderr) = run_cli(&[
            "user",
            verb,
            "--server",
            "http://127.0.0.1:1",
            "--token",
            "not-a-real-token",
            "--format",
            "jsonl",
        ]);

        assert_ne!(exit_code, 0, "`user {verb} --format jsonl` must fail");
        let output = format!("{stdout}{stderr}");
        assert!(
            output.contains("jsonl"),
            "the error must quote the rejected format: {output}"
        );
        for known in ["table", "json", "csv"] {
            assert!(
                output.contains(known),
                "the error must name `{known}` as a valid format: {output}"
            );
        }
        assert!(
            !output.contains("error sending request"),
            "the format must be validated before the server is called: {output}"
        );
    }
}

/// `--format` is matched case-insensitively, so an uppercase value reaches the
/// network rather than being rejected as unknown.
#[test]
fn test_format_flag_is_case_insensitive() {
    let (exit_code, stdout, stderr) = run_cli(&[
        "user",
        "get",
        "--server",
        "http://127.0.0.1:1",
        "--token",
        "not-a-real-token",
        "--format",
        "JSON",
    ]);

    assert_ne!(exit_code, 0, "the closed port must still fail the command");
    let output = format!("{stdout}{stderr}");
    assert!(
        !output.contains("Unknown format"),
        "`JSON` must be accepted as `json`: {output}"
    );
}

/// The tracing preamble must never reach stdout: `--format json` / `csv` put a
/// machine-readable payload there, and a log line ahead of it makes the payload
/// unparseable for any caller that pipes the command.
///
/// `auth status` is the probe because it dispatches before the KeyManager/DB
/// bootstrap, so it needs neither a database nor a reachable server.
#[test]
fn test_logs_go_to_stderr_not_stdout() {
    let (_exit_code, stdout, stderr) = run_cli(&["auth", "status"]);

    assert!(
        !stdout.contains("Pierre MCP Server CLI"),
        "the startup log must not contaminate stdout: {stdout}"
    );
    assert!(
        stderr.contains("Pierre MCP Server CLI"),
        "the startup log must still be emitted, on stderr: {stderr}"
    );
}

/// `--verbose` raises the filter to debug, which must not reopen the stdout
/// contamination that `test_logs_go_to_stderr_not_stdout` closes: stdout still
/// carries the payload alone.
#[test]
fn test_verbose_logs_stay_off_stdout() {
    let (_exit_code, stdout, stderr) = run_cli(&["--verbose", "auth", "status"]);

    assert!(
        !stdout.contains("Pierre MCP Server CLI"),
        "debug logging must not contaminate stdout: {stdout}"
    );
    assert!(
        stderr.contains("Pierre MCP Server CLI"),
        "the startup log must still be emitted, on stderr: {stderr}"
    );
}
