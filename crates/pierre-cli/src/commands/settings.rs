// ABOUTME: `pierre-cli settings` — remote GET/PUT over /admin/settings/{guardian,harness}
// ABOUTME: Guardian gets typed flags with read-modify-write; harness takes a whole JSON document

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Remote management of server settings: the Guardian security policy and
//! the chat harness configuration.
//!
//! Thin HTTP wrappers over the admin-token twins of the settings endpoints
//! (`/admin/settings/guardian`, `/admin/settings/harness`), authenticated
//! with the cached device-login token from `pierre-cli auth login`. The
//! guardian `set` verb is read-modify-write: it GETs the persisted document,
//! applies only the flags given, and PUTs the result — so concurrent fields
//! keep their values. Fields pinned by `GUARDIAN_*` env vars persist but stay
//! shadowed; the command prints exactly which ones.

use std::io::{self, Read};

use chrono::Utc;
use clap::Subcommand;
use pierre_core::errors::{AppError, AppResult};
use serde::de::DeserializeOwned;
use serde_json::Value;

use pierre_cli::remote::{CachedCredentials, RemoteClient};
use pierre_tool_runtime::guardian::{ExternalSendAllowlist, GuardianConfigDocument};

const GUARDIAN_PATH: &str = "/admin/settings/guardian";
const HARNESS_PATH: &str = "/admin/settings/harness";

/// The six guardian document fields, in display order. `--clear` values are
/// validated against this list so a typo errors instead of silently no-oping.
const GUARDIAN_FIELDS: [&str; 6] = [
    "mode",
    "max_destructive_per_turn",
    "max_writes_per_turn",
    "external_send",
    "tainted_destructive",
    "plan_mode",
];

/// `pierre-cli settings` — server settings surfaces.
#[non_exhaustive]
#[derive(Subcommand)]
pub enum SettingsCommand {
    /// Guardian security policy (mode, budgets, taint severity, plan mode)
    Guardian {
        /// Verb to run against the guardian settings
        #[command(subcommand)]
        action: GuardianAction,
    },
    /// Chat harness configuration (compaction, guardrails, verification)
    Harness {
        /// Verb to run against the harness settings
        #[command(subcommand)]
        action: HarnessAction,
    },
}

/// Verbs for `settings guardian`.
#[non_exhaustive]
#[derive(Subcommand)]
pub enum GuardianAction {
    /// Show the persisted document, effective policy, and per-field sources
    Show,
    /// Update fields; unset flags leave their fields unchanged
    Set {
        /// How denials are applied: off | observe | enforce
        #[arg(long)]
        mode: Option<String>,
        /// Severity for a destructive tool in a tainted turn: log | confirm | deny
        #[arg(long)]
        tainted_destructive: Option<String>,
        /// Plan-then-verify posture: off | enforce
        #[arg(long)]
        plan_mode: Option<String>,
        /// Max `IRREVERSIBLE` tool executions per turn
        #[arg(long)]
        max_destructive_per_turn: Option<u16>,
        /// Max `WRITES_DATA` tool executions per turn (>= 1)
        #[arg(long)]
        max_writes_per_turn: Option<u16>,
        /// External-send tenant allowlist: none | all | comma-separated tenant UUIDs
        #[arg(long)]
        external_send: Option<String>,
        /// Reset a field to follow the compiled-in default (repeatable)
        #[arg(long, value_name = "FIELD")]
        clear: Vec<String>,
    },
}

/// Verbs for `settings harness`.
#[non_exhaustive]
#[derive(Subcommand)]
pub enum HarnessAction {
    /// Show the harness config document and its source
    Show,
    /// Replace the harness config document with the given JSON (`-` reads stdin)
    Set {
        /// The full document JSON, or `-` to read it from stdin
        json: String,
    },
}

fn client() -> AppResult<RemoteClient> {
    let creds = CachedCredentials::require(Utc::now().timestamp())?;
    RemoteClient::from_cached(&creds)
}

/// Dispatch a `settings` subcommand.
///
/// # Errors
/// Returns an error if not logged in, the server rejects the request, or a
/// flag value fails to parse.
pub async fn dispatch(command: SettingsCommand) -> AppResult<()> {
    match command {
        SettingsCommand::Guardian { action } => match action {
            GuardianAction::Show => guardian_show().await,
            GuardianAction::Set {
                mode,
                tainted_destructive,
                plan_mode,
                max_destructive_per_turn,
                max_writes_per_turn,
                external_send,
                clear,
            } => {
                guardian_set(GuardianSetArgs {
                    mode,
                    tainted_destructive,
                    plan_mode,
                    max_destructive_per_turn,
                    max_writes_per_turn,
                    external_send,
                    clear,
                })
                .await
            }
        },
        SettingsCommand::Harness { action } => match action {
            HarnessAction::Show => harness_show().await,
            HarnessAction::Set { json } => harness_set(json).await,
        },
    }
}

async fn guardian_show() -> AppResult<()> {
    let response = client()?.get_json(GUARDIAN_PATH).await?;
    print_guardian(&response);
    Ok(())
}

/// Flag bundle for `guardian set`, so the apply step reads as one unit.
struct GuardianSetArgs {
    mode: Option<String>,
    tainted_destructive: Option<String>,
    plan_mode: Option<String>,
    max_destructive_per_turn: Option<u16>,
    max_writes_per_turn: Option<u16>,
    external_send: Option<String>,
    clear: Vec<String>,
}

impl GuardianSetArgs {
    fn is_empty(&self) -> bool {
        self.mode.is_none()
            && self.tainted_destructive.is_none()
            && self.plan_mode.is_none()
            && self.max_destructive_per_turn.is_none()
            && self.max_writes_per_turn.is_none()
            && self.external_send.is_none()
            && self.clear.is_empty()
    }
}

async fn guardian_set(args: GuardianSetArgs) -> AppResult<()> {
    if args.is_empty() {
        return Err(AppError::invalid_input(
            "nothing to change — pass at least one --<field> flag or --clear <field>",
        ));
    }
    for field in &args.clear {
        if !GUARDIAN_FIELDS.contains(&field.as_str()) {
            return Err(AppError::invalid_input(format!(
                "--clear {field}: unknown field (valid: {})",
                GUARDIAN_FIELDS.join(", ")
            )));
        }
    }

    let client = client()?;

    // Read-modify-write: start from the persisted document so unset flags
    // keep their stored values.
    let current = client.get_json(GUARDIAN_PATH).await?;
    let mut document: GuardianConfigDocument = serde_json::from_value(
        current.get("config").cloned().unwrap_or(Value::Null),
    )
    .map_err(|e| {
        AppError::invalid_input(format!(
            "server returned an unparseable guardian config document: {e}"
        ))
    })?;

    if let Some(raw) = args.mode {
        document.mode = Some(parse_wire("mode", &raw)?);
    }
    if let Some(raw) = args.tainted_destructive {
        document.tainted_destructive = Some(parse_wire("tainted_destructive", &raw)?);
    }
    if let Some(raw) = args.plan_mode {
        document.plan_mode = Some(parse_wire("plan_mode", &raw)?);
    }
    if let Some(n) = args.max_destructive_per_turn {
        document.max_destructive_per_turn = Some(n);
    }
    if let Some(n) = args.max_writes_per_turn {
        document.max_writes_per_turn = Some(n);
    }
    if let Some(raw) = args.external_send {
        document.external_send = Some(parse_external_send(&raw)?);
    }
    for field in &args.clear {
        clear_field(&mut document, field);
    }

    let response = client.put_json(GUARDIAN_PATH, &document).await?;
    println!("  Guardian config updated.");
    print_guardian(&response);
    Ok(())
}

fn clear_field(document: &mut GuardianConfigDocument, field: &str) {
    match field {
        "mode" => document.mode = None,
        "max_destructive_per_turn" => document.max_destructive_per_turn = None,
        "max_writes_per_turn" => document.max_writes_per_turn = None,
        "external_send" => document.external_send = None,
        "tainted_destructive" => document.tainted_destructive = None,
        "plan_mode" => document.plan_mode = None,
        // Unreachable: names are validated against GUARDIAN_FIELDS above.
        _ => {}
    }
}

/// Parse an enum flag through its serde wire form so the CLI accepts exactly
/// what the server does, and the error lists the valid values.
fn parse_wire<T: DeserializeOwned>(field: &str, raw: &str) -> AppResult<T> {
    serde_json::from_value(Value::String(raw.trim().to_ascii_lowercase()))
        .map_err(|e| AppError::invalid_input(format!("--{field} {raw}: {e}")))
}

fn parse_external_send(raw: &str) -> AppResult<ExternalSendAllowlist> {
    let trimmed = raw.trim();
    let value = if trimmed.eq_ignore_ascii_case("none") || trimmed.eq_ignore_ascii_case("all") {
        Value::String(trimmed.to_ascii_lowercase())
    } else {
        Value::Array(
            trimmed
                .split(',')
                .map(|s| Value::String(s.trim().to_owned()))
                .collect(),
        )
    };
    serde_json::from_value(value).map_err(|e| {
        AppError::invalid_input(format!(
            "--external-send {raw}: {e} (expected none, all, or comma-separated tenant UUIDs)"
        ))
    })
}

fn print_guardian(response: &Value) {
    let source = response
        .get("source")
        .and_then(Value::as_str)
        .unwrap_or("?");
    let updated = response
        .get("updated_at")
        .and_then(Value::as_str)
        .unwrap_or("never");
    println!("  Guardian policy (document: {source}, updated: {updated})");
    println!("  {:<26}  {:<38}  SOURCE", "FIELD", "EFFECTIVE");
    for field in GUARDIAN_FIELDS {
        let effective = display_value(response.pointer(&format!("/effective/{field}")));
        let field_source = response
            .pointer(&format!("/sources/{field}"))
            .and_then(Value::as_str)
            .unwrap_or("?");
        println!("  {field:<26}  {effective:<38}  {field_source}");
    }
    let pinned: Vec<&str> = response
        .get("env_pinned")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    if !pinned.is_empty() {
        println!(
            "  NOTE: env-pinned fields shadow admin edits until the GUARDIAN_* var is unset: {}",
            pinned.join(", ")
        );
    }
}

fn display_value(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Array(a)) => format!("{} tenant(s)", a.len()),
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
        .unwrap_or("never");
    println!("  Harness config (document: {source}, updated: {updated})");
    let config = response.get("config").cloned().unwrap_or(Value::Null);
    // The harness document is nested and operator-edited as a whole; pretty
    // JSON is the faithful view (and pipes cleanly into `settings harness set -`).
    println!(
        "{}",
        serde_json::to_string_pretty(&config).unwrap_or_else(|_| config.to_string())
    );
    Ok(())
}

async fn harness_set(json: String) -> AppResult<()> {
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

    let response = client()?.put_json(HARNESS_PATH, &document).await?;
    let updated = response
        .get("updated_at")
        .and_then(Value::as_str)
        .unwrap_or("?");
    println!("  Harness config updated ({updated}).");
    Ok(())
}
