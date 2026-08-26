// ABOUTME: `user get` / `user set` — the operator's view of users and tiers over the admin API
// ABOUTME: HTTP rather than a direct DB handle, so one binary serves local and deployed alike

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Standard get/set access to users.
//!
//! Every other `pierre-cli` user command holds a [`RepositoryRegistry`] and
//! talks to `DATABASE_URL` directly. That works on a laptop and cannot reach a
//! deployed environment at all: dev's Cloud SQL is on a private IP, so
//! answering "who is on which tier" there meant hand-writing SQL through a
//! Cloud Run job. These two commands go over the admin API instead, so the same
//! binary serves local and dev with nothing but a different `--server`.
//!
//! `get` with no selector lists; `get <email|id>` reads one; `set` writes. The
//! listing pages with an opaque cursor and prints each page as it arrives, so
//! `--all` streams rather than accumulating — the table only grows.

use std::fmt::Write as _;
use std::io::{self, Write as _};

use pierre_core::errors::{AppError, AppResult};
use serde_json::{json, Value};

use pierre_cli::remote::RemoteClient;

/// How a listing is printed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    /// Aligned columns for a human.
    Table,
    /// One JSON object per line, for piping into `jq`.
    Json,
    /// Comma-separated, header first.
    Csv,
}

impl OutputFormat {
    /// Parse the `--format` flag.
    ///
    /// # Errors
    ///
    /// Returns [`AppError::invalid_input`] for an unknown format, naming the
    /// three that exist rather than silently falling back to a default — a
    /// typo'd `--format jsonl` should not quietly print a table.
    pub fn parse(raw: &str) -> AppResult<Self> {
        match raw.to_ascii_lowercase().as_str() {
            "table" => Ok(Self::Table),
            "json" => Ok(Self::Json),
            "csv" => Ok(Self::Csv),
            other => Err(AppError::invalid_input(format!(
                "Unknown format '{other}' — expected table, json, or csv"
            ))),
        }
    }
}

/// Fields printed per user, in order. One list so the table header, the CSV
/// header and the row bodies cannot disagree about which columns exist.
const COLUMNS: [&str; 5] = ["email", "tier", "id", "display_name", "last_active"];

fn field(user: &Value, key: &str) -> String {
    user.get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned()
}

/// Print one page in the requested format.
///
/// `header` is honoured only on the first page: a streamed listing prints as it
/// pages, and repeating the header mid-stream would corrupt the CSV for anything
/// consuming it.
fn print_page(users: &[Value], format: OutputFormat, header: bool) {
    let mut out = io::stdout().lock();
    match format {
        OutputFormat::Table => {
            if header {
                let _ = writeln!(
                    out,
                    "{:<38}  {:<13}  {:<38}  DISPLAY NAME",
                    "EMAIL", "TIER", "ID"
                );
            }
            for u in users {
                let _ = writeln!(
                    out,
                    "{:<38}  {:<13}  {:<38}  {}",
                    field(u, "email"),
                    field(u, "tier"),
                    field(u, "id"),
                    field(u, "display_name")
                );
            }
        }
        OutputFormat::Json => {
            for u in users {
                let _ = writeln!(out, "{u}");
            }
        }
        OutputFormat::Csv => {
            if header {
                let _ = writeln!(out, "{}", COLUMNS.join(","));
            }
            for u in users {
                // Display names are user-supplied and do contain commas.
                let row: Vec<String> = COLUMNS.iter().map(|c| csv_cell(&field(u, c))).collect();
                let _ = writeln!(out, "{}", row.join(","));
            }
        }
    }
    let _ = out.flush();
}

/// Filters and paging for a listing.
pub struct GetUsersArgs {
    /// Status to list. `None` lets the server default to active.
    pub status: Option<String>,
    /// Tier filter, or `None` for every tier.
    pub tier: Option<String>,
    /// Page size requested; the server clamps it.
    pub limit: Option<i32>,
    /// Keep paging until the listing is exhausted.
    pub all: bool,
    /// How to print.
    pub format: OutputFormat,
}

/// List users, paging while the server says there is more.
///
/// # Errors
///
/// Returns the client's error when a page cannot be fetched. A partial listing
/// is not swallowed: pages already printed stay printed, and the error names
/// the page that failed, because silently ending a stream early looks exactly
/// like reaching the end of the table.
pub async fn get_users(client: &RemoteClient, args: &GetUsersArgs) -> AppResult<()> {
    let mut cursor: Option<String> = None;
    let mut page_index = 0_usize;
    let mut printed = 0_usize;

    loop {
        let mut path = String::from("/admin/users?");
        if let Some(status) = args.status.as_deref() {
            let _ = write!(path, "status={status}&");
        }
        if let Some(tier) = args.tier.as_deref() {
            let _ = write!(path, "tier={tier}&");
        }
        if let Some(limit) = args.limit {
            let _ = write!(path, "limit={limit}&");
        }
        if let Some(c) = cursor.as_deref() {
            let _ = write!(path, "cursor={c}&");
        }

        let body: Value = client.get_json(&path).await.map_err(|e| {
            AppError::internal(format!("listing users failed on page {page_index}: {e}"))
        })?;
        let data = body.get("data").unwrap_or(&Value::Null);
        let users = data
            .get("users")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();

        print_page(&users, args.format, page_index == 0);
        printed += users.len();
        page_index += 1;

        let has_more = data
            .get("has_more")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        cursor = data
            .get("next_cursor")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);

        if !args.all || !has_more || cursor.is_none() {
            break;
        }
    }

    if matches!(args.format, OutputFormat::Table) {
        println!("\n{printed} user(s)");
    }
    Ok(())
}

/// Resolve an email or id to a user id.
///
/// The admin routes are keyed by id, but an operator reaches for the email —
/// it is what they see in Telegram, in a support thread, and in this command's
/// own listing. A value that parses as a UUID is taken as an id; anything else
/// is looked up by paging the listing, so `user set jf@dravr.ai --tier ...`
/// works without a round trip through the UI to copy an id.
///
/// # Errors
///
/// Returns [`AppError::not_found`] when no user carries that email, rather than
/// falling through and letting the server 404 on a path built from an email.
pub async fn resolve_user_id(client: &RemoteClient, selector: &str) -> AppResult<String> {
    if uuid::Uuid::parse_str(selector).is_ok() {
        return Ok(selector.to_owned());
    }
    let wanted = selector.to_ascii_lowercase();
    let mut cursor: Option<String> = None;
    loop {
        let mut path = String::from("/admin/users?limit=100&");
        if let Some(c) = cursor.as_deref() {
            let _ = write!(path, "cursor={c}&");
        }
        let body: Value = client.get_json(&path).await?;
        let data = body.get("data").unwrap_or(&Value::Null);
        if let Some(users) = data.get("users").and_then(Value::as_array) {
            for u in users {
                if field(u, "email").to_ascii_lowercase() == wanted {
                    return Ok(field(u, "id"));
                }
            }
        }
        let has_more = data
            .get("has_more")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        cursor = data
            .get("next_cursor")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        if !has_more || cursor.is_none() {
            break;
        }
    }
    Err(AppError::not_found(format!("User with email {selector}")))
}

/// Read one user by id.
///
/// # Errors
///
/// Returns the client's error when the fetch fails.
pub async fn get_user(client: &RemoteClient, user_id: &str) -> AppResult<()> {
    let body: Value = client.get_json(&format!("/admin/users/{user_id}")).await?;
    println!(
        "{}",
        serde_json::to_string_pretty(&body).unwrap_or_default()
    );
    Ok(())
}

/// Set a user's tier.
///
/// # Errors
///
/// Returns [`AppError::invalid_input`] for an unknown tier — rejected here as
/// well as server-side so a typo costs a round trip, not a silent no-op — and
/// the client's error when the write fails.
pub async fn set_user_tier(
    client: &RemoteClient,
    user_id: &str,
    tier: &str,
    note: Option<&str>,
) -> AppResult<()> {
    let parsed = match tier.to_ascii_lowercase().as_str() {
        t @ ("starter" | "professional" | "enterprise") => t.to_owned(),
        other => {
            return Err(AppError::invalid_input(format!(
                "Unknown tier '{other}' — expected starter, professional, or enterprise"
            )));
        }
    };
    let payload = json!({
        "tier": parsed,
        "note": note.unwrap_or("set via pierre-cli user set"),
    });
    let body: Value = client
        .post_json(&format!("/admin/users/{user_id}/tier"), &payload)
        .await?;
    println!(
        "{}",
        serde_json::to_string_pretty(&body).unwrap_or_default()
    );
    Ok(())
}

/// Columns of the pre-approval listing, in order.
const ALLOWED_COLUMNS: [&str; 5] = [
    "email",
    "account_status",
    "created_at",
    "allowed_by_email",
    "note",
];

/// Pre-approve an email address over the admin API.
///
/// # Errors
///
/// Returns the client's error when the write fails, including the server's
/// rejection of a malformed address.
pub async fn allow_email(client: &RemoteClient, email: &str, note: Option<&str>) -> AppResult<()> {
    let payload = json!({ "email": email, "note": note });
    let body: Value = client
        .post_json("/admin/pre-approved-emails", &payload)
        .await?;
    println!("{}", message_or_json(&body));
    Ok(())
}

/// Remove a standing pre-approval over the admin API.
///
/// # Errors
///
/// Returns the client's error when the delete fails.
pub async fn disallow_email(client: &RemoteClient, email: &str) -> AppResult<()> {
    let encoded = urlencoding::encode(email);
    let body: Value = client
        .delete_json(&format!("/admin/pre-approved-emails/{encoded}"))
        .await?;
    println!("{}", message_or_json(&body));
    Ok(())
}

/// List standing pre-approvals with each address's registration state.
///
/// # Errors
///
/// Returns the client's error when the listing fails.
pub async fn list_allowed(client: &RemoteClient, format: OutputFormat) -> AppResult<()> {
    let body: Value = client.get_json("/admin/pre-approved-emails").await?;
    let entries = body
        .get("data")
        .and_then(|d| d.get("emails"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    if entries.is_empty() && matches!(format, OutputFormat::Table) {
        println!("No pre-approved emails");
        return Ok(());
    }

    let mut out = io::stdout().lock();
    match format {
        OutputFormat::Table => {
            let _ = writeln!(out, "Pre-approved emails ({} total):", entries.len());
            let _ = writeln!(
                out,
                "{:<36}  {:<10}  {:<17}  {:<28}  NOTE",
                "EMAIL", "ACCOUNT", "ALLOWED AT", "ALLOWED BY"
            );
            for entry in &entries {
                let _ = writeln!(
                    out,
                    "{:<36}  {:<10}  {:<17}  {:<28}  {}",
                    field(entry, "email"),
                    account_label(entry),
                    allowed_at(entry),
                    operator_label(entry),
                    note_label(entry)
                );
            }
        }
        OutputFormat::Json => {
            for entry in &entries {
                let _ = writeln!(out, "{entry}");
            }
        }
        OutputFormat::Csv => {
            let _ = writeln!(out, "{}", ALLOWED_COLUMNS.join(","));
            for entry in &entries {
                let row: Vec<String> = ALLOWED_COLUMNS
                    .iter()
                    .map(|c| csv_cell(&field(entry, c)))
                    .collect();
                let _ = writeln!(out, "{}", row.join(","));
            }
        }
    }
    let _ = out.flush();
    Ok(())
}

/// The server's own sentence for a write, falling back to the whole body when
/// a response arrives in a shape this build does not know — printing nothing
/// would read as success.
fn message_or_json(body: &Value) -> String {
    body.get("message").and_then(Value::as_str).map_or_else(
        || serde_json::to_string_pretty(body).unwrap_or_default(),
        ToOwned::to_owned,
    )
}

/// `not yet` reads better than an empty cell for an address nobody has
/// registered against — the normal steady state of a standing allow.
fn account_label(entry: &Value) -> String {
    let status = field(entry, "account_status");
    if status.is_empty() {
        "not yet".to_owned()
    } else {
        status
    }
}

/// Minute-precision timestamp; the RFC3339 the server sends is too wide for a
/// column an operator scans.
fn allowed_at(entry: &Value) -> String {
    let raw = field(entry, "created_at");
    raw.get(..16).map_or(raw.clone(), |s| s.replace('T', " "))
}

/// The operator who recorded the allow, or `-` when it was not attributable
/// (a service token, or a pre-bootstrap allow).
fn operator_label(entry: &Value) -> String {
    let email = field(entry, "allowed_by_email");
    if email.is_empty() {
        "-".to_owned()
    } else {
        email
    }
}

/// The operator note, or `-` when none was recorded.
fn note_label(entry: &Value) -> String {
    let note = field(entry, "note");
    if note.is_empty() {
        "-".to_owned()
    } else {
        note
    }
}

/// Quote a cell that would otherwise break the CSV. Notes are operator-supplied
/// prose and do contain commas.
fn csv_cell(value: &str) -> String {
    if value.contains([',', '"', '\n']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_owned()
    }
}
