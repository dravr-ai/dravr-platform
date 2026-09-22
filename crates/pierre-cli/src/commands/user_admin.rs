// ABOUTME: `user get` / `set` / `disconnect` / `delete` — the operator's view of users over the admin API
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
//! `disconnect` and `delete` hand the server's disconnect chokepoint the work,
//! so every grant they drop is revoked at the provider too.

use std::fmt::Write as _;
use std::io::{self, Write as _};

use clap::Args;
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

/// Every status the admin listing filters by, in the order a lookup walks them.
const USER_STATUSES: [&str; 3] = ["active", "pending", "suspended"];

/// Resolve an email or id to a user id.
///
/// The admin routes are keyed by id, but an operator reaches for the email —
/// it is what they see in Telegram, in a support thread, and in this command's
/// own listing. A value that parses as a UUID is taken as an id; anything else
/// is looked up by paging the listing, so `user set jf@dravr.ai --tier ...`
/// works without a round trip through the UI to copy an id. Every status is
/// searched, because the listing defaults to active and the account an
/// operator is about to delete is often suspended.
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
    for status in USER_STATUSES {
        if let Some(id) = find_user_in_status(client, &wanted, status).await? {
            return Ok(id);
        }
    }
    Err(AppError::not_found(format!("User with email {selector}")))
}

/// Page one status of the listing for a lowercase email.
async fn find_user_in_status(
    client: &RemoteClient,
    wanted: &str,
    status: &str,
) -> AppResult<Option<String>> {
    let mut cursor: Option<String> = None;
    loop {
        let mut path = format!("/admin/users?status={status}&limit=100&");
        if let Some(c) = cursor.as_deref() {
            let _ = write!(path, "cursor={c}&");
        }
        let body: Value = client.get_json(&path).await?;
        let data = body.get("data").unwrap_or(&Value::Null);
        if let Some(users) = data.get("users").and_then(Value::as_array) {
            for u in users {
                if field(u, "email").to_ascii_lowercase() == wanted {
                    return Ok(Some(field(u, "id")));
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
            return Ok(None);
        }
    }
}

/// `user disconnect` — disconnect one provider for a user.
#[derive(Debug, Args)]
pub struct DisconnectArgs {
    /// Email (or user id) of the account
    #[arg(long)]
    pub email: String,

    /// Provider to disconnect, e.g. strava
    #[arg(long)]
    pub provider: String,

    /// Server base URL (defaults to the cached login)
    #[arg(long)]
    pub server: Option<String>,

    /// Admin token (defaults to the cached login)
    #[arg(long)]
    pub token: Option<String>,
}

/// `user delete` — remove a user completely.
#[derive(Debug, Args)]
pub struct DeleteArgs {
    /// Email (or user id) of the account
    #[arg(long)]
    pub email: String,

    /// Reason recorded with the deletion
    #[arg(long)]
    pub reason: Option<String>,

    /// Delete for real. Without it, print what would be removed and exit
    /// non-zero, deleting nothing.
    #[arg(long)]
    pub yes: bool,

    /// Server base URL (defaults to the cached login)
    #[arg(long)]
    pub server: Option<String>,

    /// Admin token (defaults to the cached login)
    #[arg(long)]
    pub token: Option<String>,
}

/// One `{tenant_id, provider}` entry as a readable line.
fn provider_line(entry: &Value) -> String {
    format!(
        "{} (tenant {})",
        field(entry, "provider"),
        field(entry, "tenant_id")
    )
}

/// One disconnected `{tenant_id, provider, revocation}` entry as a readable
/// line. Only a `revoked` status is reported as revoked: anything else,
/// including a status this build does not know, says the grant may still be
/// authorized at the provider.
fn disconnected_line(entry: &Value) -> String {
    let held = provider_line(entry);
    let revocation = entry.get("revocation").unwrap_or(&Value::Null);
    match revocation.get("status").and_then(Value::as_str) {
        Some("revoked") => format!("{held} revoked at the provider"),
        Some("no_grant") => format!("{held} disconnected; it held no grant at the provider"),
        _ => {
            let reason = revocation
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or("no confirmation recorded");
            format!(
                "{held} disconnected here, but the provider did not confirm the revocation ({reason}); the grant may still be authorized there"
            )
        }
    }
}

/// The `{tenant_id, provider}` entries under `data.<key>`.
fn provider_entries(body: &Value, key: &str) -> Vec<Value> {
    body.get("data")
        .and_then(|d| d.get(key))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

/// Disconnect one provider for a user over the admin API.
///
/// The server runs the user's own disconnect on their behalf, so the grant is
/// revoked at the provider and the seat is freed on both sides; each tenant's
/// line says whether the provider confirmed the revocation.
///
/// # Errors
///
/// Returns the client's error when the call fails, including the server's 404
/// for a provider the user holds no connection to.
pub async fn disconnect_provider(
    client: &RemoteClient,
    user_id: &str,
    provider: &str,
) -> AppResult<()> {
    let encoded = urlencoding::encode(provider);
    let body: Value = client
        .delete_json(&format!("/admin/users/{user_id}/providers/{encoded}"))
        .await?;
    println!("{}", message_or_json(&body));
    for entry in provider_entries(&body, "disconnected") {
        println!("  - {}", disconnected_line(&entry));
    }
    Ok(())
}

/// Remove a user completely over the admin API.
///
/// Without `confirmed`, reads the user and prints what a delete would remove —
/// the account and each connected provider, revoked at the provider — then
/// fails, so a script that forgot `--yes` exits non-zero having deleted
/// nothing.
///
/// # Errors
///
/// Returns [`AppError::invalid_input`] when not confirmed, and the client's
/// error when a call fails — including the server's 409 naming the groups or
/// operator records that must be reassigned first.
pub async fn delete_user(
    client: &RemoteClient,
    user_id: &str,
    reason: Option<&str>,
    confirmed: bool,
) -> AppResult<()> {
    if !confirmed {
        let body: Value = client.get_json(&format!("/admin/users/{user_id}")).await?;
        let user = body.get("data").unwrap_or(&Value::Null);
        println!(
            "Would delete {} ({}) and everything the account owns.",
            field(user, "email"),
            field(user, "id")
        );
        let providers = provider_entries(&body, "connected_providers");
        if providers.is_empty() {
            println!("  No connected providers.");
        }
        for entry in &providers {
            println!(
                "  - {} would be disconnected and revoked at the provider",
                provider_line(entry)
            );
        }
        println!("Re-run with --yes to delete.");
        return Err(AppError::invalid_input(
            "user delete not confirmed: nothing was deleted (re-run with --yes)",
        ));
    }

    let payload = json!({ "reason": reason });
    let body: Value = client
        .delete_json_with_body(&format!("/admin/users/{user_id}"), &payload)
        .await?;
    println!("{}", message_or_json(&body));
    for entry in provider_entries(&body, "disconnected") {
        println!("  - {}", disconnected_line(&entry));
    }
    for entry in provider_entries(&body, "not_revocable") {
        println!(
            "  - {} removed locally; this server cannot revoke it upstream",
            provider_line(&entry)
        );
    }
    Ok(())
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
pub async fn allow_email(
    client: &RemoteClient,
    email: &str,
    note: Option<&str>,
    send_invite: bool,
) -> AppResult<()> {
    let payload = json!({ "email": email, "note": note, "send_invite": send_invite });
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
    raw.get(..16)
        .map_or_else(|| raw.clone(), |s| s.replace('T', " "))
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
