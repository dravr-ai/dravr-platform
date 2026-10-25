// ABOUTME: `pierre-cli user code` — mint, list and revoke signup codes over the admin API
// ABOUTME: A minted code is printed once; the server keeps only its hash (carnet#1128)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Signup codes from the command line.
//!
//! An admin mints a code here and hands it out; whoever enters it at sign-up
//! or on the waiting screen is approved. `list` shows every code, coaches'
//! included, with the accounts that came in on each; `revoke` withdraws one.

use std::io;

use clap::{Subcommand, ValueEnum};
use pierre_core::errors::AppResult;
use secrecy::SecretString;
use serde_json::{json, Value};
use uuid::Uuid;

use pierre_cli::remote::RemoteClient;

use super::auth::admin_client;
use super::user_admin::{stdout_written, OutputFormat};

/// Admin API path of the signup codes.
const CODES_PATH: &str = "/admin/signup-codes";

/// Which kind of code `revoke` names: the admin API path segment.
#[derive(Clone, Copy, ValueEnum)]
pub(crate) enum CodeKind {
    /// A code an admin minted
    Admin,
    /// A coach's approving group invite
    Coach,
}

impl CodeKind {
    /// The path segment the admin API names this kind by.
    const fn path_segment(self) -> &'static str {
        match self {
            Self::Admin => "admin",
            Self::Coach => "coach",
        }
    }
}

/// Verbs for `user code`.
#[derive(Subcommand)]
pub(crate) enum CodeCommand {
    /// Mint a signup code: whoever enters it is approved. Printed once.
    Mint {
        /// Cohort or reason, shown in the listing
        #[arg(long)]
        note: Option<String>,
        /// Readable prefix (letters and digits), e.g. BETA
        #[arg(long)]
        prefix: Option<String>,
        /// Accounts it may approve (default: unlimited)
        #[arg(long)]
        max_uses: Option<i32>,
        /// Days until it stops working (default: never)
        #[arg(long)]
        expires_days: Option<i64>,
        /// Server base URL (defaults to the cached login)
        #[arg(long, env = "PIERRE_SERVER")]
        server: Option<String>,
        /// Admin token (defaults to the cached login)
        #[arg(long, env = "PIERRE_TOKEN", hide_env_values = true)]
        token: Option<SecretString>,
    },
    /// List every signup code, admins' and coaches', with who came in on each
    List {
        /// Output format: table | json
        #[arg(long, default_value = "table")]
        format: String,
        /// Server base URL (defaults to the cached login)
        #[arg(long, env = "PIERRE_SERVER")]
        server: Option<String>,
        /// Admin token (defaults to the cached login)
        #[arg(long, env = "PIERRE_TOKEN", hide_env_values = true)]
        token: Option<SecretString>,
    },
    /// Revoke a signup code; accounts it approved stay approved
    Revoke {
        /// The code's id, from `user code list`
        id: Uuid,
        /// Which kind it is, from `user code list`
        #[arg(long, value_enum, default_value_t = CodeKind::Admin)]
        kind: CodeKind,
        /// Server base URL (defaults to the cached login)
        #[arg(long, env = "PIERRE_SERVER")]
        server: Option<String>,
        /// Admin token (defaults to the cached login)
        #[arg(long, env = "PIERRE_TOKEN", hide_env_values = true)]
        token: Option<SecretString>,
    },
}

/// Run one `user code` verb.
///
/// # Errors
///
/// Returns the client's error when the request fails or the server refuses it.
pub(crate) async fn run(action: CodeCommand) -> AppResult<()> {
    match action {
        CodeCommand::Mint {
            note,
            prefix,
            max_uses,
            expires_days,
            server,
            token,
        } => {
            let client = admin_client(server, token)?;
            mint(&client, note, prefix, max_uses, expires_days).await
        }
        CodeCommand::List {
            format,
            server,
            token,
        } => {
            let client = admin_client(server, token)?;
            list(&client, OutputFormat::parse(&format)?).await
        }
        CodeCommand::Revoke {
            id,
            kind,
            server,
            token,
        } => {
            let client = admin_client(server, token)?;
            let body: Value = client
                .delete_json(&format!("{CODES_PATH}/{}/{id}", kind.path_segment()))
                .await?;
            println!("{}", message(&body));
            Ok(())
        }
    }
}

async fn mint(
    client: &RemoteClient,
    note: Option<String>,
    prefix: Option<String>,
    max_uses: Option<i32>,
    expires_days: Option<i64>,
) -> AppResult<()> {
    let payload = json!({
        "note": note,
        "prefix": prefix,
        "max_uses": max_uses,
        "expires_in_days": expires_days,
    });
    let body: Value = client.post_json(CODES_PATH, &payload).await?;
    let code = body
        .pointer("/data/code")
        .and_then(Value::as_str)
        .unwrap_or("-");
    println!("  {}", message(&body));
    println!();
    println!("  {code}");
    println!();
    println!(
        "  uses: {}   expires: {}",
        text(body.pointer("/data/record/max_uses"), "unlimited"),
        text(body.pointer("/data/record/expires_at"), "never"),
    );
    Ok(())
}

async fn list(client: &RemoteClient, format: OutputFormat) -> AppResult<()> {
    let body: Value = client.get_json(CODES_PATH).await?;
    let codes = body
        .pointer("/data/codes")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    stdout_written(write_codes(&mut io::stdout().lock(), &codes, format))
}

/// Print the listing in the requested format.
fn write_codes(out: &mut impl io::Write, codes: &[Value], format: OutputFormat) -> io::Result<()> {
    if !matches!(format, OutputFormat::Table) {
        for code in codes {
            writeln!(out, "{code}")?;
        }
        return Ok(());
    }
    if codes.is_empty() {
        return writeln!(out, "No signup codes");
    }
    writeln!(out, "Signup codes ({} total):", codes.len())?;
    writeln!(
        out,
        "{:<36}  {:<5}  {:<6}  {:<5}  {:<9}  {:<20}  {:<28}  NOTE",
        "ID", "KIND", "ENDS", "LIVE", "USES", "EXPIRES", "MINTED BY"
    )?;
    for code in codes {
        let uses = format!(
            "{}/{}",
            text(code.get("use_count"), "0"),
            text(code.get("max_uses"), "∞")
        );
        let live = if code.get("live").and_then(Value::as_bool) == Some(true) {
            "yes"
        } else {
            "no"
        };
        writeln!(
            out,
            "{:<36}  {:<5}  …{:<5}  {:<5}  {:<9}  {:<20}  {:<28}  {}",
            text(code.get("id"), "-"),
            text(code.get("kind"), "-"),
            text(code.get("code_hint"), "-"),
            live,
            uses,
            text(code.get("expires_at"), "never"),
            text(code.get("created_by_email"), "-"),
            text(code.get("note"), ""),
        )?;
        for who in code
            .get("redeemed_by")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            writeln!(
                out,
                "      ↳ {} ({})",
                text(who.get("email"), "-"),
                text(who.get("status"), "-")
            )?;
        }
    }
    Ok(())
}

/// A JSON value as display text, `fallback` when absent or null.
fn text(value: Option<&Value>, fallback: &str) -> String {
    match value {
        None | Some(Value::Null) => fallback.to_owned(),
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
    }
}

/// The server's message, else the whole body.
fn message(body: &Value) -> String {
    body.get("message")
        .and_then(Value::as_str)
        .map_or_else(|| body.to_string(), str::to_owned)
}
