// ABOUTME: `pierre-cli provider purge` — the super-admin termination purge over /admin/providers/{provider}/data
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
use serde_json::Value;

use pierre_cli::remote::RemoteClient;

use crate::commands::auth::admin_client;

/// `pierre-cli provider` — provider-wide operations.
#[non_exhaustive]
#[derive(Subcommand)]
pub enum ProviderCommand {
    /// Delete every row a provider contributed, for every user in every tenant (super-admin)
    Purge {
        /// Provider name as the rows store it (for example `whoop`)
        provider: String,

        /// Delete for real. Without it, print what would be deleted and exit
        /// non-zero, deleting nothing.
        #[arg(long)]
        yes: bool,

        /// Server base URL (defaults to the cached login)
        #[arg(long)]
        server: Option<String>,

        /// Super-admin token (defaults to the cached login)
        #[arg(long)]
        token: Option<String>,
    },
}

/// Run one `provider` verb.
///
/// # Errors
///
/// Returns [`AppError::invalid_input`] when a purge is not confirmed with
/// `--yes`, and the client's error when the call fails — including the
/// server's 403 for a token that is not super-admin.
pub async fn dispatch(command: ProviderCommand) -> AppResult<()> {
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
    }
}

/// Purge `provider` over the admin API and print what was removed.
///
/// # Errors
///
/// Returns the client's error when the call fails.
pub async fn purge(client: &RemoteClient, provider: &str) -> AppResult<()> {
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
