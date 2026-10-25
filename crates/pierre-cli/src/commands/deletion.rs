// ABOUTME: `pierre-cli deletion` — verify the account-deletion receipt chain end to end, and list or clear tombstones
// ABOUTME: The chain is recomputed here from the stored receipts, never taken on the server's word

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The deletion ledger, against a remote server.
//!
//! `verify-receipts` reads every deletion receipt over the admin API and
//! recomputes the chain itself (`verify_receipt_chain`): each `seq` follows
//! the one before it, each `prev_hash` is the previous `hash`, and each `hash`
//! is the one the receipt's own fields produce. It prints the head, which an
//! operator records somewhere the database cannot reach, and on a broken
//! chain names the first broken link and exits non-zero.
//!
//! `tombstones` lists the import tombstones deleted accounts left, and
//! `clear-tombstone` clears one as the operator, when the person who holds
//! the identity asks for it and cannot clear it themselves.

use clap::Subcommand;
use pierre_core::errors::{AppError, AppResult};
use pierre_database::repositories::{
    verify_receipt_chain, ChainVerification, DeletionReceipt, ImportTombstone,
};
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::Value;

use pierre_cli::remote::RemoteClient;

use crate::commands::auth::admin_client;

/// How many receipts one page asks for; the server caps it the same.
const RECEIPT_PAGE: usize = 1000;

/// `pierre-cli deletion` — the account-deletion ledger.
#[non_exhaustive]
#[derive(Subcommand)]
pub(crate) enum DeletionCommand {
    /// Recompute the whole deletion receipt chain and report its first broken link (super-admin)
    VerifyReceipts {
        /// Server base URL (defaults to the cached login)
        #[arg(long, env = "PIERRE_SERVER")]
        server: Option<String>,

        /// Super-admin token (defaults to the cached login)
        #[arg(long, env = "PIERRE_TOKEN", hide_env_values = true)]
        token: Option<SecretString>,
    },
    /// List the import tombstones deleted accounts left (super-admin)
    Tombstones {
        /// Include cleared tombstones
        #[arg(long)]
        include_cleared: bool,

        /// Server base URL (defaults to the cached login)
        #[arg(long, env = "PIERRE_SERVER")]
        server: Option<String>,

        /// Super-admin token (defaults to the cached login)
        #[arg(long, env = "PIERRE_TOKEN", hide_env_values = true)]
        token: Option<SecretString>,
    },
    /// Clear one import tombstone, so its identity is accepted again (super-admin)
    ClearTombstone {
        /// The tombstone id a refusal named
        tombstone_id: String,

        /// Clear for real. Without it, print what would change and exit
        /// non-zero, clearing nothing.
        #[arg(long)]
        yes: bool,

        /// Server base URL (defaults to the cached login)
        #[arg(long, env = "PIERRE_SERVER")]
        server: Option<String>,

        /// Super-admin token (defaults to the cached login)
        #[arg(long, env = "PIERRE_TOKEN", hide_env_values = true)]
        token: Option<SecretString>,
    },
}

/// Run one `deletion` verb.
///
/// # Errors
///
/// Returns the client's error when a call fails, an external-service error
/// for an answer of an unexpected shape, an internal error naming the first
/// broken link when the chain does not verify, and an invalid-input error
/// for a clear not confirmed with `--yes`.
pub(crate) async fn dispatch(command: DeletionCommand) -> AppResult<()> {
    match command {
        DeletionCommand::VerifyReceipts { server, token } => {
            let client = admin_client(server, token)?;
            let receipts = fetch_receipts(&client).await?;
            let verification = verify_receipt_chain(&receipts);
            for line in verification_lines(&verification) {
                println!("{line}");
            }
            verification.first_break.map_or(Ok(()), |fault| {
                Err(AppError::internal(format!(
                    "deletion receipt chain broken at seq {}: {}",
                    fault.seq, fault.reason
                )))
            })
        }
        DeletionCommand::Tombstones {
            include_cleared,
            server,
            token,
        } => {
            let client = admin_client(server, token)?;
            let body = client
                .get_json(&format!(
                    "/admin/import-tombstones?include_cleared={include_cleared}"
                ))
                .await?;
            for line in tombstone_lines(&tombstones_from(&body)?) {
                println!("{line}");
            }
            Ok(())
        }
        DeletionCommand::ClearTombstone {
            tombstone_id,
            yes,
            server,
            token,
        } => {
            if !yes {
                println!(
                    "Would clear tombstone {tombstone_id}: the provider account or email it names would be accepted again."
                );
                println!("Re-run with --yes to clear.");
                return Err(AppError::invalid_input(
                    "tombstone clear not confirmed: nothing was cleared (re-run with --yes)",
                ));
            }
            let client = admin_client(server, token)?;
            let encoded = urlencoding::encode(&tombstone_id);
            let body = client
                .post_json(
                    &format!("/admin/import-tombstones/{encoded}/clear"),
                    &serde_json::json!({}),
                )
                .await?;
            let message = body
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("Tombstone cleared");
            println!("{message}");
            Ok(())
        }
    }
}

/// Every receipt, page after page, in `seq` order.
async fn fetch_receipts(client: &RemoteClient) -> AppResult<Vec<DeletionReceipt>> {
    let mut receipts: Vec<DeletionReceipt> = Vec::new();
    loop {
        let after = receipts.last().map_or(0, |r| r.seq);
        let body = client
            .get_json(&format!(
                "/admin/deletion-receipts?after_seq={after}&limit={RECEIPT_PAGE}"
            ))
            .await?;
        let page = receipts_from(&body)?;
        let full = page.len() >= RECEIPT_PAGE;
        receipts.extend(page);
        if !full {
            return Ok(receipts);
        }
    }
}

/// The error for an admin answer that is not the shape its route sends.
fn shape_error(route: &str, detail: &str) -> AppError {
    AppError::external_service(
        "pierre-server",
        format!("unexpected {route} response shape: {detail}"),
    )
}

/// The receipts in a `/admin/deletion-receipts` answer.
///
/// An answer of any other shape is an error, never an empty, verified chain.
///
/// # Errors
///
/// Returns an external-service error when `body` has no `data.receipts` list
/// or a receipt in it does not decode.
fn receipts_from(body: &Value) -> AppResult<Vec<DeletionReceipt>> {
    let route = "/admin/deletion-receipts";
    let receipts = body
        .pointer("/data/receipts")
        .ok_or_else(|| shape_error(route, "no data.receipts list"))?;
    Vec::deserialize(receipts).map_err(|e| shape_error(route, &e.to_string()))
}

/// The tombstones in a `/admin/import-tombstones` answer.
///
/// # Errors
///
/// Returns an external-service error when `body` has no `data.tombstones`
/// list or a tombstone in it does not decode.
fn tombstones_from(body: &Value) -> AppResult<Vec<ImportTombstone>> {
    let route = "/admin/import-tombstones";
    let tombstones = body
        .pointer("/data/tombstones")
        .ok_or_else(|| shape_error(route, "no data.tombstones list"))?;
    Vec::deserialize(tombstones).map_err(|e| shape_error(route, &e.to_string()))
}

/// What `verify-receipts` prints for `verification`.
fn verification_lines(verification: &ChainVerification) -> Vec<String> {
    let mut lines = Vec::new();
    match &verification.head {
        None => lines.push("No deletion receipt is recorded.".to_owned()),
        Some((seq, hash)) => lines.push(format!("Head: seq {seq}  hash {hash}")),
    }
    match &verification.first_break {
        None => lines.push(format!(
            "Chain verified: {} receipts, every link intact.",
            verification.verified
        )),
        Some(fault) => {
            lines.push(format!(
                "Chain BROKEN: {} receipts verified before the first broken link.",
                verification.verified
            ));
            lines.push(format!(
                "First broken link: seq {} (receipt {}): {}",
                fault.seq,
                fault.id.as_deref().unwrap_or("missing"),
                fault.reason
            ));
        }
    }
    lines
}

/// One line per tombstone: id, kind, provider, when written, and whether and
/// by whom it was cleared.
fn tombstone_lines(tombstones: &[ImportTombstone]) -> Vec<String> {
    if tombstones.is_empty() {
        return vec!["No import tombstone is recorded.".to_owned()];
    }
    tombstones
        .iter()
        .map(|t| {
            let state = match (&t.cleared_at, &t.cleared_by) {
                (Some(at), by) => format!(
                    "cleared {} by {}",
                    at.to_rfc3339(),
                    by.as_deref().unwrap_or("unknown")
                ),
                (None, _) => "live".to_owned(),
            };
            format!(
                "{}  {:<16}  {:<14}  {}  {state}",
                t.id,
                t.kind.as_str(),
                t.provider.as_deref().unwrap_or("-"),
                t.created_at.to_rfc3339(),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};

    use pierre_database::repositories::{verify_receipt_chain, DeletionReceipt, GENESIS_HASH};

    use super::{receipts_from, verification_lines};

    /// Two receipts sealed the way the server seals them.
    fn sealed_pair() -> Vec<DeletionReceipt> {
        let mut prev = GENESIS_HASH.to_owned();
        (1..=2)
            .map(|seq| {
                let mut receipt = DeletionReceipt {
                    id: format!("r-{seq}"),
                    seq,
                    prev_hash: prev.clone(),
                    hash: String::new(),
                    deleted_at: "2026-10-20T12:00:00.000000Z".to_owned(),
                    subject_hash: "ab".repeat(32),
                    actor: "operator".to_owned(),
                    scan_result: "{}".to_owned(),
                    revocations: "[]".to_owned(),
                    not_deleted: "[]".to_owned(),
                    tombstone_ids: "[]".to_owned(),
                };
                receipt.hash = receipt.computed_hash().unwrap();
                prev.clone_from(&receipt.hash);
                receipt
            })
            .collect()
    }

    #[test]
    fn a_non_json_answer_is_an_error_not_an_empty_verified_chain() {
        let err = receipts_from(&Value::Null).unwrap_err();
        assert!(err.to_string().contains("no data.receipts list"), "{err}");
    }

    #[test]
    fn a_tampered_receipt_from_the_wire_is_named_as_the_first_broken_link() {
        let mut receipts = sealed_pair();
        receipts[1].not_deleted = r#"["nothing"]"#.to_owned();
        let body = json!({ "data": { "receipts": receipts } });
        let verification = verify_receipt_chain(&receipts_from(&body).unwrap());
        let lines = verification_lines(&verification);
        assert_eq!(lines.len(), 3, "{lines:?}");
        assert!(
            lines[1].starts_with("Chain BROKEN: 1 receipts"),
            "{}",
            lines[1]
        );
        assert!(
            lines[2].starts_with("First broken link: seq 2 (receipt r-2)"),
            "{}",
            lines[2]
        );
    }

    #[test]
    fn an_intact_chain_prints_its_head_and_count() {
        let receipts = sealed_pair();
        let lines = verification_lines(&verify_receipt_chain(&receipts));
        assert_eq!(
            lines,
            vec![
                format!("Head: seq 2  hash {}", receipts[1].hash),
                "Chain verified: 2 receipts, every link intact.".to_owned(),
            ]
        );
    }
}
