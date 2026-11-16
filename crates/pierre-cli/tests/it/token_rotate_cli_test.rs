// ABOUTME: `token rotate` mints the replacement the one rotation definition describes, and revokes the old token
// ABOUTME: Runs the real binary against a scratch SQLite database and reads what `token list --detailed` says

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::env;
use std::fs;
use std::process::{Command, Output};

use uuid::Uuid;

/// Base64 of 32 fixed bytes — the local KEK the binary bootstraps with.
const MASTER_KEY: &str = "MDEyMzQ1Njc4OWFiY2RlZjAxMjM0NTY3ODlhYmNkZWY=";

fn cli_binary() -> String {
    env::var("CARGO_BIN_EXE_pierre-cli")
        .unwrap_or_else(|_| env!("CARGO_BIN_EXE_pierre-cli").to_owned())
}

/// What stands where a command printed a token.
const TOKEN_NOT_QUOTED: &str = "[a token, not quoted]";

/// What a command printed, fit to quote in a failed assertion: `token
/// generate` and `token rotate` print the minted token three times (alone,
/// in an `export` line, in a `curl` example), and each is replaced. A
/// token is a JWT, which starts `eyJ` and runs over base64url and dots.
fn printed(output: &Output) -> String {
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let mut quotable = String::with_capacity(text.len());
    let mut rest = text.as_str();
    while let Some(start) = rest.find("eyJ") {
        let token = &rest[start..];
        let end = token
            .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')))
            .unwrap_or(token.len());
        quotable.push_str(&rest[..start]);
        quotable.push_str(TOKEN_NOT_QUOTED);
        rest = &token[end..];
    }
    quotable.push_str(rest);
    quotable
}

/// The value after `label` on the first line of `text` that carries it.
fn value_after<'a>(text: &'a str, label: &str) -> &'a str {
    text.lines()
        .find_map(|line| line.split_once(label).map(|(_, value)| value.trim()))
        .unwrap_or_else(|| panic!("no `{label}` line in: {text}"))
}

/// The block `token list --detailed` prints for the token `id`.
fn listed<'a>(listing: &'a str, id: &str) -> &'a str {
    listing
        .split("Key Token ID: ")
        .find(|block| block.starts_with(id))
        .unwrap_or_else(|| panic!("token {id} is not listed: {listing}"))
}

/// A rotation carries over what `CreateAdminTokenRequest::rotation_of`
/// carries: the name, the description and the permissions the token holds,
/// never the defaults of a new one, without the super-admin flag it never
/// had. The old token is revoked, and a revoked token has no replacement.
#[test]
fn a_rotation_keeps_the_name_the_description_and_the_permissions() {
    let path = env::temp_dir().join(format!("pierre-cli-rotate-{}.db", Uuid::new_v4()));
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
        "admin@example.com",
        "--password",
        "correct-horse-battery-staple",
    ]);
    assert!(created.status.success(), "{}", printed(&created));
    let generated = run(&[
        "token",
        "generate",
        "--service",
        "trail-reader",
        "--description",
        "reads the trail",
        "--permissions",
        "view_audit_logs",
        "--expires-days",
        "30",
    ]);
    assert!(generated.status.success(), "{}", printed(&generated));
    let generated = printed(&generated);
    let old_id = value_after(&generated, "Token ID: ").to_owned();

    let rotated = run(&["token", "rotate", &old_id, "--expires-days", "7"]);
    let again = run(&["token", "rotate", &old_id]);
    let listing = run(&["token", "list", "--include-inactive", "--detailed"]);
    // Best-effort cleanup of the temp files.
    drop(fs::remove_file(&path));

    assert!(rotated.status.success(), "{}", printed(&rotated));
    let rotated = printed(&rotated);
    let new_id = value_after(&rotated, "New Token: ")
        .trim_end_matches(" (active)")
        .to_owned();
    assert_ne!(new_id, old_id);

    assert!(listing.status.success(), "{}", printed(&listing));
    let listing = printed(&listing);
    let replacement = listed(&listing, &new_id);
    assert_eq!(value_after(replacement, "Service: "), "trail-reader");
    assert_eq!(value_after(replacement, "Description: "), "reads the trail");
    assert_eq!(value_after(replacement, "Status: "), "Active Active");
    assert_eq!(value_after(replacement, "Super Admin: "), "Error No");
    assert_eq!(
        value_after(replacement, "Permissions: "),
        "[ViewAuditLogs]",
        "the permissions the token held, not a new token's defaults"
    );
    assert_ne!(
        value_after(replacement, "Expires: "),
        "Never",
        "a token without the super-admin flag expires"
    );
    assert_eq!(
        value_after(listed(&listing, &old_id), "Status: "),
        "Inactive Inactive",
        "the old token is revoked"
    );

    assert!(
        !again.status.success(),
        "a revoked token is not rotated: {}",
        printed(&again)
    );
    assert!(
        printed(&again).contains("cannot be rotated"),
        "{}",
        printed(&again)
    );
    assert!(
        printed(&again).contains(&new_id) && !printed(&again).contains("mint a new one"),
        "a second rotation names the replacement and asks for no new token: {}",
        printed(&again)
    );
}

/// `token revoke` on a rotated token fails and names the replacement
/// (carnet#1589). It used to warn that the token was already inactive and
/// exit 0, so a script that revoked the old id read a credential as ended
/// while its replacement stayed live. The replacement is revoked like any
/// token, and revoking it again, a token that names no replacement, keeps
/// the idempotent success.
#[test]
fn revoking_a_rotated_token_fails_and_names_its_replacement() {
    let path = env::temp_dir().join(format!("pierre-cli-revoke-rotated-{}.db", Uuid::new_v4()));
    let url = format!("sqlite:{}?mode=rwc", path.display());
    let run = |args: &[&str]| {
        Command::new(cli_binary())
            .env("DATABASE_URL", &url)
            .env("PIERRE_MASTER_ENCRYPTION_KEY", MASTER_KEY)
            .args(args)
            .output()
            .unwrap()
    };
    let created = run(&[
        "user",
        "create",
        "--email",
        "admin@example.com",
        "--password",
        "correct-horse-battery-staple",
    ]);
    assert!(created.status.success(), "{}", printed(&created));
    let generated = run(&["token", "generate", "--service", "nightly-job"]);
    assert!(generated.status.success(), "{}", printed(&generated));
    let old_id = value_after(&printed(&generated), "Token ID: ").to_owned();
    let rotated = run(&["token", "rotate", &old_id]);
    assert!(rotated.status.success(), "{}", printed(&rotated));
    let new_id = value_after(&printed(&rotated), "New Token: ")
        .trim_end_matches(" (active)")
        .to_owned();

    let revoke_the_old = run(&["token", "revoke", &old_id]);
    let listing_after_refusal = run(&["token", "list", "--include-inactive", "--detailed"]);
    let revoke_the_new = run(&["token", "revoke", &new_id]);
    let revoke_the_new_again = run(&["token", "revoke", &new_id]);
    let listing = run(&["token", "list", "--include-inactive", "--detailed"]);
    // Best-effort cleanup of the temp files.
    drop(fs::remove_file(&path));

    let refusal = printed(&revoke_the_old);
    assert!(
        !revoke_the_old.status.success(),
        "a rotated token is not revoked again: {refusal}"
    );
    assert!(
        refusal.contains(&new_id) && refusal.contains("revoke that one"),
        "the refusal names the replacement and what to do: {refusal}"
    );
    let listing_after_refusal = printed(&listing_after_refusal);
    assert_eq!(
        value_after(listed(&listing_after_refusal, &new_id), "Status: "),
        "Active Active",
        "the refused revoke left the replacement live"
    );
    assert_eq!(
        value_after(listed(&listing_after_refusal, &old_id), "Rotated To: "),
        new_id,
        "the listing names what replaced the rotated token"
    );

    assert!(
        revoke_the_new.status.success(),
        "{}",
        printed(&revoke_the_new)
    );
    assert!(
        revoke_the_new_again.status.success(),
        "a token revoked earlier keeps the idempotent answer: {}",
        printed(&revoke_the_new_again)
    );
    assert_eq!(
        value_after(listed(&printed(&listing), &new_id), "Status: "),
        "Inactive Inactive"
    );
}
