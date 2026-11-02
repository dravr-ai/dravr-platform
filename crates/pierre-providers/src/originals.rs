// ABOUTME: OriginalsStore — the one store of the files athletes uploaded: bytes in barrage's blob store, a row naming each
// ABOUTME: Every write is verified against the file's SHA-256; with no blob store placed, uploads are refused

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
//! - A deleted account's files are erased with everything else it holds in
//!   the blob store ([`OriginalsStore::erase_user`]), from the erasure queue
//!   the account deletion writes in its own transaction.
//!
//! The table held the bytes of the files uploaded before this store kept
//! them; a drain copied each out, verified, and the column was dropped
//! (Athlete Data Consolidation, S12; carnet#1309). No bytes are kept in the
//! table any more, so where no blob store is placed (a Postgres deployment
//! with no bucket) an upload is refused as unavailable before anything is
//! written, a read of a file a row names fails, and a delete fails before it
//! removes the row, so its retry finishes once a store is placed.

use std::fmt;
use std::sync::Arc;

use bytes::Bytes;
use dravr_barrage::blob::{BlobKey, BlobMeta, BlobPrefix, BlobStore};
use dravr_barrage::digest::Sha256;
use dravr_barrage::{BarrageError, Scope, UserId};
use pierre_database::repositories::{UploadedActivityFileRepository, UPLOAD_ORIGIN};
use uuid::Uuid;

use crate::errors::{AppError, AppResult};
use crate::models::TenantId;

/// The media type an uploaded activity file is stored with.
const FIT_MEDIA_TYPE: &str = "application/vnd.ant.fit";

/// The files athletes uploaded, over the blob store and the table naming them.
pub struct OriginalsStore {
    /// The blob store keeping the bytes, or why none is placed here.
    blobs: Result<Arc<dyn BlobStore>, String>,
    files: Arc<dyn UploadedActivityFileRepository>,
}

impl fmt::Debug for OriginalsStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let blobs = match &self.blobs {
            Ok(_) => "placed",
            Err(reason) => reason.as_str(),
        };
        f.debug_struct("OriginalsStore")
            .field("blobs", &blobs)
            .finish_non_exhaustive()
    }
}

impl OriginalsStore {
    /// The store over `blobs`, or over none for the reason `blobs` carries,
    /// and the table naming the files.
    #[must_use]
    pub fn new(
        blobs: Result<Arc<dyn BlobStore>, String>,
        files: Arc<dyn UploadedActivityFileRepository>,
    ) -> Self {
        Self { blobs, files }
    }

    /// The blob store keeping the files here.
    ///
    /// # Errors
    ///
    /// [`AppError::resource_unavailable`] naming why none is placed.
    fn blobs(&self) -> AppResult<&Arc<dyn BlobStore>> {
        self.blobs.as_ref().map_err(|reason| {
            AppError::resource_unavailable(format!("uploaded files cannot be kept: {reason}"))
        })
    }

    /// Keep the file `file_sha256` the athlete uploaded in this tenant.
    /// Returns `true` when it was new to them, `false` when they already
    /// held it.
    ///
    /// # Errors
    ///
    /// [`AppError::resource_unavailable`] when no blob store is placed here,
    /// before anything is written; [`AppError::invalid_input`] when the hash
    /// is not 64 lowercase hex characters; an internal error when the blob
    /// store reports other bytes than the hash names; a database error when
    /// the row cannot be written.
    pub async fn put_upload(
        &self,
        tenant_id: &TenantId,
        user_id: Uuid,
        file_sha256: &str,
        bytes: Bytes,
    ) -> AppResult<bool> {
        let blobs = self.blobs()?;
        let size = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        put_verified(blobs, tenant_id, user_id, file_sha256, bytes).await?;
        self.files
            .record_uploaded_file(tenant_id, user_id, file_sha256, size, UPLOAD_ORIGIN)
            .await
    }

    /// The bytes of the file `file_sha256` the athlete holds in this tenant,
    /// `None` when they hold no such file.
    ///
    /// # Errors
    ///
    /// [`AppError::resource_unavailable`] when the row names a file and no
    /// blob store is placed here; an internal error when the blob store does
    /// not hold the bytes the row names, or fails.
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
        let key = upload_key(tenant_id, user_id, file_sha256)?;
        let bytes = self
            .blobs()?
            .get(&key)
            .await
            .map_err(|e| blob_error(&e))?
            .ok_or_else(|| {
                AppError::internal(format!(
                    "uploaded file {file_sha256} is recorded but the blob store holds no bytes for it"
                ))
            })?;
        Ok(Some(bytes))
    }

    /// Delete the file `file_sha256` the athlete holds in this tenant: its
    /// bytes, then its row. Returns whether they held it.
    ///
    /// # Errors
    ///
    /// [`AppError::resource_unavailable`] when no blob store is placed here;
    /// the blob store's or the database's error. The row stays when the
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
        let key = upload_key(tenant_id, user_id, file_sha256)?;
        self.blobs()?
            .delete(&key)
            .await
            .map_err(|e| blob_error(&e))?;
        self.files
            .delete_uploaded_file(tenant_id, user_id, file_sha256)
            .await
    }

    /// Erase every file the account `user_id` uploaded, in every tenant,
    /// with everything else the blob store keeps for it. Returns how many
    /// blobs were removed: none where no blob store is placed, as nothing
    /// can be kept there.
    ///
    /// # Errors
    ///
    /// The blob store's error.
    pub async fn erase_user(&self, user_id: Uuid) -> AppResult<u64> {
        let Ok(blobs) = &self.blobs else {
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
