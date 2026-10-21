// ABOUTME: An account deletion in one transaction: every provider's data, the account, its tombstones and its receipt
// ABOUTME: The receipt chains to the one before it by SHA-256, and a pure verifier names the first broken link

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Provable account deletion.
//!
//! [`AccountDeletionRepository::delete_account`] runs everything the database
//! owes a deleted account in ONE transaction, after the caller has withdrawn
//! the grants upstream (network calls never run inside it):
//!
//! 1. it reads, before anything is deleted, the account's email and every
//!    provider-side id its tokens hold;
//! 2. it deletes every row each planned provider backend contributed, with
//!    the same statements a disconnect's purge runs, without the purge's
//!    attestation row (that row names the user, and would be deleted with
//!    the account in this same transaction: the receipt records the counts
//!    instead);
//! 3. it clears the account itself (`clear_user_in_transaction`),
//!    re-reading every purged table and refusing to commit while a row
//!    survives;
//! 4. it makes sure a live tombstone names each provider-side id and the
//!    email ([`super::import_tombstones`]);
//! 5. it appends a [`DeletionReceipt`] chained to the previous one.
//!
//! A failure at any step rolls back all of it: the account and its provider
//! data stay, and no receipt or tombstone is written.
//!
//! A receipt holds no user id. It names the account by its
//! [`subject_hash`](super::import_tombstones::subject_hash), and records who
//! asked, the survivor re-read's result per table, what each provider said
//! about its grant, the copies this deletion could not delete, and the
//! tombstones it left. Its `hash` is the SHA-256 of the previous receipt's
//! hash, a newline, and the canonical JSON of its other fields
//! ([`DeletionReceipt::computed_hash`]); the first receipt's `prev_hash` is
//! [`GENESIS_HASH`]. Every hashed field is stored as the exact text that was
//! hashed, so [`verify_receipt_chain`] recomputes each link from the stored
//! columns alone. A chain proves no receipt was altered or removed from the
//! middle; removing the newest receipts leaves a shorter chain that still
//! verifies, so an operator who needs that guarantee records the head
//! ([`ChainVerification::head`]) somewhere the database cannot reach.
//!
//! ISOLATION EXEMPTION: `deletion_receipts` carries neither `tenant_id` nor
//! `user_id`: it outlives the account and must not point at it, and an
//! account deletion spans every tenant. It is read only by the super-admin
//! route an operator verifies the chain through.

use std::collections::BTreeMap;

use async_trait::async_trait;
use chrono::{DateTime, SecondsFormat, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::import_tombstones::sha256_hex;

/// The `prev_hash` of the first receipt: 64 zeros.
pub const GENESIS_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// Who asked for an account to be deleted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeletionActor {
    /// The athlete, from the app.
    Athlete,
    /// An operator, from an admin surface.
    Operator,
    /// The athlete, from a messaging channel (`/deleteaccount`).
    Messaging,
}

impl DeletionActor {
    /// The receipt's `actor` value. A persisted value: receipts already hold
    /// it, and the hash covers it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Athlete => "athlete",
            Self::Operator => "operator",
            Self::Messaging => "messaging",
        }
    }
}

/// One provider backend whose data the deletion purges, in the tenant that
/// holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedBackend {
    /// Tenant the rows live under.
    pub tenant_id: TenantId,
    /// The backend the rows name (`strava`, `sciotte`, …).
    pub backend: String,
}

/// What the caller decided before the transaction: the account, who asked,
/// the backends to purge, and what it already knows it cannot delete.
#[derive(Debug, Clone)]
pub struct AccountDeletionPlan {
    /// The account to delete.
    pub user_id: Uuid,
    /// Who asked.
    pub actor: DeletionActor,
    /// The Firebase project the account's identity is queued for deletion
    /// in, when this server has one.
    pub firebase_project: Option<String>,
    /// Every backend whose data is purged, both halves of each coalesced
    /// pair.
    pub backends: Vec<PlannedBackend>,
    /// What each provider said about its grant, as the canonical JSON the
    /// receipt stores and hashes verbatim.
    pub revocations: String,
    /// The copies this deletion cannot delete, each a short sentence naming
    /// the copy and why. Stored in the receipt as a JSON list.
    pub not_deleted: Vec<String>,
}

/// One stored receipt, every hashed field as the text that was hashed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeletionReceipt {
    /// Row id.
    pub id: String,
    /// Position in the chain, from 1.
    pub seq: i64,
    /// The previous receipt's `hash`, [`GENESIS_HASH`] for the first.
    pub prev_hash: String,
    /// This receipt's hash ([`Self::computed_hash`] when it was written).
    pub hash: String,
    /// When the deletion committed, RFC 3339 with microseconds, UTC.
    pub deleted_at: String,
    /// The deleted account's subject hash.
    pub subject_hash: String,
    /// [`DeletionActor::as_str`].
    pub actor: String,
    /// JSON: the survivor re-read per table, and the rows removed.
    pub scan_result: String,
    /// JSON: what each provider said about its grant.
    pub revocations: String,
    /// JSON list: the copies this deletion could not delete.
    pub not_deleted: String,
    /// JSON list: the tombstones this deletion left live.
    pub tombstone_ids: String,
}

/// The fields a receipt's hash covers besides `prev_hash`, in a fixed order.
#[derive(Serialize)]
struct ReceiptBody<'a> {
    id: &'a str,
    seq: i64,
    deleted_at: &'a str,
    subject_hash: &'a str,
    actor: &'a str,
    scan_result: &'a str,
    revocations: &'a str,
    not_deleted: &'a str,
    tombstone_ids: &'a str,
}

impl DeletionReceipt {
    /// The hash this receipt's fields produce: SHA-256 of `prev_hash`, a
    /// newline, and the canonical JSON of every other field but `hash`.
    ///
    /// # Errors
    /// Returns an internal error if the body cannot be serialized, which a
    /// struct of strings and an integer never fails to be.
    pub fn computed_hash(&self) -> AppResult<String> {
        let body = serde_json::to_string(&ReceiptBody {
            id: &self.id,
            seq: self.seq,
            deleted_at: &self.deleted_at,
            subject_hash: &self.subject_hash,
            actor: &self.actor,
            scan_result: &self.scan_result,
            revocations: &self.revocations,
            not_deleted: &self.not_deleted,
            tombstone_ids: &self.tombstone_ids,
        })
        .map_err(|e| AppError::internal(format!("Failed to encode a deletion receipt: {e}")))?;
        let mut preimage = Vec::with_capacity(self.prev_hash.len() + 1 + body.len());
        preimage.extend_from_slice(self.prev_hash.as_bytes());
        preimage.push(b'\n');
        preimage.extend_from_slice(body.as_bytes());
        Ok(sha256_hex(&preimage))
    }

    /// The time a receipt records for a deletion committing at `at`:
    /// RFC 3339, microseconds, `Z`.
    #[must_use]
    pub fn timestamp(at: DateTime<Utc>) -> String {
        at.to_rfc3339_opts(SecondsFormat::Micros, true)
    }
}

/// Where a chain stops verifying.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChainBreak {
    /// The `seq` of the first receipt that does not verify (or the one
    /// expected where a receipt is missing).
    pub seq: i64,
    /// That receipt's id, when it exists.
    pub id: Option<String>,
    /// What does not hold there.
    pub reason: String,
}

/// The outcome of [`verify_receipt_chain`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChainVerification {
    /// Receipts that verified before the first break (all of them when
    /// there is none).
    pub verified: usize,
    /// The newest receipt's `(seq, hash)`, for an operator to record
    /// outside the database.
    pub head: Option<(i64, String)>,
    /// The first broken link, `None` when the whole chain verifies.
    pub first_break: Option<ChainBreak>,
}

/// Verify a whole chain, `receipts` in `seq` order from the first.
///
/// Each `seq` follows the one before it from 1, each `prev_hash` is the
/// previous `hash` ([`GENESIS_HASH`] for the first), and each `hash` is the
/// one its fields produce. Stops at the first link that does not hold.
#[must_use]
pub fn verify_receipt_chain(receipts: &[DeletionReceipt]) -> ChainVerification {
    let head = receipts.last().map(|r| (r.seq, r.hash.clone()));
    let mut expected_prev = GENESIS_HASH.to_owned();
    for (index, receipt) in receipts.iter().enumerate() {
        let expected_seq = i64::try_from(index).unwrap_or(i64::MAX).saturating_add(1);
        let reason = if receipt.seq == expected_seq {
            link_fault(receipt, &expected_prev)
        } else {
            Some(format!(
                "expected seq {expected_seq} here but found seq {}: a receipt is missing or out of order",
                receipt.seq
            ))
        };
        if let Some(reason) = reason {
            return ChainVerification {
                verified: index,
                head,
                first_break: Some(ChainBreak {
                    seq: expected_seq,
                    id: Some(receipt.id.clone()),
                    reason,
                }),
            };
        }
        expected_prev.clone_from(&receipt.hash);
    }
    ChainVerification {
        verified: receipts.len(),
        head,
        first_break: None,
    }
}

/// What is wrong with one receipt whose `seq` is in place, or `None`.
fn link_fault(receipt: &DeletionReceipt, expected_prev: &str) -> Option<String> {
    if receipt.prev_hash != expected_prev {
        return Some(format!(
            "prev_hash {} is not the previous receipt's hash {expected_prev}",
            receipt.prev_hash
        ));
    }
    match receipt.computed_hash() {
        Ok(hash) if hash == receipt.hash => None,
        Ok(hash) => Some(format!(
            "stored hash {} does not match the hash of its fields {hash}: the row was altered",
            receipt.hash
        )),
        Err(e) => Some(format!("its fields could not be hashed: {e}")),
    }
}

/// What an account deletion did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountDeletion {
    /// Rows the account clear removed itself, per table.
    pub rows_removed: BTreeMap<String, u64>,
    /// Rows each planned backend's purge removed, per `backend@tenant`, per
    /// table.
    pub provider_rows_removed: BTreeMap<String, BTreeMap<String, u64>>,
    /// The receipt the deletion appended.
    pub receipt: DeletionReceipt,
    /// The live tombstones the deletion left (written, or already live).
    pub tombstone_ids: Vec<String>,
}

impl AccountDeletion {
    /// Rows the account clear removed from `table`; zero when it held none.
    #[must_use]
    pub fn removed_from(&self, table: &str) -> u64 {
        self.rows_removed.get(table).copied().unwrap_or(0)
    }
}

/// Account deletion with its receipt, and the receipt chain.
#[async_trait]
pub trait AccountDeletionRepository: Send + Sync {
    /// Delete the account `plan` names, in one transaction, as the module
    /// documentation describes.
    ///
    /// # Errors
    /// Returns a not-found error for an unknown account, the conflict a row
    /// still referencing the account raises (`ErrorCode::ResourceLocked`),
    /// an internal error when a purged table still holds a row for it, and a
    /// database error for any other failure, including a receipt racing
    /// another deletion's for the same `seq`. Nothing is committed then.
    async fn delete_account(&self, plan: &AccountDeletionPlan) -> AppResult<AccountDeletion>;

    /// Up to `limit` receipts after `after_seq`, in `seq` order.
    ///
    /// # Errors
    /// Returns a database error when the rows cannot be read or decoded.
    async fn list_deletion_receipts(
        &self,
        after_seq: i64,
        limit: i64,
    ) -> AppResult<Vec<DeletionReceipt>>;
}

/// The account's email, `$1` its id through the backend's uuid codec.
pub(crate) const SUBJECT_EMAIL_SQL: &str = "SELECT email FROM users WHERE id = $1";

/// Every provider-side id the account's tokens hold, `$1` its id as text. A
/// scrape session stores none. Scoped by `user_id` alone, across every
/// tenant: the deletion removes the account from all of them.
pub(crate) const PROVIDER_IDENTITIES_SQL: &str = r"
            SELECT DISTINCT provider, provider_user_id
            FROM user_oauth_tokens
            WHERE CAST(user_id AS TEXT) = $1
              AND provider_user_id IS NOT NULL AND provider_user_id <> ''
            ORDER BY provider, provider_user_id
            ";

/// The newest receipt's `seq` and `hash`.
pub(crate) const RECEIPT_HEAD_SQL: &str =
    "SELECT seq, hash FROM deletion_receipts ORDER BY seq DESC LIMIT 1";

/// Append one receipt.
pub(crate) const INSERT_RECEIPT_SQL: &str = r"
            INSERT INTO deletion_receipts
                (id, seq, prev_hash, hash, deleted_at, subject_hash, actor,
                 scan_result, revocations, not_deleted, tombstone_ids)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
            ";

/// Up to `$2` receipts after seq `$1`.
pub(crate) const LIST_RECEIPTS_SQL: &str = r"
            SELECT id, seq, prev_hash, hash, deleted_at, subject_hash, actor,
                   scan_result, revocations, not_deleted, tombstone_ids
            FROM deletion_receipts
            WHERE seq > $1
            ORDER BY seq
            LIMIT $2
            ";

/// Decode one receipt row.
///
/// # Errors
/// Returns a database error when a column is missing or does not decode.
pub(crate) fn receipt_from_row<R>(row: &R) -> AppResult<DeletionReceipt>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    i64: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let text = |name: &str| -> AppResult<String> {
        row.try_get(name).map_err(|e| {
            AppError::database(format!("Failed to read deletion_receipts.{name}: {e}"))
        })
    };
    Ok(DeletionReceipt {
        id: text("id")?,
        seq: row.try_get("seq").map_err(|e| {
            AppError::database(format!("Failed to read deletion_receipts.seq: {e}"))
        })?,
        prev_hash: text("prev_hash")?,
        hash: text("hash")?,
        deleted_at: text("deleted_at")?,
        subject_hash: text("subject_hash")?,
        actor: text("actor")?,
        scan_result: text("scan_result")?,
        revocations: text("revocations")?,
        not_deleted: text("not_deleted")?,
        tombstone_ids: text("tombstone_ids")?,
    })
}

/// The scan result a receipt records, as JSON.
///
/// Every table the survivor read covered with what it found there (zero,
/// since the transaction commits only then), the rows the account clear
/// removed, and the rows each backend's purge removed.
///
/// # Errors
/// Returns an internal error if the maps cannot be serialized.
pub fn scan_result_json(
    tables_rescanned: &[&str],
    rows_removed: &BTreeMap<String, u64>,
    provider_rows_removed: &BTreeMap<String, BTreeMap<String, u64>>,
) -> AppResult<String> {
    let rescanned: BTreeMap<&str, u64> = tables_rescanned.iter().map(|t| (*t, 0)).collect();
    serde_json::to_string(&serde_json::json!({
        "survivors_by_table": rescanned,
        "rows_removed": rows_removed,
        "provider_rows_removed": provider_rows_removed,
    }))
    .map_err(|e| AppError::internal(format!("Failed to encode a deletion scan result: {e}")))
}

/// A list of strings as the JSON text a receipt stores and hashes.
///
/// # Errors
/// Returns an internal error if the list cannot be serialized.
pub fn json_list(items: &[String]) -> AppResult<String> {
    serde_json::to_string(items)
        .map_err(|e| AppError::internal(format!("Failed to encode a receipt list: {e}")))
}

/// Emit the whole [`AccountDeletionRepository`] implementation for one
/// backend type: `$ids` is its uuid codec and `$purge` its `UserPurge`. The
/// invoking shell must `use` every statement and helper named here, the user
/// clear's own (see `clear_user_in_transaction`), the tombstone ones
/// (`ensure_tombstone_in_transaction`), the provider purge statements and
/// `TransactionGuard`.
macro_rules! impl_account_deletion_repository {
    ($ty:ty, $ids:ident, $purge:ident) => {
        #[async_trait::async_trait]
        impl AccountDeletionRepository for $ty {
            async fn delete_account(
                &self,
                plan: &AccountDeletionPlan,
            ) -> AppResult<AccountDeletion> {
                let user_id = plan.user_id;
                let owner = user_id.to_string();
                let tx = self.pool().begin().await.map_err(|e| {
                    AppError::database(format!("Failed to begin the account deletion: {e}"))
                })?;
                let mut guard = TransactionGuard::new(tx);

                // What the tombstones name, read before anything is deleted.
                let email: String = sqlx::query(SUBJECT_EMAIL_SQL)
                    .bind($ids::bind(user_id))
                    .fetch_optional(guard.executor()?)
                    .await
                    .map_err(|e| AppError::database(format!("Failed to read the account: {e}")))?
                    .ok_or_else(|| AppError::not_found(format!("User {user_id} not found")))?
                    .try_get("email")
                    .map_err(|e| AppError::database(format!("Failed to read users.email: {e}")))?;
                let identity_rows = sqlx::query(PROVIDER_IDENTITIES_SQL)
                    .bind(&owner)
                    .fetch_all(guard.executor()?)
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to read the provider identities: {e}"))
                    })?;
                let mut identities = Vec::with_capacity(identity_rows.len());
                for row in &identity_rows {
                    let provider: String = row.try_get("provider").map_err(|e| {
                        AppError::database(format!("Failed to read a token's provider: {e}"))
                    })?;
                    let provider_user_id: String =
                        row.try_get("provider_user_id").map_err(|e| {
                            AppError::database(format!("Failed to read a provider user id: {e}"))
                        })?;
                    identities.push((provider, provider_identity_hash(&provider_user_id)));
                }
                identities.dedup();

                let mut provider_rows_removed = BTreeMap::new();
                for planned in &plan.backends {
                    let tenant = planned.tenant_id.to_string();
                    let mut removed = BTreeMap::new();
                    for (table, statement) in USER_WELLNESS_ROWS_SQL
                        .iter()
                        .chain(USER_ACTIVITY_ROWS_SQL.iter())
                        .chain(USER_SYNC_STATE_SQL.iter())
                    {
                        let count = sqlx::query(statement)
                            .bind(&owner)
                            .bind(&tenant)
                            .bind(&planned.backend)
                            .execute(guard.executor()?)
                            .await
                            .map_err(|e| delete_user_error(user_id, &e))?
                            .rows_affected();
                        if count > 0 {
                            removed.insert((*table).to_owned(), count);
                        }
                    }
                    provider_rows_removed.insert(format!("{}@{tenant}", planned.backend), removed);
                }

                let rows_removed = clear_user_in_transaction!(
                    guard,
                    $ids,
                    user_id,
                    $purge,
                    plan.firebase_project.as_deref()
                );

                let now = Utc::now();
                let subject = subject_hash(user_id);
                let mut tombstone_ids = Vec::with_capacity(identities.len() + 1);
                for (provider, identity_hash) in &identities {
                    tombstone_ids.push(ensure_tombstone_in_transaction!(
                        guard,
                        &subject,
                        TombstoneKind::ProviderAccount,
                        Some(provider.as_str()),
                        identity_hash.as_str(),
                        now
                    ));
                }
                let email_hash = email_identity_hash(&email);
                tombstone_ids.push(ensure_tombstone_in_transaction!(
                    guard,
                    &subject,
                    TombstoneKind::Email,
                    None,
                    email_hash.as_str(),
                    now
                ));

                let head = sqlx::query(RECEIPT_HEAD_SQL)
                    .fetch_optional(guard.executor()?)
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to read the receipt chain head: {e}"))
                    })?;
                let (seq, prev_hash) = match head {
                    Some(row) => {
                        let seq: i64 = row.try_get("seq").map_err(|e| {
                            AppError::database(format!("Failed to read deletion_receipts.seq: {e}"))
                        })?;
                        let hash: String = row.try_get("hash").map_err(|e| {
                            AppError::database(format!(
                                "Failed to read deletion_receipts.hash: {e}"
                            ))
                        })?;
                        (seq.saturating_add(1), hash)
                    }
                    None => (1, GENESIS_HASH.to_owned()),
                };
                let mut receipt = DeletionReceipt {
                    id: Uuid::new_v4().to_string(),
                    seq,
                    prev_hash,
                    hash: String::new(),
                    deleted_at: DeletionReceipt::timestamp(now),
                    subject_hash: subject,
                    actor: plan.actor.as_str().to_owned(),
                    scan_result: scan_result_json(
                        $purge.tables,
                        &rows_removed,
                        &provider_rows_removed,
                    )?,
                    revocations: plan.revocations.clone(),
                    not_deleted: json_list(&plan.not_deleted)?,
                    tombstone_ids: json_list(&tombstone_ids)?,
                };
                receipt.hash = receipt.computed_hash()?;
                sqlx::query(INSERT_RECEIPT_SQL)
                    .bind(&receipt.id)
                    .bind(receipt.seq)
                    .bind(&receipt.prev_hash)
                    .bind(&receipt.hash)
                    .bind(&receipt.deleted_at)
                    .bind(&receipt.subject_hash)
                    .bind(&receipt.actor)
                    .bind(&receipt.scan_result)
                    .bind(&receipt.revocations)
                    .bind(&receipt.not_deleted)
                    .bind(&receipt.tombstone_ids)
                    .execute(guard.executor()?)
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to append the deletion receipt: {e}"))
                    })?;

                guard.commit().await?;
                Ok(AccountDeletion {
                    rows_removed,
                    provider_rows_removed,
                    receipt,
                    tombstone_ids,
                })
            }

            async fn list_deletion_receipts(
                &self,
                after_seq: i64,
                limit: i64,
            ) -> AppResult<Vec<DeletionReceipt>> {
                let rows = sqlx::query(LIST_RECEIPTS_SQL)
                    .bind(after_seq)
                    .bind(limit)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to list deletion receipts: {e}"))
                    })?;
                rows.iter().map(receipt_from_row).collect()
            }
        }
    };
}
pub(crate) use impl_account_deletion_repository;

#[cfg(test)]
mod tests {
    use super::*;

    /// A chain of `n` receipts, each sealed as `delete_account` seals it.
    fn chain(n: i64) -> Vec<DeletionReceipt> {
        let mut prev = GENESIS_HASH.to_owned();
        (1..=n)
            .map(|seq| {
                let mut receipt = DeletionReceipt {
                    id: format!("receipt-{seq}"),
                    seq,
                    prev_hash: prev.clone(),
                    hash: String::new(),
                    deleted_at: "2026-10-20T12:00:00.000001Z".to_owned(),
                    subject_hash: sha256_hex(format!("subject-{seq}").as_bytes()),
                    actor: DeletionActor::Athlete.as_str().to_owned(),
                    scan_result: r#"{"survivors_by_table":{"users":0}}"#.to_owned(),
                    revocations: "[]".to_owned(),
                    not_deleted: json_list(&["posthog_person".to_owned()]).unwrap(),
                    tombstone_ids: "[]".to_owned(),
                };
                receipt.hash = receipt.computed_hash().unwrap();
                prev.clone_from(&receipt.hash);
                receipt
            })
            .collect()
    }

    #[test]
    fn an_intact_chain_verifies_and_reports_its_head() {
        let receipts = chain(3);
        let verification = verify_receipt_chain(&receipts);
        assert_eq!(verification.verified, 3);
        assert_eq!(verification.first_break, None);
        assert_eq!(verification.head, Some((3, receipts[2].hash.clone())));
    }

    #[test]
    fn an_empty_chain_verifies_with_no_head() {
        let verification = verify_receipt_chain(&[]);
        assert_eq!(verification.verified, 0);
        assert_eq!(verification.head, None);
        assert_eq!(verification.first_break, None);
    }

    #[test]
    fn an_altered_field_breaks_the_link_it_is_in() {
        let mut receipts = chain(3);
        receipts[1].actor = DeletionActor::Operator.as_str().to_owned();
        let verification = verify_receipt_chain(&receipts);
        assert_eq!(verification.verified, 1);
        let fault = verification.first_break.unwrap();
        assert_eq!(fault.seq, 2);
        assert_eq!(fault.id.as_deref(), Some("receipt-2"));
        assert!(fault.reason.contains("altered"), "{}", fault.reason);
    }

    #[test]
    fn a_rehashed_row_breaks_the_next_link() {
        let mut receipts = chain(3);
        receipts[1].not_deleted = "[]".to_owned();
        receipts[1].hash = receipts[1].computed_hash().unwrap();
        let fault = verify_receipt_chain(&receipts).first_break.unwrap();
        assert_eq!(fault.seq, 3);
        assert!(fault.reason.contains("prev_hash"), "{}", fault.reason);
    }

    #[test]
    fn a_removed_receipt_is_named_where_it_is_missing() {
        let mut receipts = chain(3);
        receipts.remove(1);
        let fault = verify_receipt_chain(&receipts).first_break.unwrap();
        assert_eq!(fault.seq, 2);
        assert!(fault.reason.contains("missing"), "{}", fault.reason);
    }

    #[test]
    fn a_receipt_timestamp_keeps_microseconds_in_utc() {
        let at = DateTime::parse_from_rfc3339("2026-10-20T12:34:56.123456789+02:00")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(
            DeletionReceipt::timestamp(at),
            "2026-10-20T10:34:56.123456Z"
        );
    }
}
