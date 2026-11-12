// ABOUTME: `pierre-cli settings guardian` — show, set and history over /admin/settings/guardian
// ABOUTME: A save is read-modify-write with the token it read; a refused one lists why and exits non-zero

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Remote management of the Guardian security policy.
//!
//! The `set` verb is read-modify-write: it GETs the persisted document, applies
//! only the flags given, and PUTs the result with the `updated_at` it read, so the
//! server refuses the save if the row changed in between.
//!
//! A deployment runs several instances, and `show` reports the one that answered:
//! `effective`, `loaded_at` and `in_sync` can differ between them for up to a poll
//! interval after a save. The persisted column is the row, the same for all.

use clap::Subcommand;
use pierre_cli::remote::RemoteClient;
use pierre_core::errors::{AppError, AppResult};
use std::collections::BTreeSet;

use pierre_tool_runtime::guardian::{validate_document, GuardianConfigDocument};
use serde_json::Value;

use super::settings::{client, display_value};

const GUARDIAN_PATH: &str = "/admin/settings/guardian";

/// The guardian document fields, in display order. `--clear` values are
/// validated against this list so a typo errors instead of silently no-oping.
const GUARDIAN_FIELDS: [&str; 2] = ["max_destructive_per_turn", "max_writes_per_turn"];

/// The audit-trail actions `settings guardian history` lists.
const HISTORY_ACTION_PREFIX: &str = "guardian.";
/// The 409 code of a save whose base is stale.
const STALE_CODE: &str = "guardian_config_stale";
/// The 409 code of a harness save whose base is stale, which shares this answer.
const HARNESS_STALE_CODE: &str = "harness_config_stale";

/// What a field with no stored value shows.
const UNSET: &str = "(unset)";

/// Verbs for `settings guardian`.
#[non_exhaustive]
#[derive(Subcommand)]
pub(crate) enum GuardianAction {
    /// Show each field's persisted value beside what the instance that answered
    /// enforces, plus that instance's id, sync state and degradation
    Show {
        /// Print the server's raw JSON response
        #[arg(long)]
        json: bool,
    },
    /// Update fields; unset flags leave their fields unchanged
    Set {
        /// Max `IRREVERSIBLE` tool executions per turn
        #[arg(long)]
        max_destructive_per_turn: Option<u16>,
        /// Max `WRITES_DATA` tool executions per turn (>= 1)
        #[arg(long)]
        max_writes_per_turn: Option<u16>,
        /// Reset a field to follow the compiled-in default (repeatable)
        #[arg(long, value_name = "FIELD")]
        clear: Vec<String>,
        /// Print the resolved change, and write nothing
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
            max_destructive_per_turn,
            max_writes_per_turn,
            clear,
            dry_run,
        } => {
            guardian_set(GuardianSetArgs {
                max_destructive_per_turn,
                max_writes_per_turn,
                clear,
                dry_run,
            })
            .await
        }
        GuardianAction::History { limit } => guardian_history(limit).await,
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
    max_destructive_per_turn: Option<u16>,
    max_writes_per_turn: Option<u16>,
    clear: Vec<String>,
    dry_run: bool,
}

impl GuardianSetArgs {
    fn is_empty(&self) -> bool {
        self.max_destructive_per_turn.is_none()
            && self.max_writes_per_turn.is_none()
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
    fn apply_to(&self, current: &GuardianConfigDocument) -> GuardianConfigDocument {
        let mut document = current.clone();
        if let Some(n) = self.max_destructive_per_turn {
            document.max_destructive_per_turn = Some(n);
        }
        if let Some(n) = self.max_writes_per_turn {
            document.max_writes_per_turn = Some(n);
        }
        for field in &self.clear {
            clear_field(&mut document, field);
        }
        document
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
    let document = args.apply_to(&stored);

    if args.dry_run {
        print_lines(&dry_run_lines(&stored, &document)?);
        return validate_document(&document);
    }

    let body = guardian_put_body(
        &document,
        current.get("updated_at").cloned().unwrap_or(Value::Null),
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
        _ => message.to_owned(),
    }
}

/// What `set --dry-run` prints: the document change. Nothing here writes.
fn dry_run_lines(
    stored: &GuardianConfigDocument,
    proposed: &GuardianConfigDocument,
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
    for change in &changes {
        lines.push(format!(
            "  {}: {} -> {}",
            change.field, change.from, change.to
        ));
    }
    Ok(lines)
}

/// One field a change moves.
struct FieldChange {
    field: String,
    from: String,
    to: String,
}

/// The fields that differ between two serialized documents, every key either side
/// holds included, so a history row written before a field was retired still says
/// what it moved. A non-object side, such as a history row with no prior document,
/// counts every field as unset.
fn document_changes(from: &Value, to: &Value) -> Vec<FieldChange> {
    let keys: BTreeSet<&str> = [from, to]
        .into_iter()
        .filter_map(Value::as_object)
        .flat_map(|fields| fields.keys().map(String::as_str))
        .collect();
    keys.into_iter()
        .filter_map(|field| {
            let (old, new) = (from.get(field), to.get(field));
            (stored_cell(old) != stored_cell(new)).then(|| FieldChange {
                field: field.to_owned(),
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

/// The PUT body: the document's fields beside the base the save edits.
/// `expected_updated_at` is echoed as the GET returned it, a string or `null`.
fn guardian_put_body(
    document: &GuardianConfigDocument,
    expected_updated_at: Value,
) -> AppResult<Value> {
    let mut body = serde_json::to_value(document).map_err(|e| {
        AppError::invalid_input(format!("guardian config document did not serialize: {e}"))
    })?;
    if let Some(fields) = body.as_object_mut() {
        fields.insert("expected_updated_at".to_owned(), expected_updated_at);
    }
    Ok(body)
}

fn clear_field(document: &mut GuardianConfigDocument, field: &str) {
    match field {
        "max_destructive_per_turn" => document.max_destructive_per_turn = None,
        "max_writes_per_turn" => document.max_writes_per_turn = None,
        // Unreachable: names are validated against GUARDIAN_FIELDS above.
        _ => {}
    }
}

/// The `show` view: each field stored and in force, then which instance
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
            "  Guardian policy (updated: {})",
            text("updated_at", "never")
        ),
        format!("  {:<26}  {:<24}  EFFECTIVE", "FIELD", "PERSISTED"),
    ];
    for field in GUARDIAN_FIELDS {
        lines.push(format!(
            "  {field:<26}  {:<24}  {}",
            stored_cell(response.pointer(&format!("/config/{field}"))),
            display_value(response.pointer(&format!("/effective/{field}"))),
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_response() -> Value {
        json!({
            "config": {"max_destructive_per_turn": 1},
            "effective": {
                "max_destructive_per_turn": 1,
                "max_writes_per_turn": 50
            },
            "updated_at": "2026-11-04T12:00:00.123456+00:00",
            "loaded_at": "2026-11-04T12:00:30+00:00",
            "instance_id": "rev-7:abc",
            "in_sync": false,
            "degraded": true,
            "degraded_reason": "max_writes_per_turn=0 is refused",
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
    fn clearing_a_field_unsets_that_field_alone() {
        for field in GUARDIAN_FIELDS {
            let mut document = GuardianConfigDocument {
                max_destructive_per_turn: Some(2),
                max_writes_per_turn: Some(3),
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
            max_destructive_per_turn: Some(2),
            max_writes_per_turn: Some(9),
        };
        let edited = GuardianSetArgs {
            max_writes_per_turn: Some(4),
            clear: vec!["max_writes_per_turn".to_owned()],
            ..GuardianSetArgs::default()
        }
        .apply_to(&stored);

        assert_eq!(edited.max_destructive_per_turn, Some(2), "left alone");
        assert_eq!(edited.max_writes_per_turn, None, "--clear wins");
        assert_eq!(
            stored.max_writes_per_turn,
            Some(9),
            "the input is untouched"
        );
    }

    #[test]
    fn set_refuses_no_flags_and_an_unknown_clear_before_any_network() {
        assert!(GuardianSetArgs::default().check().is_err());
        let refused = GuardianSetArgs {
            clear: vec!["mode".to_owned()],
            ..GuardianSetArgs::default()
        }
        .check()
        .unwrap_err()
        .to_string();
        assert!(refused.contains("--clear mode"), "{refused}");
        assert!(refused.contains("max_writes_per_turn"), "{refused}");
    }

    #[test]
    fn show_prints_the_persisted_value_beside_the_one_in_force() {
        let lines = guardian_lines(&sample_response());

        let destructive = line_of(&lines, "max_destructive_per_turn");
        assert!(destructive.contains('1'), "{destructive}");

        let writes = line_of(&lines, "max_writes_per_turn");
        assert!(writes.contains(UNSET), "{writes}");
        assert!(writes.contains("50"), "{writes}");

        let instance = line_of(&lines, "Answered by");
        assert!(instance.contains("rev-7:abc"), "{instance}");
        assert!(instance.contains("2026-11-04T12:00:30"), "{instance}");
        assert!(line_of(&lines, "In sync").contains("NO"));
        assert!(line_of(&lines, "Degraded").contains("max_writes_per_turn=0 is refused"));
        assert!(line_of(&lines, "Stored fields").contains("future_field"));
    }

    #[test]
    fn show_says_when_all_is_well_and_flags_nothing_absent() {
        let mut response = sample_response();
        response["in_sync"] = json!(true);
        response["degraded"] = json!(false);
        response["degraded_reason"] = Value::Null;
        response
            .as_object_mut()
            .unwrap()
            .remove("stored_unknown_fields");

        let lines = guardian_lines(&response);
        assert_eq!(line_of(&lines, "In sync").trim(), "In sync: yes");
        assert_eq!(line_of(&lines, "Degraded").trim(), "Degraded: no");
        assert!(
            !lines
                .iter()
                .any(|line| line.trim_start().starts_with("Stored fields")),
            "{lines:#?}"
        );
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
    fn an_unknown_refusal_still_prints_what_the_server_said() {
        assert_eq!(
            conflict_message(&json!({"code": "something_new", "message": "Not today."})),
            "Not today."
        );
        assert!(conflict_message(&Value::Null).contains("nothing was saved"));
    }

    #[test]
    fn a_dry_run_prints_each_change_and_nothing_else() {
        let stored = GuardianConfigDocument::default();
        let proposed = GuardianConfigDocument {
            max_writes_per_turn: Some(9),
            ..GuardianConfigDocument::default()
        };
        let lines = dry_run_lines(&stored, &proposed).unwrap();

        assert!(line_of(&lines, "Dry run").contains("nothing was written"));
        line_of(&lines, "max_writes_per_turn: (unset) -> 9");
        assert_eq!(lines.len(), 2, "{lines:#?}");

        let nothing = dry_run_lines(&stored, &stored).unwrap();
        line_of(&nothing, "No field changes");
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
                json!({"max_destructive_per_turn": 3, "max_writes_per_turn": 9}),
                json!({"max_destructive_per_turn": 1, "max_writes_per_turn": 9}),
            ),
            event(
                "2026-11-04T12:00:00+00:00",
                Value::Null,
                json!({"max_destructive_per_turn": 2}),
            ),
            event(
                "2026-11-04T11:30:00+00:00",
                json!({"schema_version": 2, "mode": "observe"}),
                json!({"schema_version": 2, "mode": "enforce"}),
            ),
            event(
                "2026-11-04T11:00:00+00:00",
                json!("{broken"),
                json!({}),
            ),
            {
                "occurred_at": "2026-11-04T10:00:00+00:00",
                "actor_email": null,
                "source": "service",
                "action": "guardian.config.update",
                "outcome": "failure",
                "summary": "A save with no document",
                "metadata": {}
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
        line_of(&lines, "max_destructive_per_turn: 3 -> 1");
        assert!(!lines
            .iter()
            .any(|line| line.contains("max_writes_per_turn")));
        line_of(&lines, "max_destructive_per_turn: (unset) -> 2");
        // A save from before the mode was retired still says what it moved.
        line_of(&lines, "mode: observe -> enforce");
        line_of(&lines, "previous row was not a document: {broken");
        assert!(line_of(&lines, "2026-11-04T10:00:00").contains("service"));
        line_of(&lines, "A save with no document");
        assert!(line_of(&lines, "2026-11-04T09:00:00").contains("refused"));
        line_of(&lines, "Refused a Guardian policy save (409)");

        assert_eq!(
            history_lines(&json!({"data": {"events": [], "next_before": null}})),
            ["  No recorded Guardian actions."]
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
        )
        .unwrap();
        assert_eq!(edit["max_writes_per_turn"], 9);
        assert_eq!(
            edit["expected_updated_at"],
            "2026-11-04T12:00:00.123456+00:00"
        );
        assert_eq!(edit.as_object().unwrap().len(), 2, "{edit}");

        // No row was read: the field is present and null, which the server tells
        // apart from a client that sent no token at all.
        let create = guardian_put_body(&document, Value::Null).unwrap();
        assert_eq!(create.get("expected_updated_at"), Some(&Value::Null));
    }
}
