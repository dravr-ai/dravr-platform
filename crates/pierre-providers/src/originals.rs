// ABOUTME: OriginalsStore — the one store of the files athletes uploaded: bytes in barrage's blob store, a row naming each
// ABOUTME: Every write is verified against the file's SHA-256; the drain moves bytes the table still holds into blobs

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # The originals store
//!
//! Raw original files had three byte stores, each with its own retention
//! (Athlete Data Consolidation, D10): uploaded `.fit` files in a table
//! column, pushed Garmin files in a decode queue, an import's recordings in
//! dravr-barrage's blob store. Uploaded files now live where an import's
//! recordings live (S12): in the blob store, under the key
//! `t/<tenant>/u/<user>/upload/<sha256>` ([`BlobKey::upload`]), apart from an
//! import's originals so that deleting one never takes the other. A row in
//! `uploaded_activity_files` names each file — owner, hash, size, origin —
//! and is what scopes a read to the athlete who uploaded it. The pushed
//! Garmin files stay a short-lived decode queue (DECISION 3).
//!
//! - A write stores the bytes first and checks what the blob store says it
//!   stored (digest and size) against the file's hash before the row is
//!   written, so a row never names bytes the store does not hold.
//! - A read finds the row first, then the bytes.
//! - A delete removes the bytes first, then the row, so a failure part-way
//!   leaves a row a retry finds and finishes, never bytes no row names.
//! - Files uploaded before this store kept them still hold their bytes in
//!   the table. [`OriginalsStore::drain_pending`] copies them out, verified
//!   the same way, and clears the column; a read of a file not drained yet
//!   is served from the table's copy, which only the drain clears.
//! - A deleted account's files are erased with everything else it holds in
//!   the blob store ([`OriginalsStore::erase_user`]), from the erasure queue
//!   the account deletion writes in its own transaction.
//!
//! Where no blob store is placed (a Postgres deployment whose bucket has not
//! been applied yet), the store keeps uploads in the table, as it did before
//! the blob store existed: a write records the bytes in the row, a read
//! serves them from it, a delete removes the row and the bytes with it, and
//! nothing drains. The day a blob store is placed, new uploads go to it and
//! the drain moves the table's bytes out, so turning the bucket on needs no
//! deploy of its own and never strands a file.

use std::fmt;
use std::sync::Arc;

use bytes::Bytes;
use dravr_barrage::blob::{BlobKey, BlobMeta, BlobPrefix, BlobStore};
use dravr_barrage::digest::Sha256;
use dravr_barrage::{BarrageError, Scope, UserId};
use pierre_database::repositories::{
    PendingUploadBytes, UploadedActivityFileRepository, UPLOAD_ORIGIN,
};
use tracing::{info, warn};
use uuid::Uuid;

use crate::errors::{AppError, AppResult};
use crate::models::TenantId;

/// The media type an uploaded activity file is stored with.
const FIT_MEDIA_TYPE: &str = "application/vnd.ant.fit";

/// What one drain pass did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DrainReport {
    /// Files copied to the blob store, verified, and cleared from the table.
    pub drained: usize,
    /// Files left in the table: their bytes do not match their recorded hash
    /// or size, or their ids do not parse. Logged one by one, and stepped
    /// over, so they never hold back the files behind them.
    pub refused: usize,
}

/// Where the bytes of uploaded files are kept.
enum Backing {
    /// In the blob store; the table names each file.
    Blobs(Arc<dyn BlobStore>),
    /// In the table, because no blob store is placed here, for this reason.
    Table(String),
}

/// The files athletes uploaded, over the blob store and the table naming them.
pub struct OriginalsStore {
    backing: Backing,
    files: Arc<dyn UploadedActivityFileRepository>,
}

impl fmt::Debug for OriginalsStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let backing = match &self.backing {
            Backing::Blobs(_) => "blob store",
            Backing::Table(_) => "table",
        };
        f.debug_struct("OriginalsStore")
            .field("backing", &backing)
            .finish_non_exhaustive()
    }
}

impl OriginalsStore {
    /// The store over `blobs` — or over the table alone, for the reason
    /// `blobs` carries — and the table naming the files.
    #[must_use]
    pub fn new(
        blobs: Result<Arc<dyn BlobStore>, String>,
        files: Arc<dyn UploadedActivityFileRepository>,
    ) -> Self {
        let backing = match blobs {
            Ok(store) => Backing::Blobs(store),
            Err(reason) => Backing::Table(reason),
        };
        Self { backing, files }
    }

    /// Whether files are kept in a blob store here (and so drained into it
    /// and erased from it), rather than in the table.
    #[must_use]
    pub const fn keeps_blobs(&self) -> bool {
        matches!(self.backing, Backing::Blobs(_))
    }

    /// Why no blob store keeps the files here, `None` when one does.
    #[must_use]
    pub fn table_reason(&self) -> Option<&str> {
        match &self.backing {
            Backing::Blobs(_) => None,
            Backing::Table(reason) => Some(reason),
        }
    }

    /// Keep the file `file_sha256` the athlete uploaded in this tenant.
    /// Returns `true` when it was new to them, `false` when they already
    /// held it.
    ///
    /// # Errors
    ///
    /// [`AppError::invalid_input`] when the hash is not 64 lowercase hex
    /// characters, or (kept in the table) the bytes are not the ones it
    /// names; an internal error when the blob store reports other bytes than
    /// the hash names; a database error when the row cannot be written.
    pub async fn put_upload(
        &self,
        tenant_id: &TenantId,
        user_id: Uuid,
        file_sha256: &str,
        bytes: Bytes,
    ) -> AppResult<bool> {
        let size = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        match &self.backing {
            Backing::Blobs(blobs) => {
                put_verified(blobs, tenant_id, user_id, file_sha256, bytes).await?;
                self.files
                    .record_uploaded_file(tenant_id, user_id, file_sha256, size, UPLOAD_ORIGIN)
                    .await
            }
            Backing::Table(_) => {
                let named = Sha256::parse_hex(file_sha256).map_err(|e| {
                    AppError::invalid_input(format!("uploaded file hash {file_sha256}: {e}"))
                })?;
                if Sha256::of(&bytes) != named {
                    return Err(AppError::invalid_input(format!(
                        "the uploaded bytes are not the file {file_sha256} names"
                    )));
                }
                self.files
                    .record_uploaded_file_bytes(
                        tenant_id,
                        user_id,
                        file_sha256,
                        &bytes,
                        UPLOAD_ORIGIN,
                    )
                    .await
            }
        }
    }

    /// The bytes of the file `file_sha256` the athlete holds in this tenant,
    /// `None` when they hold no such file.
    ///
    /// A file uploaded before the blob store kept files, and not drained
    /// yet, is served from the table; only the drain clears the table's
    /// copy, so a read never changes what an older instance still serving
    /// during a rollout reads.
    ///
    /// # Errors
    ///
    /// An internal error when the row names a file neither the blob store
    /// nor the table holds, or the store fails.
    pub async fn get_upload(
        &self,
        tenant_id: &TenantId,
        user_id: Uuid,
        file_sha256: &str,
    ) -> AppResult<Option<Bytes>> {
        if !self
            .files
            .holds_uploaded_file(tenant_id, user_id, file_sha256)
            .await?
        {
            return Ok(None);
        }
        if let Backing::Blobs(blobs) = &self.backing {
            let key = upload_key(tenant_id, user_id, file_sha256)?;
            if let Some(bytes) = blobs.get(&key).await.map_err(|e| blob_error(&e))? {
                return Ok(Some(bytes));
            }
        }
        self.files
            .pending_upload_bytes(Some((tenant_id, user_id, file_sha256)), 1, 0)
            .await?
            .into_iter()
            .next()
            .map_or_else(
                || {
                    Err(AppError::internal(format!(
                        "uploaded file {file_sha256} is recorded but no store holds its bytes"
                    )))
                },
                |held| Ok(Some(Bytes::from(held.bytes))),
            )
    }

    /// Delete the file `file_sha256` the athlete holds in this tenant: its
    /// bytes, then its row. Returns whether they held it.
    ///
    /// # Errors
    ///
    /// The blob store's or the database's error. The row stays when the
    /// bytes could not be deleted, so the retry finds the file again.
    pub async fn delete_upload(
        &self,
        tenant_id: &TenantId,
        user_id: Uuid,
        file_sha256: &str,
    ) -> AppResult<bool> {
        if !self
            .files
            .holds_uploaded_file(tenant_id, user_id, file_sha256)
            .await?
        {
            return Ok(false);
        }
        if let Backing::Blobs(blobs) = &self.backing {
            let key = upload_key(tenant_id, user_id, file_sha256)?;
            blobs.delete(&key).await.map_err(|e| blob_error(&e))?;
        }
        self.files
            .delete_uploaded_file(tenant_id, user_id, file_sha256)
            .await
    }

    /// Copy files the table still holds bytes for into the blob store, each
    /// verified against its recorded hash and size before the table's copy
    /// is cleared, until `limit` are drained or none is left. A file refused
    /// is stepped over, so refused files never fill a pass.
    ///
    /// Nothing to do where the files are kept in the table.
    ///
    /// # Errors
    ///
    /// The database's error when the files cannot be listed. One file's
    /// failure is logged, counted, and the pass goes on.
    pub async fn drain_pending(&self, limit: i64) -> AppResult<DrainReport> {
        let Backing::Blobs(blobs) = &self.backing else {
            return Ok(DrainReport::default());
        };
        let page = limit.max(1);
        let mut report = DrainReport::default();
        loop {
            let skip = i64::try_from(report.refused).unwrap_or(i64::MAX);
            let files = self.files.pending_upload_bytes(None, page, skip).await?;
            let listed = files.len();
            for file in files {
                let name = file.file_sha256.clone();
                match self.drain_one(blobs, file).await {
                    Ok(()) => report.drained += 1,
                    Err(e) => {
                        report.refused += 1;
                        warn!(file_sha256 = %name, error = %e, "an uploaded file could not be drained to the blob store");
                    }
                }
            }
            let short = i64::try_from(listed).map_or(true, |n| n < page);
            if short || i64::try_from(report.drained).unwrap_or(i64::MAX) >= page {
                break;
            }
        }
        if report != DrainReport::default() {
            info!(
                drained = report.drained,
                refused = report.refused,
                "uploaded files drained to the blob store"
            );
        }
        Ok(report)
    }

    /// Erase every file the account `user_id` uploaded, in every tenant,
    /// with everything else the blob store keeps for it. Returns how many
    /// blobs were removed: none where the files are kept in the table, whose
    /// rows go with the account.
    ///
    /// # Errors
    ///
    /// The blob store's error.
    pub async fn erase_user(&self, user_id: Uuid) -> AppResult<u64> {
        let Backing::Blobs(blobs) = &self.backing else {
            return Ok(0);
        };
        let prefix = BlobPrefix::User(UserId(user_id));
        let mut erased = 0;
        let mut token = None;
        loop {
            let page = blobs
                .delete_prefix_page(&prefix, token)
                .await
                .map_err(|e| blob_error(&e))?;
            erased += page.deleted;
            match page.next {
                Some(next) => token = Some(next),
                None => return Ok(erased),
            }
        }
    }

    /// Copy one file the table still holds into the blob store, verified,
    /// then clear the table's copy. When the row went away meanwhile (the
    /// athlete deleted the file, or the account was erased, between the
    /// listing and the copy), the copy just written is removed, so no blob
    /// outlives the row naming it.
    async fn drain_one(
        &self,
        blobs: &Arc<dyn BlobStore>,
        file: PendingUploadBytes,
    ) -> AppResult<()> {
        let tenant_id = TenantId::parse_str(&file.tenant_id).map_err(|e| {
            AppError::invalid_input(format!("uploaded file tenant {}: {e}", file.tenant_id))
        })?;
        let user_id = Uuid::parse_str(&file.user_id).map_err(|e| {
            AppError::invalid_input(format!("uploaded file user {}: {e}", file.user_id))
        })?;
        if i64::try_from(file.bytes.len()).ok() != Some(file.byte_size) {
            return Err(AppError::internal(format!(
                "uploaded file {} holds {} bytes, its row records {}",
                file.file_sha256,
                file.bytes.len(),
                file.byte_size
            )));
        }
        put_verified(
            blobs,
            &tenant_id,
            user_id,
            &file.file_sha256,
            Bytes::from(file.bytes),
        )
        .await?;
        let cleared = self
            .files
            .clear_drained_bytes(&tenant_id, user_id, &file.file_sha256)
            .await?;
        if !cleared
            && !self
                .files
                .holds_uploaded_file(&tenant_id, user_id, &file.file_sha256)
                .await?
        {
            let key = upload_key(&tenant_id, user_id, &file.file_sha256)?;
            blobs.delete(&key).await.map_err(|e| blob_error(&e))?;
            info!(file_sha256 = %file.file_sha256, "an uploaded file was deleted while it drained; its copy was removed");
        }
        Ok(())
    }
}

/// Store `bytes` under the upload key `file_sha256` names, and refuse unless
/// the blob store reports it stored exactly those bytes: the digest the key
/// names and their length. A write that stored anything else is removed.
async fn put_verified(
    blobs: &Arc<dyn BlobStore>,
    tenant_id: &TenantId,
    user_id: Uuid,
    file_sha256: &str,
    bytes: Bytes,
) -> AppResult<()> {
    let key = upload_key(tenant_id, user_id, file_sha256)?;
    let size = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    let stat = blobs
        .put(&key, bytes, &BlobMeta::new(FIT_MEDIA_TYPE))
        .await
        .map_err(|e| blob_error(&e))?;
    if stat.sha256.to_hex() == file_sha256 && stat.size == size {
        return Ok(());
    }
    // The bytes are not the ones the hash names: never leave them under it.
    blobs.delete(&key).await.map_err(|e| blob_error(&e))?;
    Err(AppError::internal(format!(
        "the blob store stored {} bytes with digest {} under upload {file_sha256} ({size} bytes)",
        stat.size,
        stat.sha256.to_hex()
    )))
}

/// The blob key of the file `file_sha256` the athlete uploaded in a tenant.
fn upload_key(tenant_id: &TenantId, user_id: Uuid, file_sha256: &str) -> AppResult<BlobKey> {
    let digest = Sha256::parse_hex(file_sha256)
        .map_err(|e| AppError::invalid_input(format!("uploaded file hash {file_sha256}: {e}")))?;
    Ok(BlobKey::upload(
        Scope::new(tenant_id.as_uuid(), user_id),
        digest,
    ))
}

/// A blob store failure, as the platform reports one.
fn blob_error(e: &BarrageError) -> AppError {
    AppError::internal(format!("blob store: {e}"))
}
