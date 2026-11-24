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
//! - A screenshot an athlete attaches to a feedback message is a file they
//!   uploaded too, kept under the same upload key
//!   ([`OriginalsStore::put_named_file`]); the `feedback_messages` row
//!   names it and scopes its read, as `uploaded_activity_files` does for an
//!   activity file (carnet#1392). The erasure takes it with the rest.
//!
//! # Keyed names (carnet#1701)
//!
//! A file is kept under its keyed name ([`BlobKey::keyed_upload`], a MAC of
//! the athlete and the digest under the Index key) or, kept before that name
//! was written, under its content name. Every row naming a file keeps its
//! digest, so every reader derives both names: a read tries the keyed name,
//! then the content name, and checks the bytes against the digest; a delete
//! removes both. The content names stay until carnet#1550.
//!
//! The write, the delete and the keyed-name copy of one file each run under
//! that file's lease ([`OriginalsStore::lock_file`]), held across the bytes
//! and the row naming them, so none interleaves with another. A holder
//! fences its lease after the bytes and before the row: one whose lease
//! lapsed and was taken over writes no row.
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
use dravr_barrage::store::ActivityRecordStore;
use dravr_barrage::{BarrageError, Scope, UserId};
use pierre_database::repositories::{
    UploadNameLock, UploadedActivityFileRepository, UploadedFileWrite, UPLOAD_ORIGIN,
};
use uuid::Uuid;

use crate::errors::{AppError, AppResult};
use crate::models::TenantId;
use crate::upload_provider::UploadActivityId;

/// The media type an uploaded activity file is stored with.
const FIT_MEDIA_TYPE: &str = "application/vnd.ant.fit";

/// The media type an uploaded lab report is stored with.
pub const LAB_DOCUMENT_MEDIA_TYPE: &str = "application/pdf";

/// The files athletes uploaded, over the blob store and the table naming them.
pub struct OriginalsStore {
    /// The blob store keeping the bytes, or why none is placed here.
    blobs: Result<Arc<dyn BlobStore>, String>,
    /// The context's one record store, whose Index key ([`ActivityRecordStore::keyed`])
    /// names the files: one key for the records and the uploads.
    records: Arc<dyn ActivityRecordStore>,
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
    /// the record store whose Index key names the files, and the table naming
    /// them.
    #[must_use]
    pub fn new(
        blobs: Result<Arc<dyn BlobStore>, String>,
        records: Arc<dyn ActivityRecordStore>,
        files: Arc<dyn UploadedActivityFileRepository>,
    ) -> Self {
        Self {
            blobs,
            records,
            files,
        }
    }

    /// Wait for, then hold, the lock on the athlete's file `file_sha256` in
    /// this tenant: every write, delete and keyed-name copy of it runs under
    /// it, from the bytes to the row naming them, whichever table names it.
    ///
    /// # Errors
    ///
    /// [`AppError::invalid_input`] when the hash is not 64 lowercase hex
    /// characters; [`AppError::resource_unavailable`] when another holder
    /// keeps it past the wait; a database error when a claim fails.
    pub async fn lock_file(
        &self,
        tenant_id: &TenantId,
        user_id: Uuid,
        file_sha256: &str,
    ) -> AppResult<UploadNameLock> {
        upload_key(tenant_id, user_id, file_sha256)?;
        self.files
            .lock_upload_name(tenant_id, user_id, file_sha256)
            .await
    }

    /// Renew every lease in `locks` by its token: the fence a holder of many
    /// files passes before a statement that deletes their rows at once, and
    /// renews at intervals while it works, so none lapses under it.
    ///
    /// # Errors
    ///
    /// [`AppError::resource_unavailable`] when any lease lapsed and was taken
    /// over: the caller deletes no row and retries; a database error.
    pub async fn fence_all(&self, locks: &[UploadNameLock]) -> AppResult<()> {
        for lock in locks {
            self.files.fence_upload_name(lock).await?;
        }
        Ok(())
    }

    /// The digest of the file an uploaded activity's id names, when the
    /// athlete holds it in this tenant: a digest id carries it, a keyed id
    /// names it through its row's keyed base.
    ///
    /// # Errors
    ///
    /// A database error when the read fails.
    pub async fn file_of_activity(
        &self,
        tenant_id: &TenantId,
        user_id: Uuid,
        id: UploadActivityId<'_>,
    ) -> AppResult<Option<String>> {
        match id {
            UploadActivityId::Digest { sha, .. } => Ok(Some(sha.to_owned())),
            UploadActivityId::Keyed { base, .. } => {
                self.files
                    .file_of_record_base(tenant_id, user_id, base)
                    .await
            }
        }
    }

    /// The names the file `file_sha256` may be kept under, the keyed one
    /// first ([`upload_names`]).
    fn names(
        &self,
        tenant_id: &TenantId,
        user_id: Uuid,
        file_sha256: &str,
    ) -> AppResult<[BlobKey; 2]> {
        upload_names(self.records.as_ref(), tenant_id, user_id, file_sha256)
    }

    /// The bytes of the file `file_sha256`, under its keyed name or else its
    /// content name, checked against the digest; `None` when neither holds
    /// them.
    async fn read_file(
        &self,
        tenant_id: &TenantId,
        user_id: Uuid,
        file_sha256: &str,
    ) -> AppResult<Option<Bytes>> {
        let blobs = self.blobs()?;
        for key in self.names(tenant_id, user_id, file_sha256)? {
            if let Some(bytes) = blobs.get(&key).await.map_err(|e| blob_error(&e))? {
                let read = Sha256::of(&bytes).to_hex();
                if read != file_sha256 {
                    return Err(AppError::internal(format!(
                        "the blob store holds bytes with digest {read} for upload {file_sha256}"
                    )));
                }
                return Ok(Some(bytes));
            }
        }
        Ok(None)
    }

    /// Delete both names the locked file may be kept under.
    async fn delete_names(&self, lock: &UploadNameLock) -> AppResult<()> {
        let blobs = self.blobs()?;
        for key in self.names(lock.tenant_id(), lock.user_id(), lock.file_sha256())? {
            blobs.delete(&key).await.map_err(|e| blob_error(&e))?;
        }
        Ok(())
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
        let lock = self.lock_file(tenant_id, user_id, file_sha256).await?;
        let size = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        put_verified(
            blobs,
            tenant_id,
            user_id,
            file_sha256,
            FIT_MEDIA_TYPE,
            bytes,
        )
        .await?;
        self.files.fence_upload_name(&lock).await?;
        let recorded = self
            .files
            .record_uploaded_file(&UploadedFileWrite {
                tenant_id,
                user_id,
                file_sha256,
                byte_size: size,
                origin: UPLOAD_ORIGIN,
                record_base: None,
                blob_keyed: false,
            })
            .await;
        drop(lock);
        recorded
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
        let bytes = self
            .read_file(tenant_id, user_id, file_sha256)
            .await?
            .ok_or_else(|| {
                AppError::internal(format!(
                    "uploaded file {file_sha256} is recorded but the blob store holds no bytes for it"
                ))
            })?;
        Ok(Some(bytes))
    }

    /// Delete the file `file_sha256` the athlete holds in this tenant: its
    /// bytes under both names, then its row, under its lock. Returns whether
    /// they held it.
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
        let lock = self.lock_file(tenant_id, user_id, file_sha256).await?;
        if !self
            .files
            .holds_uploaded_file(tenant_id, user_id, file_sha256)
            .await?
        {
            return Ok(false);
        }
        self.delete_names(&lock).await?;
        self.files.fence_upload_name(&lock).await?;
        let deleted = self
            .files
            .delete_uploaded_file(tenant_id, user_id, file_sha256)
            .await;
        drop(lock);
        deleted
    }

    /// Keep the locked file, one the athlete uploaded that its own table's
    /// row names — a screenshot attached to a feedback message
    /// (`feedback_messages`), a lab report (`lab_documents`) — under the
    /// upload key its digest names. The caller writes that row once this
    /// returns, still holding the lock.
    ///
    /// # Errors
    ///
    /// [`AppError::resource_unavailable`] when no blob store is placed here,
    /// before anything is written; [`AppError::invalid_input`] when the hash
    /// is not 64 lowercase hex characters; an internal error when the blob
    /// store reports other bytes than the hash names.
    pub async fn put_named_file(
        &self,
        lock: &UploadNameLock,
        media_type: &str,
        bytes: Bytes,
    ) -> AppResult<()> {
        put_verified(
            self.blobs()?,
            lock.tenant_id(),
            lock.user_id(),
            lock.file_sha256(),
            media_type,
            bytes,
        )
        .await?;
        self.files.fence_upload_name(lock).await
    }

    /// The bytes of a file the athlete uploaded in this tenant that its own
    /// table's row names, whose row the caller has already read, under its
    /// keyed name or else its content name and checked against the digest;
    /// `None` when the blob store no longer holds them.
    ///
    /// # Errors
    ///
    /// [`AppError::resource_unavailable`] when no blob store is placed here;
    /// [`AppError::invalid_input`] for a malformed hash; an internal error
    /// when the bytes are not the ones the hash names; the blob store's
    /// error.
    pub async fn get_named_file(
        &self,
        tenant_id: &TenantId,
        user_id: Uuid,
        image_sha256: &str,
    ) -> AppResult<Option<Bytes>> {
        self.read_file(tenant_id, user_id, image_sha256).await
    }

    /// Delete the locked file, one the athlete uploaded that its own table's
    /// row names, under both its names, before the caller deletes that row
    /// still holding the lock, so no row ever names bytes that are gone
    /// while a failure leaves the row for a retry.
    ///
    /// # Errors
    ///
    /// [`AppError::resource_unavailable`] when no blob store is placed here;
    /// the blob store's error.
    pub async fn delete_named_file(&self, lock: &UploadNameLock) -> AppResult<()> {
        self.delete_names(lock).await?;
        self.files.fence_upload_name(lock).await
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

/// Store `bytes` as `media_type` under the upload key `file_sha256` names,
/// and refuse unless the blob store reports it stored exactly those bytes:
/// the digest the key names and their length. A write that stored anything
/// else is removed.
async fn put_verified(
    blobs: &Arc<dyn BlobStore>,
    tenant_id: &TenantId,
    user_id: Uuid,
    file_sha256: &str,
    media_type: &str,
    bytes: Bytes,
) -> AppResult<()> {
    let key = upload_key(tenant_id, user_id, file_sha256)?;
    let size = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    let stat = blobs
        .put(&key, bytes, &BlobMeta::new(media_type))
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
///
/// An uploaded recording or a feedback screenshot: the one place an upload's
/// key is made, which the originals bundle reads its files by.
///
/// # Errors
///
/// [`AppError::invalid_input`] when the hash is not 64 hex characters.
pub fn upload_key(tenant_id: &TenantId, user_id: Uuid, file_sha256: &str) -> AppResult<BlobKey> {
    let digest = Sha256::parse_hex(file_sha256)
        .map_err(|e| AppError::invalid_input(format!("uploaded file hash {file_sha256}: {e}")))?;
    Ok(BlobKey::upload(
        Scope::new(tenant_id.as_uuid(), user_id),
        digest,
    ))
}

/// The two names an uploaded file may be kept under, the keyed one first.
///
/// Its keyed name ([`BlobKey::keyed_upload`], under the Index key `records`
/// holds), then its content name ([`upload_key`]), kept before the keyed
/// name was written and held until carnet#1550.
///
/// # Errors
///
/// [`AppError::invalid_input`] when the hash is not 64 hex characters; an
/// internal error when the Index key fails.
fn upload_names(
    records: &dyn ActivityRecordStore,
    tenant_id: &TenantId,
    user_id: Uuid,
    file_sha256: &str,
) -> AppResult<[BlobKey; 2]> {
    let content = upload_key(tenant_id, user_id, file_sha256)?;
    let keyed = keyed_upload_key(records, tenant_id, user_id, file_sha256)?;
    Ok([keyed, content])
}

/// The keyed name of the file `file_sha256` the athlete uploaded in a tenant
/// ([`BlobKey::keyed_upload`]), under the Index key `records` holds.
///
/// # Errors
///
/// [`AppError::invalid_input`] when the hash is not 64 hex characters; an
/// internal error when the Index key fails.
pub fn keyed_upload_key(
    records: &dyn ActivityRecordStore,
    tenant_id: &TenantId,
    user_id: Uuid,
    file_sha256: &str,
) -> AppResult<BlobKey> {
    let digest = Sha256::parse_hex(file_sha256)
        .map_err(|e| AppError::invalid_input(format!("uploaded file hash {file_sha256}: {e}")))?;
    BlobKey::keyed_upload(
        Scope::new(tenant_id.as_uuid(), user_id),
        records.keyed(),
        digest,
    )
    .map_err(|e| AppError::internal(format!("keyed upload name: {e}")))
}

/// A blob store failure, as the platform reports one.
fn blob_error(e: &BarrageError) -> AppError {
    AppError::internal(format!("blob store: {e}"))
}
