// ABOUTME: `pierre-cli settings` — remote GET/PUT over /admin/settings/{harness,signup}, and the guardian and connect-order verbs' entry
// ABOUTME: Sign-up gets typed flags, harness takes a whole JSON document, and Guardian has its own module

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Remote management of server settings: the Guardian security policy
//! ([`super::guardian_settings`]), the chat harness configuration and the
//! self-serve sign-up bounds.
//!
//! Thin HTTP wrappers over the admin-token twins of the settings endpoints
//! (`/admin/settings/harness`, `/admin/settings/signup`), authenticated with the
//! cached device-login token from `pierre-cli auth login`.

use std::io::{self, Read};

use chrono::Utc;
use clap::Subcommand;
use pierre_core::errors::{AppError, AppResult};
use serde_json::Value;

use pierre_cli::remote::{CachedCredentials, RemoteClient};

use super::{connect_order, guardian_settings};

const HARNESS_PATH: &str = "/admin/settings/harness";
/// What `settings harness show` prints for a document no row holds, and what
/// `settings harness set --expected-updated-at` takes back.
const NO_ROW_READ: &str = "never";
const SIGNUP_PATH: &str = "/admin/settings/signup";

/// `pierre-cli settings` — server settings surfaces.
#[non_exhaustive]
#[derive(Subcommand)]
pub(crate) enum SettingsCommand {
    /// Which way a new Strava or Garmin connection goes first: OAuth or the
    /// account sign-in (super-admin)
    ConnectOrder {
        /// Verb to run against the connect order
        #[command(subcommand)]
        action: connect_order::ConnectOrderAction,
    },
    /// Guardian security policy (mode and per-turn budgets)
    Guardian {
        /// Verb to run against the guardian settings
        #[command(subcommand)]
        action: guardian_settings::GuardianAction,
    },
    /// Chat harness configuration (compaction, guardrails, verification)
    Harness {
        /// Verb to run against the harness settings
        #[command(subcommand)]
        action: HarnessAction,
    },
    /// Self-serve sign-up: the kill switch, the 24-hour ceiling, activation
    /// on a verified email
    Signup {
        /// Verb to run against the sign-up settings
        #[command(subcommand)]
        action: SignupAction,
    },
}

/// Verbs for `settings signup`.
#[non_exhaustive]
#[derive(Subcommand)]
pub(crate) enum SignupAction {
    /// Show the sign-up settings and the accounts the daily ceiling counts
    Show,
    /// Restart the daily ceiling's count from zero now; the limit is unchanged
    Reset,
    /// Update settings; unset flags leave their settings unchanged
    Set {
        /// The kill switch: `true` admits self-serve sign-ups, `false` refuses them all
        #[arg(long)]
        open: Option<bool>,
        /// The most accounts admitted in any 24 hours (0 sets no ceiling)
        #[arg(long)]
        daily_limit: Option<i64>,
        /// Whether confirming the email activates a self-registered account
        #[arg(long)]
        activate_on_verified_email: Option<bool>,
    },
}

/// Verbs for `settings harness`.
#[non_exhaustive]
#[derive(Subcommand)]
pub(crate) enum HarnessAction {
    /// Show the harness config document and its source
    Show,
    /// Replace the harness config document with the given JSON (`-` reads stdin)
    Set {
        /// The full document JSON, or `-` to read it from stdin
        json: String,
        /// The `updated:` value `settings harness show` printed for the document
        /// this was edited from (`never` when no row existed). A save over a row
        /// that has moved since is refused
        #[arg(long)]
        expected_updated_at: String,
    },
}

pub(super) fn client() -> AppResult<RemoteClient> {
    let creds = CachedCredentials::require(Utc::now().timestamp())?;
    RemoteClient::from_cached(&creds)
}

/// Dispatch a `settings` subcommand.
///
/// # Errors
/// Returns an error if not logged in, the server rejects the request, or a
/// flag value fails to parse.
pub(crate) async fn dispatch(command: SettingsCommand) -> AppResult<()> {
    match command {
        SettingsCommand::ConnectOrder { action } => connect_order::dispatch(action).await,
        SettingsCommand::Guardian { action } => guardian_settings::dispatch(action).await,
        SettingsCommand::Harness { action } => match action {
            HarnessAction::Show => harness_show().await,
            HarnessAction::Set {
                json,
                expected_updated_at,
            } => harness_set(json, &expected_updated_at).await,
        },
        SettingsCommand::Signup { action } => match action {
            SignupAction::Show => signup_show().await,
            SignupAction::Reset => signup_reset().await,
            SignupAction::Set {
                open,
                daily_limit,
                activate_on_verified_email,
            } => signup_set(open, daily_limit, activate_on_verified_email).await,
        },
    }
}

pub(super) fn display_value(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Array(a)) => a
            .iter()
            .map(|item| {
                item.as_str()
                    .map_or_else(|| item.to_string(), str::to_owned)
            })
            .collect::<Vec<_>>()
            .join(","),
        Some(other) => other.to_string(),
        None => "?".to_owned(),
    }
}

async fn harness_show() -> AppResult<()> {
    let response = client()?.get_json(HARNESS_PATH).await?;
    let source = response
        .get("source")
        .and_then(Value::as_str)
        .unwrap_or("?");
    let updated = response
        .get("updated_at")
        .and_then(Value::as_str)
        .unwrap_or(NO_ROW_READ);
    println!("  Harness config (document: {source}, updated: {updated})");
    if let Some(reason) = response.get("degraded_reason").and_then(Value::as_str) {
        println!("  Degraded: {reason}. A save replaces the stored row.");
    }
    let config = response.get("config").cloned().unwrap_or(Value::Null);
    // The harness document is nested and operator-edited as a whole; pretty
    // JSON is the faithful view (and pipes cleanly into `settings harness set -`).
    println!(
        "{}",
        serde_json::to_string_pretty(&config).unwrap_or_else(|_| config.to_string())
    );
    Ok(())
}

async fn harness_set(json: String, expected_updated_at: &str) -> AppResult<()> {
    let raw = if json == "-" {
        let mut buf = String::new();
        io::stdin()
            .read_to_string(&mut buf)
            .map_err(|e| AppError::invalid_input(format!("failed to read stdin: {e}")))?;
        buf
    } else {
        json
    };
    // Syntax-check client-side so a malformed pipe fails before the network;
    // semantic validation stays server-side (single source of truth).
    let document: Value = serde_json::from_str(&raw)
        .map_err(|e| AppError::invalid_input(format!("document is not valid JSON: {e}")))?;

    let client = client()?;
    let body = harness_put_body(document, expected_updated_at)?;
    let reply = client.put_raw(HARNESS_PATH, &body).await?;
    if reply.status().as_u16() == 409 {
        let refusal = reply.json::<Value>().await.unwrap_or(Value::Null);
        return Err(AppError::invalid_input(
            guardian_settings::conflict_message(&refusal),
        ));
    }
    let response = RemoteClient::json_or_error(HARNESS_PATH, reply).await?;
    let updated = response
        .get("updated_at")
        .and_then(Value::as_str)
        .unwrap_or("?");
    println!("  Harness config updated ({updated}).");
    Ok(())
}

/// The PUT body: the document beside the version it was edited from, as
/// `settings harness show` printed it. Reading the version here instead would
/// protect nothing: a whole document was edited from an earlier read, so a save
/// landing since then would be overwritten without a word (carnet#1418).
fn harness_put_body(mut document: Value, expected_updated_at: &str) -> AppResult<Value> {
    let fields = document
        .as_object_mut()
        .ok_or_else(|| AppError::invalid_input("document must be a JSON object"))?;
    let version = match expected_updated_at {
        NO_ROW_READ => Value::Null,
        stamp => Value::String(stamp.to_owned()),
    };
    fields.insert("expected_updated_at".to_owned(), version);
    Ok(document)
}

async fn signup_show() -> AppResult<()> {
    let response = client()?.get_json(SIGNUP_PATH).await?;
    print_signup(&response);
    Ok(())
}

async fn signup_reset() -> AppResult<()> {
    let response = client()?
        .post_json(&format!("{SIGNUP_PATH}/reset"), &serde_json::json!({}))
        .await?;
    println!("  Sign-up daily count reset.");
    print_signup(&response);
    Ok(())
}

async fn signup_set(
    open: Option<bool>,
    daily_limit: Option<i64>,
    activate_on_verified_email: Option<bool>,
) -> AppResult<()> {
    if open.is_none() && daily_limit.is_none() && activate_on_verified_email.is_none() {
        return Err(AppError::invalid_input(
            "nothing to change — pass --open, --daily-limit or --activate-on-verified-email",
        ));
    }
    let body = serde_json::json!({
        "open": open,
        "daily_limit": daily_limit,
        "activate_on_verified_email": activate_on_verified_email,
    });
    let response = client()?.put_json(SIGNUP_PATH, &body).await?;
    println!("  Sign-up settings updated.");
    print_signup(&response);
    Ok(())
}

fn print_signup(response: &Value) {
    let field = |name: &str| display_value(response.pointer(&format!("/data/{name}")));
    let daily_limit = match response
        .pointer("/data/daily_limit")
        .and_then(Value::as_i64)
    {
        Some(0) => "none".to_owned(),
        _ => field("daily_limit"),
    };
    println!("  Self-serve sign-up");
    println!("  {:<28}  {}", "open", field("open"));
    println!("  {:<28}  {daily_limit}", "daily_limit");
    println!(
        "  {:<28}  {}",
        "activate_on_verified_email",
        field("activate_on_verified_email")
    );
    println!(
        "  {:<28}  {} since {}",
        "counted",
        field("counted"),
        field("counted_since")
    );
    println!(
        "  {:<28}  {}",
        "turnstile_configured",
        field("turnstile_configured")
    );
}
