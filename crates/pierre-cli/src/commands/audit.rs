// ABOUTME: `pierre-cli audit list` — the admin audit trail of a remote server, newest first
// ABOUTME: What operators did through the console and pierre-cli: who, what, which target, the outcome

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The admin audit trail (carnet#1391), read over the admin API.

use clap::Subcommand;
use pierre_core::errors::{AppError, AppResult};
use secrecy::SecretString;
use serde_json::Value;

use crate::commands::auth::admin_client;

/// `pierre-cli audit` — the admin audit trail.
#[non_exhaustive]
#[derive(Subcommand)]
pub(crate) enum AuditCommand {
    /// List what operators did, newest first
    List {
        /// At most this many rows (the server caps it at 200)
        #[arg(long, default_value_t = 50)]
        limit: u32,

        /// Only actions starting with this text (`user.`, `DELETE`)
        #[arg(long)]
        action: Option<String>,

        /// Only `success`, `refused` or `failed`
        #[arg(long)]
        outcome: Option<String>,

        /// Only rows older than this RFC 3339 instant (the previous page's last time)
        #[arg(long)]
        before: Option<String>,

        /// Print the rows as JSON instead of one line each
        #[arg(long)]
        json: bool,

        /// Server base URL (defaults to the cached login)
        #[arg(long, env = "PIERRE_SERVER")]
        server: Option<String>,

        /// Admin token (defaults to the cached login)
        #[arg(long, env = "PIERRE_TOKEN", hide_env_values = true)]
        token: Option<SecretString>,
    },
}

/// The query string a list asks the server for.
fn list_query(
    limit: u32,
    action: Option<&str>,
    outcome: Option<&str>,
    before: Option<&str>,
) -> String {
    let filters = [("action", action), ("outcome", outcome), ("before", before)];
    let mut parts = vec![format!("limit={limit}")];
    parts.extend(filters.into_iter().filter_map(|(key, value)| {
        value.map(|value| format!("{key}={}", urlencoding::encode(value)))
    }));
    parts.join("&")
}

/// One row as a line: time, outcome, who, what.
fn event_line(event: &Value) -> String {
    let text = |key: &str| event.get(key).and_then(Value::as_str).unwrap_or("-");
    let actor = event
        .get("actor_email")
        .and_then(Value::as_str)
        .unwrap_or_else(|| text("source"));
    format!(
        "{}  {:<8} {:<26} {}",
        text("occurred_at"),
        text("outcome"),
        actor,
        text("summary"),
    )
}

/// Run one `audit` verb.
///
/// # Errors
///
/// Returns the client's error when the call fails, and an external-service
/// error for an answer with no `events` list.
pub(crate) async fn dispatch(command: AuditCommand) -> AppResult<()> {
    match command {
        AuditCommand::List {
            limit,
            action,
            outcome,
            before,
            json,
            server,
            token,
        } => {
            let client = admin_client(server, token)?;
            let query = list_query(
                limit,
                action.as_deref(),
                outcome.as_deref(),
                before.as_deref(),
            );
            let body = client
                .get_json(&format!("/admin/audit-events?{query}"))
                .await?;
            let data = body.get("data").unwrap_or(&Value::Null);
            let events = data
                .get("events")
                .and_then(Value::as_array)
                .ok_or_else(|| {
                    AppError::external_service("pierre-server", "audit answer has no events list")
                })?;
            if json {
                let text = serde_json::to_string_pretty(events).map_err(|e| {
                    AppError::internal(format!("Failed to serialize the audit events: {e}"))
                })?;
                println!("{text}");
                return Ok(());
            }
            if events.is_empty() {
                println!("No audit events.");
            }
            for event in events {
                println!("{}", event_line(event));
            }
            if let Some(next) = data.get("next_before").and_then(Value::as_str) {
                println!("More: --before {next}");
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_query_names_only_the_filters_given() {
        assert_eq!(
            list_query(10, None, None, None),
            "limit=10",
            "bug: no filter given, none sent"
        );
        assert_eq!(
            list_query(5, Some("user."), Some("refused"), None),
            "limit=5&action=user.&outcome=refused",
            "bug: each given filter is sent once"
        );
        assert_eq!(
            list_query(1, None, None, Some("2026-11-04T20:31:45+00:00")),
            "limit=1&before=2026-11-04T20%3A31%3A45%2B00%3A00",
            "bug: a cursor's `+` must be percent-encoded or the server reads a space"
        );
    }

    #[test]
    fn a_deleted_actor_reads_as_the_source() {
        let line = event_line(&json!({
            "occurred_at": "2026-11-04T20:31:45Z",
            "outcome": "success",
            "source": "console",
            "summary": "Removed a pre-approval",
        }));
        assert!(
            line.contains("console"),
            "bug: a row with no actor email names its source: {line}"
        );
        assert!(
            line.contains("Removed a pre-approval"),
            "bug: the line carries the summary: {line}"
        );
    }
}
