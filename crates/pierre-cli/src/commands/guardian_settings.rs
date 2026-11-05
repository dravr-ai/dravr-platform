// ABOUTME: `pierre-cli settings guardian` — show, set, history and parked calls over /admin/settings/guardian
// ABOUTME: A save is read-modify-write with the token it read; a refused one lists why and exits non-zero

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Remote management of the Guardian security policy.
//!
//! The `set` verb is read-modify-write: it GETs the persisted document, applies
//! only the flags given, and PUTs the result with the `updated_at` it read, so the
//! server refuses the save if the row changed in between. Fields pinned by
//! `GUARDIAN_*` env vars persist but stay shadowed; the command prints exactly
//! which ones.
//!
//! A deployment runs several instances, and `show` reports the one that answered:
//! `effective`, `loaded_at` and `in_sync` can differ between them for up to a poll
//! interval after a save. The persisted column is the row, the same for all.

use clap::Subcommand;
use pierre_cli::remote::RemoteClient;
use pierre_core::errors::{AppError, AppResult};
use pierre_tool_runtime::guardian::{
    high_impact_changes, validate_document, validate_plan_mode_arming, ExternalSendAllowlist,
    GuardianConfigDocument,
};
use serde::de::DeserializeOwned;
use serde_json::Value;
use uuid::Uuid;

use super::settings::{client, display_value};

const GUARDIAN_PATH: &str = "/admin/settings/guardian";
const GUARDIAN_PENDING_PATH: &str = "/admin/settings/guardian/pending";

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

/// The audit-trail actions `settings guardian history` lists.
const HISTORY_ACTION_PREFIX: &str = "guardian.";
/// The 409 code of a save whose base is stale.
const STALE_CODE: &str = "guardian_config_stale";
/// The 409 code of a harness save whose base is stale, which shares this answer.
const HARNESS_STALE_CODE: &str = "harness_config_stale";
/// The 409 code of a high-impact save nobody acknowledged.
const IMPACT_CODE: &str = "guardian_config_impact_not_acknowledged";

/// What a field with no stored value shows.
const UNSET: &str = "(unset)";

/// Verbs for `settings guardian`.
#[non_exhaustive]
#[derive(Subcommand)]
pub(crate) enum GuardianAction {
    /// Show each field's persisted value beside what the instance that answered
    /// enforces and why, plus that instance's id, sync state and degradation
    Show {
        /// Print the server's raw JSON response
        #[arg(long)]
        json: bool,
    },
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
        /// Confirm a change that alters what athletes can do on every turn; the
        /// server lists those changes when this flag is missing
        #[arg(long)]
        acknowledge_impact: bool,
        /// Print the resolved change and its impact, and write nothing
        #[arg(long)]
        dry_run: bool,
    },
    /// Show recorded Guardian actions, saves and denials, refusals included, newest
    /// first. Needs the `ManageUsers` permission the audit trail asks of a reader
    History {
        /// How many actions to show (the server caps the page)
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
    /// Tool calls the Guardian parked for an athlete's `/confirm`
    Pending {
        /// Verb to run against the parked calls
        #[command(subcommand)]
        action: GuardianPendingAction,
    },
}

/// Verbs for `settings guardian pending`.
#[non_exhaustive]
#[derive(Subcommand)]
pub(crate) enum GuardianPendingAction {
    /// List parked calls, newest first, without their arguments or agent drafts
    List {
        /// One tenant's calls, by UUID; every tenant's when absent
        #[arg(long)]
        tenant: Option<String>,
        /// Which calls to list; `pending` when absent
        #[arg(long, value_parser = ["pending", "confirmed", "denied", "expired"])]
        status: Option<String>,
    },
    /// Deny a parked call. Super admins cannot approve one: only its athlete can
    Deny {
        /// The parked call's id, from `pending list`
        id: String,
    },
}

/// Dispatch a `settings guardian` verb.
///
/// # Errors
/// Returns an error if not logged in, the server rejects the request, or a flag
/// value fails to parse.
pub(super) async fn dispatch(action: GuardianAction) -> AppResult<()> {
    match action {
        GuardianAction::Show { json } => guardian_show(json).await,
        GuardianAction::Set {
            mode,
            tainted_destructive,
            plan_mode,
            max_destructive_per_turn,
            max_writes_per_turn,
            external_send,
            clear,
            acknowledge_impact,
            dry_run,
        } => {
            guardian_set(GuardianSetArgs {
                mode,
                tainted_destructive,
                plan_mode,
                max_destructive_per_turn,
                max_writes_per_turn,
                external_send,
                clear,
                acknowledge_impact,
                dry_run,
            })
            .await
        }
        GuardianAction::History { limit } => guardian_history(limit).await,
        GuardianAction::Pending { action } => match action {
            GuardianPendingAction::List { tenant, status } => {
                guardian_pending_list(tenant.as_deref(), status.as_deref()).await
            }
            GuardianPendingAction::Deny { id } => guardian_pending_deny(&id).await,
        },
    }
}

async fn guardian_show(json: bool) -> AppResult<()> {
    let response = client()?.get_json(GUARDIAN_PATH).await?;
    if json {
        println!("{}", pretty(&response));
    } else {
        print_lines(&guardian_lines(&response));
    }
    Ok(())
}

fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

fn print_lines(lines: &[String]) {
    for line in lines {
        println!("{line}");
    }
}

/// Flag bundle for `guardian set`, so the apply step reads as one unit.
#[derive(Default)]
struct GuardianSetArgs {
    mode: Option<String>,
    tainted_destructive: Option<String>,
    plan_mode: Option<String>,
    max_destructive_per_turn: Option<u16>,
    max_writes_per_turn: Option<u16>,
    external_send: Option<String>,
    clear: Vec<String>,
    acknowledge_impact: bool,
    dry_run: bool,
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

    /// Refuse what could never reach a save: no flags at all, or a `--clear` naming
    /// a field that does not exist.
    fn check(&self) -> AppResult<()> {
        if self.is_empty() {
            return Err(AppError::invalid_input(
                "nothing to change — pass at least one --<field> flag or --clear <field>",
            ));
        }
        for field in &self.clear {
            if !GUARDIAN_FIELDS.contains(&field.as_str()) {
                return Err(AppError::invalid_input(format!(
                    "--clear {field}: unknown field (valid: {})",
                    GUARDIAN_FIELDS.join(", ")
                )));
            }
        }
        Ok(())
    }

    /// The document these flags make of `current`. Unset flags keep their stored
    /// values, and `--clear` wins over a value given for the same field.
    fn apply_to(&self, current: &GuardianConfigDocument) -> AppResult<GuardianConfigDocument> {
        let mut document = current.clone();
        if let Some(raw) = &self.mode {
            document.mode = Some(parse_wire("mode", raw)?);
        }
        if let Some(raw) = &self.tainted_destructive {
            document.tainted_destructive = Some(parse_wire("tainted_destructive", raw)?);
        }
        if let Some(raw) = &self.plan_mode {
            document.plan_mode = Some(parse_wire("plan_mode", raw)?);
        }
        if let Some(n) = self.max_destructive_per_turn {
            document.max_destructive_per_turn = Some(n);
        }
        if let Some(n) = self.max_writes_per_turn {
            document.max_writes_per_turn = Some(n);
        }
        if let Some(raw) = &self.external_send {
            document.external_send = Some(parse_external_send(raw)?);
        }
        for field in &self.clear {
            clear_field(&mut document, field);
        }
        Ok(document)
    }
}

async fn guardian_set(args: GuardianSetArgs) -> AppResult<()> {
    args.check()?;

    let client = client()?;

    // Read-modify-write: start from the persisted document so unset flags
    // keep their stored values.
    let current = client.get_json(GUARDIAN_PATH).await?;
    let stored: GuardianConfigDocument = serde_json::from_value(
        current.get("config").cloned().unwrap_or(Value::Null),
    )
    .map_err(|e| {
        AppError::invalid_input(format!(
            "server returned an unparseable guardian config document: {e}"
        ))
    })?;
    let document = args.apply_to(&stored)?;

    if args.dry_run {
        print_lines(&dry_run_lines(&stored, &document, &current)?);
        return check_save_allowed(&document, &current);
    }

    let body = guardian_put_body(
        &document,
        current.get("updated_at").cloned().unwrap_or(Value::Null),
        args.acknowledge_impact,
    )?;
    let reply = client.put_raw(GUARDIAN_PATH, &body).await?;
    if reply.status().as_u16() == 409 {
        let refusal = reply.json::<Value>().await.unwrap_or(Value::Null);
        return Err(AppError::invalid_input(conflict_message(&refusal)));
    }
    let response = RemoteClient::json_or_error(GUARDIAN_PATH, reply).await?;
    println!("  Guardian config updated.");
    print_lines(&guardian_lines(&response));
    Ok(())
}

/// What the save's 409 tells the super admin to do. An unknown code still prints the
/// server's own sentence, because the save did not land either way.
pub(super) fn conflict_message(refusal: &Value) -> String {
    let message = refusal
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("The server refused the save, and nothing was saved.");
    match refusal.get("code").and_then(Value::as_str) {
        Some(STALE_CODE) => {
            format!("{message} Re-running this command reads the current policy first.")
        }
        Some(HARNESS_STALE_CODE) => format!(
            "{message} Read it with `settings harness show`: repeating this command writes \
             your document over what changed."
        ),
        Some(IMPACT_CODE) => {
            let mut lines = vec![
                "This save changes what athletes can do on every turn, and nothing was saved."
                    .to_owned(),
            ];
            lines.extend(impact_lines(
                refusal
                    .get("impacted_fields")
                    .and_then(Value::as_array)
                    .map_or(&[], Vec::as_slice),
            ));
            lines.push("Read each one, then repeat with --acknowledge-impact.".to_owned());
            lines.join("\n")
        }
        _ => message.to_owned(),
    }
}

/// One super-admin-facing line per impacted field, from the 409 or a dry run.
fn impact_lines(impacts: &[Value]) -> Vec<String> {
    impacts
        .iter()
        .map(|impact| {
            let text = |key: &str| impact.get(key).and_then(Value::as_str).unwrap_or("?");
            format!(
                "  {}: {} -> {}. {}",
                text("field"),
                text("from"),
                text("to"),
                text("effect")
            )
        })
        .collect()
}

/// What `set --dry-run` prints: the document change, and what the server would ask
/// about it. Nothing here writes.
fn dry_run_lines(
    stored: &GuardianConfigDocument,
    proposed: &GuardianConfigDocument,
    current: &Value,
) -> AppResult<Vec<String>> {
    let serialize = |document: &GuardianConfigDocument| {
        serde_json::to_value(document).map_err(|e| {
            AppError::invalid_input(format!("guardian config document did not serialize: {e}"))
        })
    };
    let changes = document_changes(&serialize(stored)?, &serialize(proposed)?);

    let mut lines = vec!["  Dry run: nothing was written.".to_owned()];
    if changes.is_empty() {
        lines.push("  No field changes.".to_owned());
    }
    let pinned = string_list(current.get("env_pinned"));
    for change in &changes {
        let shadowed = if pinned.contains(&change.field) {
            " (shadowed by an env pin until it is lifted)"
        } else {
            ""
        };
        lines.push(format!(
            "  {}: {} -> {}{shadowed}",
            change.field, change.from, change.to
        ));
    }

    let impacts = serde_json::to_value(high_impact_changes(stored, proposed))
        .map_err(|e| AppError::invalid_input(format!("impact list did not serialize: {e}")))?;
    let impacts = impacts.as_array().map_or(&[][..], Vec::as_slice);
    if !impacts.is_empty() {
        lines.push("  This save would need --acknowledge-impact:".to_owned());
        lines.extend(impact_lines(impacts));
    }
    Ok(lines)
}

/// The refusals a dry run can predict, so one that would fail exits non-zero. The
/// server still judges the real save; a stale base is only known then.
fn check_save_allowed(document: &GuardianConfigDocument, current: &Value) -> AppResult<()> {
    validate_document(document)?;
    let armable = current
        .get("plan_mode_armable")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    validate_plan_mode_arming(document, armable)
}

/// One field a change moves.
struct FieldChange {
    field: &'static str,
    from: String,
    to: String,
}

/// The fields that differ between two serialized documents. A non-object side, such
/// as a history row with no prior document, counts every field as unset.
fn document_changes(from: &Value, to: &Value) -> Vec<FieldChange> {
    GUARDIAN_FIELDS
        .into_iter()
        .filter_map(|field| {
            let (old, new) = (from.get(field), to.get(field));
            (stored_cell(old) != stored_cell(new)).then(|| FieldChange {
                field,
                from: stored_cell(old),
                to: stored_cell(new),
            })
        })
        .collect()
}

/// A document field as a cell: `null` and an absent key both mean it is unset.
fn stored_cell(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => UNSET.to_owned(),
        some => display_value(some),
    }
}

fn string_list(value: Option<&Value>) -> Vec<&str> {
    value
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default()
}

/// The PUT body: the document's fields beside the two save assertions.
/// `expected_updated_at` is echoed as the GET returned it, a string or `null`.
fn guardian_put_body(
    document: &GuardianConfigDocument,
    expected_updated_at: Value,
    acknowledge_impact: bool,
) -> AppResult<Value> {
    let mut body = serde_json::to_value(document).map_err(|e| {
        AppError::invalid_input(format!("guardian config document did not serialize: {e}"))
    })?;
    if let Some(fields) = body.as_object_mut() {
        fields.insert("expected_updated_at".to_owned(), expected_updated_at);
        fields.insert(
            "acknowledge_impact".to_owned(),
            Value::Bool(acknowledge_impact),
        );
    }
    Ok(body)
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

/// The `show` view: each field stored, in force and why, then which instance
/// answered and whether it has caught up.
fn guardian_lines(response: &Value) -> Vec<String> {
    let text = |key: &str, otherwise: &str| {
        response
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or(otherwise)
            .to_owned()
    };
    let flag = |key: &str| response.get(key).and_then(Value::as_bool);

    let mut lines = vec![
        format!(
            "  Guardian policy (document: {}, updated: {})",
            text("source", "?"),
            text("updated_at", "never")
        ),
        format!(
            "  {:<26}  {:<24}  {:<24}  SOURCE",
            "FIELD", "PERSISTED", "EFFECTIVE"
        ),
    ];
    for field in GUARDIAN_FIELDS {
        lines.push(format!(
            "  {field:<26}  {:<24}  {:<24}  {}",
            stored_cell(response.pointer(&format!("/config/{field}"))),
            display_value(response.pointer(&format!("/effective/{field}"))),
            response
                .pointer(&format!("/sources/{field}"))
                .and_then(Value::as_str)
                .unwrap_or("?"),
        ));
    }
    let pinned = string_list(response.get("env_pinned"));
    if !pinned.is_empty() {
        lines.push(format!(
            "  NOTE: env-pinned fields shadow admin edits until the GUARDIAN_* var is unset: {}",
            pinned.join(", ")
        ));
    }

    lines.push(format!(
        "  Answered by instance {} (loaded {})",
        text("instance_id", "?"),
        text("loaded_at", "?")
    ));
    lines.push(match flag("in_sync") {
        Some(true) => "  In sync: yes".to_owned(),
        Some(false) => {
            "  In sync: NO, this instance does not yet enforce the stored row".to_owned()
        }
        None => "  In sync: ?".to_owned(),
    });
    lines.push(match (flag("degraded"), response.get("degraded_reason")) {
        (Some(false), _) => "  Degraded: no".to_owned(),
        (Some(true), Some(Value::String(reason))) => format!("  Degraded: YES, {reason}"),
        (Some(true), _) => "  Degraded: YES".to_owned(),
        (None, _) => "  Degraded: ?".to_owned(),
    });
    lines.push(match flag("plan_mode_armable") {
        Some(true) => "  Plan mode armable: yes".to_owned(),
        Some(false) => {
            "  Plan mode armable: no (GUARDIAN_PLAN_MODE_ARMABLE is not set here)".to_owned()
        }
        None => "  Plan mode armable: ?".to_owned(),
    });
    if let Some(version) = response
        .get("stored_schema_version")
        .and_then(Value::as_u64)
    {
        lines.push(format!("  Stored schema version: {version}"));
    }
    let unknown = string_list(response.get("stored_unknown_fields"));
    if !unknown.is_empty() {
        lines.push(format!(
            "  Stored fields this binary does not read, which a save drops: {}",
            unknown.join(", ")
        ));
    }
    lines
}

async fn guardian_history(limit: u32) -> AppResult<()> {
    let response = client()?
        .get_json(&format!(
            "/admin/audit-events?action={HISTORY_ACTION_PREFIX}&limit={limit}"
        ))
        .await?;
    print_lines(&history_lines(&response));
    Ok(())
}

/// One block per recorded action: when, who, what, and the before and after it carries.
fn history_lines(response: &Value) -> Vec<String> {
    let events = response
        .pointer("/data/events")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    if events.is_empty() {
        return vec!["  No recorded Guardian actions.".to_owned()];
    }
    let mut lines = Vec::new();
    for event in events {
        lines.extend(history_event_lines(event));
    }
    lines
}

fn history_event_lines(event: &Value) -> Vec<String> {
    let text = |key: &str| event.get(key).and_then(Value::as_str).unwrap_or("?");
    let actor = event
        .get("actor_email")
        .and_then(Value::as_str)
        .unwrap_or_else(|| text("source"));
    let mut lines = vec![format!(
        "  {}  {}  {}  {}",
        text("occurred_at"),
        actor,
        text("action"),
        text("outcome")
    )];
    let metadata = event.get("metadata").unwrap_or(&Value::Null);
    let Some(after) = metadata.get("after") else {
        lines.push(format!("    {}", text("summary")));
        return lines;
    };
    let before = metadata.get("before").unwrap_or(&Value::Null);
    if let Some(raw) = before.as_str() {
        lines.push(format!("    previous row was not a document: {raw}"));
    }
    let changes = document_changes(before, after);
    if changes.is_empty() {
        lines.push("    no field changed".to_owned());
    }
    for change in changes {
        lines.push(format!(
            "    {}: {} -> {}",
            change.field, change.from, change.to
        ));
    }
    lines
}

async fn guardian_pending_list(tenant: Option<&str>, status: Option<&str>) -> AppResult<()> {
    let path = pending_list_path(tenant, status)?;
    let response = client()?.get_json(&path).await?;
    print_lines(&pending_lines(&response));
    Ok(())
}

/// The list route's path. The tenant must be a UUID and clap has limited the status
/// to its four words, so nothing here needs escaping.
fn pending_list_path(tenant: Option<&str>, status: Option<&str>) -> AppResult<String> {
    let mut query = Vec::new();
    if let Some(raw) = tenant {
        let tenant = Uuid::parse_str(raw.trim())
            .map_err(|e| AppError::invalid_input(format!("--tenant {raw}: {e}")))?;
        query.push(format!("tenant={tenant}"));
    }
    if let Some(status) = status {
        query.push(format!("status={status}"));
    }
    Ok(if query.is_empty() {
        GUARDIAN_PENDING_PATH.to_owned()
    } else {
        format!("{GUARDIAN_PENDING_PATH}?{}", query.join("&"))
    })
}

/// The deny route's path. An id is a uuid-simple token, so anything else is refused
/// here rather than interpolated into the path.
fn pending_deny_path(id: &str) -> AppResult<String> {
    let id = id.trim();
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err(AppError::invalid_input(format!(
            "'{id}' is not a parked call's id"
        )));
    }
    Ok(format!("{GUARDIAN_PENDING_PATH}/{id}/deny"))
}

fn pending_lines(response: &Value) -> Vec<String> {
    let entries = response
        .get("entries")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    let status = response
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("?");
    if entries.is_empty() {
        return vec![format!("  No parked calls with status {status}.")];
    }
    let mut lines = vec![format!(
        "  {:<32}  {:<28}  {:<36}  {:<36}  {:<9}  {:<25}  REASON",
        "ID", "TOOL", "TENANT", "USER", "STATUS", "EXPIRES"
    )];
    lines.extend(entries.iter().map(pending_line));
    lines
}

fn pending_line(entry: &Value) -> String {
    let text = |key: &str| entry.get(key).and_then(Value::as_str).unwrap_or("?");
    format!(
        "  {:<32}  {:<28}  {:<36}  {:<36}  {:<9}  {:<25}  {}",
        text("id"),
        text("tool_name"),
        text("tenant_id"),
        text("user_id"),
        text("status"),
        text("expires_at"),
        text("deny_reason"),
    )
}

async fn guardian_pending_deny(id: &str) -> AppResult<()> {
    let path = pending_deny_path(id)?;
    let response = client()?.post_json(&path, &serde_json::json!({})).await?;
    println!("  Parked call denied. It will not run.");
    if let Some(entry) = response.get("entry") {
        println!("{}", pending_line(entry));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pierre_tool_runtime::guardian::{GuardianMode, PlanMode, TaintedDestructive};
    use serde_json::json;

    const TENANT_A: &str = "11111111-1111-4111-8111-111111111111";
    const TENANT_B: &str = "22222222-2222-4222-8222-222222222222";

    fn sample_response() -> Value {
        json!({
            "config": {"schema_version": 1, "mode": "observe", "external_send": [TENANT_A, TENANT_B]},
            "effective": {
                "mode": "enforce",
                "max_destructive_per_turn": 1,
                "max_writes_per_turn": 50,
                "external_send": [TENANT_A, TENANT_B],
                "tainted_destructive": "log",
                "plan_mode": "off"
            },
            "sources": {
                "mode": "env",
                "max_destructive_per_turn": "default",
                "max_writes_per_turn": "default",
                "external_send": "database",
                "tainted_destructive": "default",
                "plan_mode": "default"
            },
            "env_pinned": ["mode"],
            "source": "persisted",
            "updated_at": "2026-11-04T12:00:00.123456+00:00",
            "loaded_at": "2026-11-04T12:00:30+00:00",
            "instance_id": "rev-7:abc",
            "in_sync": false,
            "degraded": true,
            "degraded_reason": "plan_mode enforce is not armed",
            "plan_mode_armable": false,
            "stored_schema_version": 0,
            "stored_unknown_fields": ["future_field"]
        })
    }

    fn line_of<'a>(lines: &'a [String], start: &str) -> &'a str {
        lines
            .iter()
            .find(|line| line.trim_start().starts_with(start))
            .unwrap_or_else(|| panic!("no line starts with {start}: {lines:#?}"))
    }

    #[test]
    fn external_send_parses_its_keywords_and_a_tenant_list_and_refuses_the_rest() {
        assert_eq!(
            parse_external_send(" NONE ").unwrap(),
            ExternalSendAllowlist::None
        );
        assert_eq!(
            parse_external_send("All").unwrap(),
            ExternalSendAllowlist::All
        );
        let ExternalSendAllowlist::Only(tenants) =
            parse_external_send(&format!("{TENANT_A}, {TENANT_B}")).unwrap()
        else {
            panic!("a list of tenants");
        };
        assert_eq!(tenants.len(), 2);

        let refused = parse_external_send("nobody").unwrap_err().to_string();
        assert!(refused.contains("--external-send nobody"), "{refused}");
        assert!(refused.contains("comma-separated"), "{refused}");
        assert!(parse_external_send(&format!("{TENANT_A},nope")).is_err());
    }

    #[test]
    fn clearing_a_field_unsets_that_field_alone() {
        for field in GUARDIAN_FIELDS {
            let mut document = GuardianConfigDocument {
                mode: Some(GuardianMode::Observe),
                max_destructive_per_turn: Some(2),
                max_writes_per_turn: Some(3),
                external_send: Some(ExternalSendAllowlist::All),
                tainted_destructive: Some(TaintedDestructive::Deny),
                plan_mode: Some(PlanMode::Off),
                ..GuardianConfigDocument::default()
            };
            clear_field(&mut document, field);
            let serialized = serde_json::to_value(&document).unwrap();
            for other in GUARDIAN_FIELDS {
                assert_eq!(
                    stored_cell(serialized.get(other)) == UNSET,
                    other == field,
                    "clearing {field} left {other} as {:?}",
                    serialized.get(other)
                );
            }
        }
    }

    #[test]
    fn set_keeps_what_its_flags_leave_alone_and_clear_wins() {
        let stored = GuardianConfigDocument {
            mode: Some(GuardianMode::Observe),
            max_writes_per_turn: Some(9),
            ..GuardianConfigDocument::default()
        };
        let edited = GuardianSetArgs {
            tainted_destructive: Some("CONFIRM".to_owned()),
            max_destructive_per_turn: Some(0),
            max_writes_per_turn: Some(4),
            clear: vec!["max_writes_per_turn".to_owned()],
            ..GuardianSetArgs::default()
        }
        .apply_to(&stored)
        .unwrap();

        assert_eq!(edited.mode, Some(GuardianMode::Observe), "left alone");
        assert_eq!(
            edited.tainted_destructive,
            Some(TaintedDestructive::Confirm)
        );
        assert_eq!(edited.max_destructive_per_turn, Some(0));
        assert_eq!(edited.max_writes_per_turn, None, "--clear wins");
        assert_eq!(
            stored.max_writes_per_turn,
            Some(9),
            "the input is untouched"
        );

        let typo = GuardianSetArgs {
            mode: Some("enfroce".to_owned()),
            ..GuardianSetArgs::default()
        };
        let refused = typo.apply_to(&stored).unwrap_err().to_string();
        assert!(refused.contains("--mode enfroce"), "{refused}");
    }

    #[test]
    fn set_refuses_no_flags_and_an_unknown_clear_before_any_network() {
        assert!(GuardianSetArgs::default().check().is_err());
        let refused = GuardianSetArgs {
            clear: vec!["modee".to_owned()],
            ..GuardianSetArgs::default()
        }
        .check()
        .unwrap_err()
        .to_string();
        assert!(refused.contains("--clear modee"), "{refused}");
        assert!(refused.contains("max_writes_per_turn"), "{refused}");
    }

    #[test]
    fn show_prints_the_persisted_value_a_pin_shadows_and_every_tenant() {
        let lines = guardian_lines(&sample_response());

        let mode = line_of(&lines, "mode ");
        assert!(mode.contains("observe"), "the stored value: {mode}");
        assert!(mode.contains("enforce"), "the value in force: {mode}");
        assert!(mode.ends_with("env"), "{mode}");

        let writes = line_of(&lines, "max_writes_per_turn");
        assert!(writes.contains(UNSET), "{writes}");
        assert!(writes.contains("50"), "{writes}");

        let external = line_of(&lines, "external_send");
        assert!(external.contains(TENANT_A), "{external}");
        assert!(external.contains(TENANT_B), "{external}");

        assert!(line_of(&lines, "NOTE").contains("mode"));
        let instance = line_of(&lines, "Answered by");
        assert!(instance.contains("rev-7:abc"), "{instance}");
        assert!(instance.contains("2026-11-04T12:00:30"), "{instance}");
        assert!(line_of(&lines, "In sync").contains("NO"));
        assert!(line_of(&lines, "Degraded").contains("plan_mode enforce is not armed"));
        assert!(line_of(&lines, "Plan mode armable").contains("no"));
        assert!(line_of(&lines, "Stored schema version").contains('0'));
        assert!(line_of(&lines, "Stored fields").contains("future_field"));
    }

    #[test]
    fn show_says_when_all_is_well_and_flags_nothing_absent() {
        let mut response = sample_response();
        response["in_sync"] = json!(true);
        response["degraded"] = json!(false);
        response["degraded_reason"] = Value::Null;
        response["plan_mode_armable"] = json!(true);
        response["env_pinned"] = json!([]);
        let fields = response.as_object_mut().unwrap();
        fields.remove("stored_schema_version");
        fields.remove("stored_unknown_fields");

        let lines = guardian_lines(&response);
        assert_eq!(line_of(&lines, "In sync").trim(), "In sync: yes");
        assert_eq!(line_of(&lines, "Degraded").trim(), "Degraded: no");
        assert_eq!(
            line_of(&lines, "Plan mode armable").trim(),
            "Plan mode armable: yes"
        );
        for absent in ["NOTE", "Stored schema", "Stored fields"] {
            assert!(
                !lines
                    .iter()
                    .any(|line| line.trim_start().starts_with(absent)),
                "{absent}: {lines:#?}"
            );
        }
    }

    #[test]
    fn a_stale_save_tells_the_super_admin_nothing_landed_and_what_to_do() {
        let message = conflict_message(&json!({
            "code": STALE_CODE,
            "message": "The Guardian policy changed since you read it, and nothing was saved."
        }));
        assert!(message.contains("nothing was saved"), "{message}");
        assert!(message.contains("Re-running this command"), "{message}");
    }

    #[test]
    fn an_impact_refusal_lists_each_field_and_how_to_proceed() {
        let message = conflict_message(&json!({
            "code": IMPACT_CODE,
            "message": "ignored in favour of the list",
            "impacted_fields": [
                {"field": "mode", "from": "enforce", "to": "observe", "effect": "Stops blocking."},
                {"field": "external_send", "from": "none", "to": "all", "effect": "Reaches out."}
            ]
        }));
        assert!(
            message.contains("mode: enforce -> observe. Stops blocking."),
            "{message}"
        );
        assert!(
            message.contains("external_send: none -> all. Reaches out."),
            "{message}"
        );
        assert!(message.contains("--acknowledge-impact"), "{message}");
        assert!(message.contains("nothing was saved"), "{message}");
    }

    #[test]
    fn an_unknown_refusal_still_prints_what_the_server_said() {
        assert_eq!(
            conflict_message(&json!({"code": "something_new", "message": "Not today."})),
            "Not today."
        );
        assert!(conflict_message(&Value::Null).contains("nothing was saved"));
    }

    #[test]
    fn a_dry_run_prints_the_change_its_impact_and_a_shadowing_pin() {
        let stored = GuardianConfigDocument::default();
        let proposed = GuardianConfigDocument {
            mode: Some(GuardianMode::Observe),
            max_writes_per_turn: Some(9),
            ..GuardianConfigDocument::default()
        };
        let lines = dry_run_lines(&stored, &proposed, &sample_response()).unwrap();

        assert!(line_of(&lines, "Dry run").contains("nothing was written"));
        let mode = line_of(&lines, "mode: (unset) -> observe");
        assert!(mode.contains("shadowed by an env pin"), "{mode}");
        let writes = line_of(&lines, "max_writes_per_turn: (unset) -> 9");
        assert!(!writes.contains("shadowed"), "{writes}");
        line_of(&lines, "This save would need --acknowledge-impact");
        line_of(&lines, "mode: enforce -> observe.");
    }

    #[test]
    fn a_dry_run_of_a_budget_edit_asks_for_no_acknowledgement() {
        let stored = GuardianConfigDocument::default();
        let proposed = GuardianConfigDocument {
            max_writes_per_turn: Some(9),
            ..GuardianConfigDocument::default()
        };
        let lines = dry_run_lines(&stored, &proposed, &sample_response()).unwrap();
        assert!(!lines
            .iter()
            .any(|line| line.contains("--acknowledge-impact")));

        let nothing = dry_run_lines(&stored, &stored, &sample_response()).unwrap();
        line_of(&nothing, "No field changes");
    }

    #[test]
    fn a_dry_run_exits_non_zero_on_what_the_server_would_refuse() {
        let response = sample_response();
        let armed = GuardianConfigDocument {
            plan_mode: Some(PlanMode::Enforce),
            ..GuardianConfigDocument::default()
        };
        assert!(check_save_allowed(&armed, &response).is_err());
        assert!(check_save_allowed(&armed, &json!({"plan_mode_armable": true})).is_ok());
        let zero = GuardianConfigDocument {
            max_writes_per_turn: Some(0),
            ..GuardianConfigDocument::default()
        };
        assert!(check_save_allowed(&zero, &response).is_err());
    }

    #[test]
    fn history_names_who_acted_and_what_moved() {
        let event = |at: &str, before: Value, after: Value| {
            json!({
                "occurred_at": at,
                "actor_email": "op@dravr.ai",
                "source": "console",
                "action": "guardian.config.update",
                "outcome": "success",
                "summary": "moved",
                "metadata": {"before": before, "after": after}
            })
        };
        let lines = history_lines(&json!({"success": true, "data": {"events": [
            event(
                "2026-11-04T13:00:00+00:00",
                json!({"schema_version": 1, "mode": "observe", "max_writes_per_turn": 9}),
                json!({"schema_version": 1, "mode": "enforce", "max_writes_per_turn": 9}),
            ),
            event(
                "2026-11-04T12:00:00+00:00",
                Value::Null,
                json!({"schema_version": 1, "mode": "observe"}),
            ),
            event(
                "2026-11-04T11:00:00+00:00",
                json!("{broken"),
                json!({"schema_version": 1}),
            ),
            {
                "occurred_at": "2026-11-04T10:00:00+00:00",
                "actor_email": null,
                "source": "service",
                "action": "guardian.pending.deny",
                "outcome": "success",
                "summary": "Denied the parked call of push_workout in tenant t",
                "metadata": {"target": {"pending_id": "abc"}}
            },
            {
                "occurred_at": "2026-11-04T09:00:00+00:00",
                "actor_email": "op@dravr.ai",
                "source": "cli",
                "action": "guardian.config.update",
                "outcome": "refused",
                "summary": "Refused a Guardian policy save (409)",
                "metadata": {"status": 409}
            }
        ]}}));

        let saved = line_of(&lines, "2026-11-04T13:00:00");
        assert!(saved.contains("op@dravr.ai"));
        assert!(saved.contains("guardian.config.update"));
        line_of(&lines, "mode: observe -> enforce");
        assert!(!lines
            .iter()
            .any(|line| line.contains("max_writes_per_turn")));
        line_of(&lines, "mode: (unset) -> observe");
        line_of(&lines, "previous row was not a document: {broken");
        assert!(line_of(&lines, "2026-11-04T10:00:00").contains("service"));
        line_of(&lines, "Denied the parked call of push_workout");
        assert!(line_of(&lines, "2026-11-04T09:00:00").contains("refused"));
        line_of(&lines, "Refused a Guardian policy save (409)");

        assert_eq!(
            history_lines(&json!({"data": {"events": [], "next_before": null}})),
            ["  No recorded Guardian actions."]
        );
    }

    #[test]
    fn the_pending_paths_carry_only_what_was_checked() {
        assert_eq!(
            pending_list_path(None, None).unwrap(),
            GUARDIAN_PENDING_PATH
        );
        assert_eq!(
            pending_list_path(Some(TENANT_A), Some("expired")).unwrap(),
            format!("{GUARDIAN_PENDING_PATH}?tenant={TENANT_A}&status=expired")
        );
        assert!(pending_list_path(Some("a&status=denied"), None).is_err());

        assert_eq!(
            pending_deny_path(" abc123 ").unwrap(),
            format!("{GUARDIAN_PENDING_PATH}/abc123/deny")
        );
        for bad in ["", "../../users", "a/b", "a?b"] {
            assert!(pending_deny_path(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_pending_view_lists_rows_and_says_when_there_are_none() {
        let lines = pending_lines(&json!({
            "status": "pending",
            "entries": [{
                "id": "abc123",
                "tool_name": "disconnect_provider",
                "tenant_id": TENANT_A,
                "user_id": TENANT_B,
                "status": "pending",
                "expires_at": "2026-11-04T12:05:00Z",
                "deny_reason": "tainted_sink"
            }]
        }));
        let row = line_of(&lines, "abc123");
        for cell in ["disconnect_provider", TENANT_A, TENANT_B, "tainted_sink"] {
            assert!(row.contains(cell), "{row}");
        }
        assert_eq!(
            pending_lines(&json!({"status": "denied", "entries": []})),
            ["  No parked calls with status denied."]
        );
    }

    #[test]
    fn the_guardian_put_body_carries_the_token_beside_the_document_fields() {
        let document = GuardianConfigDocument {
            max_writes_per_turn: Some(9),
            ..GuardianConfigDocument::default()
        };

        let edit = guardian_put_body(
            &document,
            Value::String("2026-11-04T12:00:00.123456+00:00".to_owned()),
            true,
        )
        .unwrap();
        assert_eq!(edit["max_writes_per_turn"], 9);
        assert_eq!(
            edit["expected_updated_at"],
            "2026-11-04T12:00:00.123456+00:00"
        );
        assert_eq!(edit["acknowledge_impact"], true);

        // No row was read: the field is present and null, which the server tells
        // apart from a client that sent no token at all.
        let create = guardian_put_body(&document, Value::Null, false).unwrap();
        assert_eq!(create.get("expected_updated_at"), Some(&Value::Null));
        assert_eq!(create["acknowledge_impact"], false);
    }
}
