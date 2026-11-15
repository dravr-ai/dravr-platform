// ABOUTME: `pierre-cli barrage adopt-cache` — keep every cached activity of a live source in the activity store
// ABOUTME: Reads the pairs with a dry run, adopts them one request each, prints a table, exits non-zero on a difference

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `pierre-cli barrage` drives `POST /admin/barrage/adopt-cache`, a
//! super-admin route (carnet#1482).
//!
//! `adopt-cache` runs once per environment after the deploy that moved every
//! provider read into the activity store. One dry run lists every
//! `(tenant, user, key)` of the cache; each live pair is then adopted in a
//! request of its own, so no request carries a whole environment. Each
//! answer says whether the store now holds as many records as the cache
//! holds rows; any difference, or a key the store does not know, makes the
//! command exit non-zero.

use std::time::Duration;

use clap::Subcommand;
use pierre_core::errors::{AppError, AppResult};
use pierre_services::cache_adoption::{AdoptionOutcome, AdoptionRow, CacheKeyKind};
use secrecy::SecretString;
use serde_json::{json, Value};

use pierre_cli::remote::RemoteClient;

use crate::commands::auth::admin_client;

const ADOPT_CACHE_PATH: &str = "/admin/barrage/adopt-cache";

/// How long one pair's adoption may take before the CLI gives up on the
/// answer: one user's whole cache under one key.
const PAIR_TIMEOUT: Duration = Duration::from_secs(300);

/// `pierre-cli barrage` — the activity store on a remote server.
#[non_exhaustive]
#[derive(Subcommand)]
pub(crate) enum BarrageCommand {
    /// Keep every cached activity of a live source in the activity store, and check the counts (super-admin)
    AdoptCache {
        /// Only this tenant's cache (its id); every tenant's when absent
        #[arg(long)]
        tenant: Option<String>,

        /// Print what would be adopted and write nothing
        #[arg(long)]
        dry_run: bool,

        /// Server base URL (defaults to the cached login)
        #[arg(long, env = "PIERRE_SERVER")]
        server: Option<String>,

        /// Super-admin token (defaults to the cached login)
        #[arg(long, env = "PIERRE_TOKEN", hide_env_values = true)]
        token: Option<SecretString>,
    },
}

/// Run one `barrage` verb.
///
/// # Errors
///
/// Returns the client's error when a call fails — the server's 403 for a
/// token that is not super-admin included — an external-service error when
/// an answer is not the route's report, and an invalid-input error naming
/// how many pairs differ when any does.
pub(crate) async fn dispatch(command: BarrageCommand) -> AppResult<()> {
    match command {
        BarrageCommand::AdoptCache {
            tenant,
            dry_run,
            server,
            token,
        } => {
            let client = admin_client(server, token)?;
            adopt_cache(&client, tenant, dry_run).await
        }
    }
}

/// Adopt the cache of `tenant`, or of every tenant, and print the table.
async fn adopt_cache(
    client: &RemoteClient,
    tenant: Option<String>,
    dry_run: bool,
) -> AppResult<()> {
    let listed = report_rows(
        &client
            .post_json_within(
                ADOPT_CACHE_PATH,
                &list_body(tenant.as_deref()),
                PAIR_TIMEOUT,
            )
            .await?,
    )?;
    let rows = if dry_run {
        listed
    } else {
        let mut adopted = Vec::with_capacity(listed.len());
        for row in listed {
            if row.kind != CacheKeyKind::Live {
                adopted.push(row);
                continue;
            }
            let answer = client
                .post_json_within(ADOPT_CACHE_PATH, &pair_body(&row), PAIR_TIMEOUT)
                .await?;
            adopted.extend(report_rows(&answer)?);
        }
        adopted
    };
    for line in table(&rows, dry_run) {
        println!("{line}");
    }
    let failures = rows.iter().filter(|row| row.outcome.is_failure()).count();
    if failures > 0 {
        return Err(AppError::invalid_input(format!(
            "{failures} (user, key) pair(s) of the cache are not held as cached: see the table"
        )));
    }
    Ok(())
}

/// The dry run that lists every pair of `tenant`, or of every tenant.
fn list_body(tenant: Option<&str>) -> Value {
    json!({ "tenant_id": tenant, "dry_run": true })
}

/// The adoption of one listed pair.
fn pair_body(row: &AdoptionRow) -> Value {
    json!({
        "tenant_id": row.tenant_id,
        "user_id": row.user_id,
        "provider": row.provider,
        "dry_run": false,
    })
}

/// The report rows of one answer of the route.
///
/// # Errors
///
/// Returns an external-service error when the answer has no `data.rows`
/// list of report rows: an answer of another shape is never read as "the
/// cache is empty".
fn report_rows(body: &Value) -> AppResult<Vec<AdoptionRow>> {
    let rows = body.pointer("/data/rows").cloned().ok_or_else(|| {
        AppError::external_service(
            "pierre-server",
            format!("unexpected {ADOPT_CACHE_PATH} response shape: no data.rows"),
        )
    })?;
    serde_json::from_value(rows).map_err(|e| {
        AppError::external_service(
            "pierre-server",
            format!("unexpected {ADOPT_CACHE_PATH} response shape: {e}"),
        )
    })
}

/// The operator's word for an outcome.
const fn outcome_label(outcome: AdoptionOutcome) -> &'static str {
    match outcome {
        AdoptionOutcome::Matched => "ok",
        AdoptionOutcome::Mismatched => "DIFFERS",
        AdoptionOutcome::Skipped => "archive, skipped",
        AdoptionOutcome::Unadoptable => "UNKNOWN KEY",
        AdoptionOutcome::WouldAdopt => "would adopt",
    }
}

/// A count the store may not have answered.
fn count(value: Option<u64>) -> String {
    value.map_or_else(|| "-".to_owned(), |n| n.to_string())
}

/// The table of `rows`: one line per pair, then the totals.
fn table(rows: &[AdoptionRow], dry_run: bool) -> Vec<String> {
    let mut lines = vec![format!(
        "  {:<36}  {:<36}  {:<22}  {:>7}  {:>7}  {:>7}  outcome",
        "tenant", "user", "key", "cached", "before", "after"
    )];
    for row in rows {
        lines.push(format!(
            "  {:<36}  {:<36}  {:<22}  {:>7}  {:>7}  {:>7}  {}",
            row.tenant_id,
            row.user_id,
            row.provider,
            row.cached,
            count(row.held_before),
            count(row.held_after),
            outcome_label(row.outcome)
        ));
    }
    let failures = rows.iter().filter(|row| row.outcome.is_failure()).count();
    lines.push(if dry_run {
        format!("  Dry run: {} pair(s), nothing written.", rows.len())
    } else {
        format!("  {} pair(s), {failures} to act on.", rows.len())
    });
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn row(provider: &str, kind: CacheKeyKind, outcome: AdoptionOutcome) -> AdoptionRow {
        AdoptionRow {
            tenant_id: "00000000-0000-0000-0000-000000000001".to_owned(),
            user_id: Uuid::nil(),
            provider: provider.to_owned(),
            kind,
            cached: 3,
            held_before: Some(0),
            held_after: Some(3),
            outcome,
        }
    }

    #[test]
    fn the_listing_is_a_dry_run_and_each_pair_is_adopted_by_name() {
        assert_eq!(
            list_body(Some("t")),
            json!({ "tenant_id": "t", "dry_run": true }),
            "bug: the listing could write"
        );
        assert_eq!(
            list_body(None),
            json!({ "tenant_id": null, "dry_run": true }),
            "bug: no --tenant does not list every tenant"
        );
        let pair = row("strava", CacheKeyKind::Live, AdoptionOutcome::WouldAdopt);
        assert_eq!(
            pair_body(&pair),
            json!({
                "tenant_id": pair.tenant_id,
                "user_id": Uuid::nil(),
                "provider": "strava",
                "dry_run": false,
            }),
            "bug: a pair's adoption names another scope"
        );
    }

    #[test]
    fn the_table_names_a_difference_and_the_totals() {
        let rows = [
            row("strava", CacheKeyKind::Live, AdoptionOutcome::Matched),
            row("garmin", CacheKeyKind::Live, AdoptionOutcome::Mismatched),
        ];
        let lines = table(&rows, false);
        assert!(
            lines[2].contains("garmin") && lines[2].contains("DIFFERS"),
            "bug: a difference is not named: {lines:?}"
        );
        assert_eq!(
            lines.last().map(String::as_str),
            Some("  2 pair(s), 1 to act on."),
            "bug: the table's last line is not the totals"
        );
    }

    #[test]
    fn an_answer_of_another_shape_is_an_error_not_an_empty_cache() {
        assert!(
            report_rows(&json!({ "data": {} })).is_err(),
            "bug: a missing rows list reads as an empty cache"
        );
        assert!(
            report_rows(&json!({ "data": { "rows": [] } })).is_ok_and(|rows| rows.is_empty()),
            "bug: an empty rows list does not read as an empty cache"
        );
    }
}
