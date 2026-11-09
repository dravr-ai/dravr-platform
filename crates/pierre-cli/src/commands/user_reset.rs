// ABOUTME: `user reset` — start a whole account over through the admin API: every provider disconnected, every owned row wiped
// ABOUTME: The account, its sign-in, role, tier and operator overrides stay; without --yes it only says what would happen

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `pierre-cli user reset --email <email|id>` drives
//! `POST /admin/users/{id}/reset`, a super-admin route. Between
//! `reset-onboarding`, which only sends an account back through the wizard,
//! and `delete`, which ends it: the account stays and everything it owns goes.

use clap::Args;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::UserReference;
use secrecy::SecretString;
use serde_json::{json, Value};

use pierre_cli::remote::RemoteClient;

use super::user_admin::{self, CLIENT_FLAGS_NOTE};

/// `user reset` — start a whole account over.
#[derive(Debug, Args)]
pub(crate) struct ResetArgs {
    /// Email (or user id) of the account
    #[arg(long)]
    pub email: String,

    /// Reason recorded with the reset
    #[arg(long)]
    pub reason: Option<String>,

    /// Reset for real. Without it, print what would change and exit
    /// non-zero, changing nothing.
    #[arg(long)]
    pub yes: bool,

    /// Server base URL (defaults to the cached login)
    #[arg(long, env = "PIERRE_SERVER")]
    pub server: Option<String>,

    /// Admin token (defaults to the cached login)
    #[arg(long, env = "PIERRE_TOKEN", hide_env_values = true)]
    pub token: Option<SecretString>,
}

/// What a reset deletes beyond the providers, as the dry run states it: the
/// lines naming what reaches other people come first.
const WIPED_LINES: &[&str] = &[
    "group memberships: the account leaves every group it belongs to",
    "coach links on both sides (delegated connections, reply reviews, attention dismissals), as the athlete and as the coach",
    "activities, health data, plans, goals, memory, notifications, devices, private agents and preferences",
    "the profile, physiology, onboarding progress and gear",
    "conversations the account shares with nobody, and its verdicts and ratings wherever it left them",
    "A2A clients, with their OAuth registrations, sessions and tasks",
];

/// What a reset keeps, as the dry run states it.
const KEPT_NOTE: &str = "  Kept: the account itself (id, email, sign-in, role, tier), its tenant memberships, \
every operator override, its own API keys and connected MCP clients, threads it \
shares with another participant (with everything posted in them), and the ledgers (audit trail, usage, \
LLM cost, billing). No tombstone and no deletion receipt is written.";

/// What a reset takes from the messaging channels.
const CHANNELS_NOTE: &str =
    "WhatsApp, Slack and other channel links are wiped: the athlete links each channel again.";

/// The `TrainingPeaks` provider as the user read names a held connection.
const TRAININGPEAKS: &str = "trainingpeaks";

/// `user reset`: resolve the account and reset it, as [`reset_account`] does.
///
/// # Errors
///
/// Returns the login's error when no server or token is known, the lookup's
/// when no account carries the email, and [`reset_account`]'s.
pub(crate) async fn run(args: ResetArgs) -> AppResult<()> {
    let client = super::auth::admin_client(args.server, args.token)?;
    let id = user_admin::resolve_user_id(&client, &args.email).await?;
    reset_account(&client, &id, args.reason.as_deref(), args.yes).await
}

/// Start a whole account over through the admin API.
///
/// Without `confirmed`, reads the user and prints what a reset would do —
/// every connected provider disconnected (coaching platforms included), what
/// it takes from other people (group seats, coach links), every row the
/// account owns wiped, and what stays — or that the server would refuse it and
/// what to reassign first, then fails, so a script that forgot `--yes` exits
/// non-zero having changed nothing.
///
/// # Errors
///
/// Returns [`AppError::invalid_input`] when not confirmed, and the client's
/// error when a call fails — including the server's 403 for a token below
/// super-admin and its 409 naming the groups, invites or agents that must be
/// reassigned first.
pub(crate) async fn reset_account(
    client: &RemoteClient,
    user_id: &str,
    reason: Option<&str>,
    confirmed: bool,
) -> AppResult<()> {
    if !confirmed {
        let body: Value = client.get_json(&format!("/admin/users/{user_id}")).await?;
        print_preview(&body)?;
        return Err(AppError::invalid_input(
            "user reset not confirmed: nothing was changed (re-run with --yes)",
        ));
    }

    let payload = json!({ "reason": reason });
    let body: Value = client
        .post_json(&format!("/admin/users/{user_id}/reset"), &payload)
        .await?;
    println!("{}", user_admin::message_or_json(&body));
    for entry in user_admin::provider_entries(&body, "disconnected") {
        println!("  - {}", user_admin::disconnected_line(&entry));
    }
    for entry in user_admin::provider_entries(&body, "not_revocable") {
        println!(
            "  - {} kept: this server cannot revoke it, so its rows stay until it can",
            user_admin::provider_line(&entry)
        );
    }
    let data = body.get("data").unwrap_or(&Value::Null);
    if let Some(rows) = data.get("rows_changed").and_then(Value::as_object) {
        for (table, count) in rows {
            println!("  - {table}: {count}");
        }
    }
    if let Some(erased) = data.get("archive_erased") {
        println!(
            "  - imported history and uploaded files erased: {} records, {} import jobs, {} files",
            erased.get("records").and_then(Value::as_u64).unwrap_or(0),
            erased.get("jobs").and_then(Value::as_u64).unwrap_or(0),
            erased.get("blobs").and_then(Value::as_u64).unwrap_or(0),
        );
    }
    println!("{CHANNELS_NOTE}");
    println!("{CLIENT_FLAGS_NOTE}");
    Ok(())
}

/// Print what a reset of the account `body` reads would do — or, when the
/// server names something that refuses it, that it would be refused and what
/// to reassign first — changing nothing.
///
/// # Errors
///
/// Returns an internal error when the server's `reset_blockers` is missing
/// or cannot be read as references.
fn print_preview(body: &Value) -> AppResult<()> {
    let user = body.get("data").unwrap_or(&Value::Null);
    // An absent list is not an empty one: previewing a wipe the server would
    // refuse is the thing this read exists to prevent.
    let listed = user.get("reset_blockers").ok_or_else(|| {
        AppError::internal("the server did not say what would refuse a reset (no reset_blockers)")
    })?;
    let blockers: Vec<UserReference> = serde_json::from_value(listed.clone()).map_err(|e| {
        AppError::internal(format!("the server's reset blockers are unreadable: {e}"))
    })?;
    if blockers.is_empty() {
        println!(
            "Would reset {} ({}): everything the account owns would be wiped.",
            user_admin::field(user, "email"),
            user_admin::field(user, "id")
        );
    } else {
        println!(
            "Would refuse to reset {} ({}) until these are reassigned or removed:",
            user_admin::field(user, "email"),
            user_admin::field(user, "id")
        );
        for blocker in &blockers {
            println!("  - {}", blocker.describe());
        }
        println!("Once they are, a reset would do this:");
    }
    let providers = user_admin::provider_entries(body, "connected_providers");
    if providers.is_empty() {
        println!("  No connected providers.");
    }
    for entry in &providers {
        println!(
            "  - {} would be disconnected and revoked at the provider (coaching platforms included; one this server cannot revoke is kept and named)",
            user_admin::provider_line(entry)
        );
    }
    let holds_trainingpeaks = providers
        .iter()
        .any(|entry| entry.get("provider").and_then(Value::as_str) == Some(TRAININGPEAKS));
    if holds_trainingpeaks && user.get("manages_roster").and_then(Value::as_bool) == Some(true) {
        if user
            .get("manages_roster_operator_grant")
            .is_some_and(|grant| !grant.is_null())
        {
            println!("  - manages_roster stays: an operator granted it");
        } else {
            println!("  - manages_roster, earned through a TrainingPeaks coach connection, would be revoked with it (an operator grant stays)");
        }
    }
    for line in WIPED_LINES {
        println!("  - {line} would be deleted");
    }
    println!("  - imported history and uploaded files would be erased");
    println!("{KEPT_NOTE}");
    println!("{CHANNELS_NOTE}");
    println!("{CLIENT_FLAGS_NOTE}");
    if blockers.is_empty() {
        println!("Re-run with --yes to reset.");
    } else {
        println!("A reset with --yes is refused (409) until then; nothing is touched.");
    }
    Ok(())
}
