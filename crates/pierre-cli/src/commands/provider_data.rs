// ABOUTME: `pierre-cli provider purge` — the termination purge; `provider purges` — the recorded deletions, for attestation
// ABOUTME: Deletes every row one provider contributed in every tenant, and refuses to run without --yes

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Provider-wide operations against a remote server.
//!
//! `purge` is what a provider's termination clause calls for (WHOOP API Terms
//! §7): every row the provider contributed, deleted for every user in every
//! tenant. An athlete's own disconnect already deletes their rows; this is the
//! platform-wide sweep. It deletes data, not connections, so a user still
//! connected keeps syncing — the answer says how many remain.

use clap::Subcommand;
use pierre_core::errors::{AppError, AppResult};
use pierre_database::repositories::ProviderDataPurgeRecord;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::Value;

use pierre_cli::remote::RemoteClient;

use crate::commands::auth::admin_client;

/// `pierre-cli provider` — provider-wide operations.
#[non_exhaustive]
#[derive(Subcommand)]
pub(crate) enum ProviderCommand {
    /// Delete every row a provider contributed, for every user in every tenant (super-admin)
    Purge {
        /// Provider name as the rows store it (for example `whoop`)
        provider: String,

        /// Delete for real. Without it, print what would be deleted and exit
        /// non-zero, deleting nothing.
        #[arg(long)]
        yes: bool,

        /// Server base URL (defaults to the cached login)
        #[arg(long, env = "PIERRE_SERVER")]
        server: Option<String>,

        /// Super-admin token (defaults to the cached login)
        #[arg(long, env = "PIERRE_TOKEN", hide_env_values = true)]
        token: Option<SecretString>,
    },
    /// List the recorded deletions of a provider's data, newest first, for an attestation (super-admin)
    Purges {
        /// Provider name as the rows store it (for example `nolio`)
        provider: String,

        /// Server base URL (defaults to the cached login)
        #[arg(long, env = "PIERRE_SERVER")]
        server: Option<String>,

        /// Super-admin token (defaults to the cached login)
        #[arg(long, env = "PIERRE_TOKEN", hide_env_values = true)]
        token: Option<SecretString>,
    },
}

/// Run one `provider` verb.
///
/// # Errors
///
/// Returns [`AppError::invalid_input`] when a purge is not confirmed with
/// `--yes`, and the client's error when the call fails — including the
/// server's 403 for a token that is not super-admin.
pub(crate) async fn dispatch(command: ProviderCommand) -> AppResult<()> {
    match command {
        ProviderCommand::Purge {
            provider,
            yes,
            server,
            token,
        } => {
            if !yes {
                println!(
                    "Would delete every row {provider} contributed — sleep, recovery, body \
                     metrics, time-series points, cached activities and their sync state — for \
                     every user in every tenant."
                );
                println!("Re-run with --yes to delete.");
                return Err(AppError::invalid_input(
                    "provider purge not confirmed: nothing was deleted (re-run with --yes)",
                ));
            }
            let client = admin_client(server, token)?;
            purge(&client, &provider).await
        }
        ProviderCommand::Purges {
            provider,
            server,
            token,
        } => {
            let client = admin_client(server, token)?;
            list_purges(&client, &provider).await
        }
    }
}

/// Print the recorded deletions of `provider`'s data, one line each: when,
/// why, how many rows, and the user and tenant (`all` for a whole-provider
/// purge).
///
/// # Errors
///
/// Returns the client's error when the call fails, and
/// [`AppError::external_service`] when the answer is not the route's
/// `data.purges` list of records.
pub(crate) async fn list_purges(client: &RemoteClient, provider: &str) -> AppResult<()> {
    let encoded = urlencoding::encode(provider);
    let body: Value = client
        .get_json(&format!("/admin/providers/{encoded}/purges"))
        .await?;
    for line in purge_lines(provider, &body)? {
        println!("{line}");
    }
    Ok(())
}

/// The attestation lines for the `/admin/providers/{provider}/purges` answer
/// `body`, newest first as the server orders them.
///
/// This is what an operator writes a provider attestation from, so an answer
/// of any other shape (a renamed envelope, a missing field, the `Null` a
/// non-JSON 200 decodes to) is an error, never "no deletion is recorded" or a
/// made-up value. Only `user_id` and `tenant_id` read `all` when absent: that
/// is how a whole-provider purge is recorded.
///
/// # Errors
///
/// Returns [`AppError::external_service`] when `body` has no `data.purges`
/// list, or a record in it does not decode.
fn purge_lines(provider: &str, body: &Value) -> AppResult<Vec<String>> {
    let shape_error = |detail: &str| {
        AppError::external_service(
            "pierre-server",
            format!("unexpected /admin/providers/{provider}/purges response shape: {detail}"),
        )
    };
    let purges = body
        .pointer("/data/purges")
        .ok_or_else(|| shape_error("no data.purges list"))?;
    let records: Vec<ProviderDataPurgeRecord> =
        Vec::deserialize(purges).map_err(|e| shape_error(&e.to_string()))?;
    if records.is_empty() {
        return Ok(vec![format!("No deletion of {provider} data is recorded.")]);
    }
    Ok(records
        .iter()
        .map(|purge| {
            format!(
                "{}  {:<20}  {:>7} rows  user {}  tenant {}",
                purge.purged_at.to_rfc3339(),
                purge.reason,
                purge.rows_removed,
                purge.user_id.as_deref().unwrap_or("all"),
                purge.tenant_id.as_deref().unwrap_or("all"),
            )
        })
        .collect())
}

/// Purge `provider` over the admin API and print what was removed.
///
/// # Errors
///
/// Returns the client's error when the call fails.
pub(crate) async fn purge(client: &RemoteClient, provider: &str) -> AppResult<()> {
    let encoded = urlencoding::encode(provider);
    let body: Value = client
        .delete_json(&format!("/admin/providers/{encoded}/data"))
        .await?;
    let message = body
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("Provider data purged");
    println!("{message}");

    let data = body.get("data").unwrap_or(&Value::Null);
    if let Some(tables) = data.get("rows_removed").and_then(Value::as_object) {
        for (table, removed) in tables {
            println!("  - {table}: {removed}");
        }
    }
    let remaining = data
        .get("connections_remaining")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    if remaining > 0 {
        println!(
            "  {remaining} user(s) are still connected to {provider} and will sync again; \
             disconnect them for the purge to hold."
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};

    use super::purge_lines;

    /// One record as the admin route serializes `ProviderDataPurgeRecord`.
    fn record(user_id: Option<&str>, tenant_id: Option<&str>) -> Value {
        json!({
            "id": "c0ffee00-0000-4000-8000-000000000001",
            "tenant_id": tenant_id,
            "user_id": user_id,
            "provider": "nolio",
            "reason": "termination",
            "rows_removed": 412,
            "purged_at": "2026-10-14T09:30:00Z",
        })
    }

    #[test]
    fn a_non_json_answer_is_an_error_not_an_empty_attestation() {
        let err = purge_lines("nolio", &Value::Null).unwrap_err();
        assert!(err.to_string().contains("no data.purges list"), "{err}");
    }

    #[test]
    fn a_renamed_envelope_is_an_error() {
        let body = json!({ "purges": [record(None, None)] });
        assert!(purge_lines("nolio", &body).is_err());
    }

    #[test]
    fn a_record_missing_its_timestamp_is_an_error() {
        let mut row = record(None, None);
        row.as_object_mut().unwrap().remove("purged_at");
        let body = json!({ "data": { "purges": [row] } });
        let err = purge_lines("nolio", &body).unwrap_err();
        assert!(err.to_string().contains("purged_at"), "{err}");
    }

    #[test]
    fn a_whole_provider_purge_reads_all_and_keeps_its_real_fields() {
        let body = json!({ "data": { "purges": [record(None, None)] } });
        let lines = purge_lines("nolio", &body).unwrap();
        assert_eq!(
            lines,
            vec![
                "2026-10-14T09:30:00+00:00  termination               412 rows  user all  tenant all"
                    .to_owned()
            ]
        );
    }

    #[test]
    fn a_per_user_purge_names_the_user_and_tenant() {
        let body = json!({ "data": { "purges": [record(Some("u-1"), Some("t-1"))] } });
        let lines = purge_lines("nolio", &body).unwrap();
        assert_eq!(lines.len(), 1);
        assert!(lines[0].ends_with("user u-1  tenant t-1"), "{}", lines[0]);
    }

    #[test]
    fn an_empty_list_is_the_recorded_no_deletion_line() {
        let body = json!({ "data": { "purges": [] } });
        assert_eq!(
            purge_lines("nolio", &body).unwrap(),
            vec!["No deletion of nolio data is recorded.".to_owned()]
        );
    }
}
