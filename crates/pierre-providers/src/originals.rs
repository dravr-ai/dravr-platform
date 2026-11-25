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
//! A file is written under its keyed name only ([`BlobKey::keyed_upload`], a
//! MAC of the athlete and the digest under the Index key), and its row says
//! so (`blob_keyed`). A file kept before that name was written is under its
//! content name until the `record_log` worker copies it
//! ([`OriginalsStore::copy_to_keyed_name`]). Every row naming a file keeps its
//! digest, so every reader derives both names: a read tries the keyed name,
//! then the content name, and checks the bytes against the digest; a delete
//! removes both. The content names stay until carnet#1550.
//!
//! An uploaded file's activities are named by its keyed base
//! ([`OriginalsStore::upload_record_base`], `upload-k<hex16>`), which its row
//! records; one uploaded before keeps the ids its digest names.
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
use dravr_barrage::keyed::{self, Keyed, KeyedDomain};
use dravr_barrage::store::ActivityRecordStore;
use dravr_barrage::{BarrageError, Scope, UserId};
use pierre_database::repositories::{
    ContentNamedFile, UploadHolder, UploadNameLock, UploadedActivityFileRepository,
    UploadedFileWrite, UPLOAD_ORIGIN,
};
use tracing::warn;
use uuid::Uuid;

use crate::errors::{AppError, AppResult};
use crate::models::TenantId;
use crate::upload_provider::{UploadActivityId, UPLOAD_ID_PREFIX};

/// The media type an uploaded activity file is stored with.
const FIT_MEDIA_TYPE: &str = "application/vnd.ant.fit";

/// The media type a file whose row records none is copied with.
const UNTYPED_MEDIA_TYPE: &str = "application/octet-stream";

/// The media type an uploaded lab report is stored with.
pub const LAB_DOCUMENT_MEDIA_TYPE: &str = "application/pdf";

/// What one page of the keyed-name copy of uploaded files did
/// ([`OriginalsStore::rename_uploads_page`]).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct UploadRenamePage {
    /// Files copied to their keyed name, their rows marked.
    pub copied: u64,
    /// Files whose bytes neither name holds, or whose copy failed: left as
    /// they are, the cursor past them.
    pub unreadable: u64,
    /// Where the next page starts; `None` once the holder's table is swept.
    pub next: Option<ContentNamedFile>,
}

/// What the keyed-name copy of one file did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeyedCopy {
    /// Copied, and every row naming it marked.
    Copied,
    /// No content-named row names it any more: deleted, or marked meanwhile.
    Gone,
    /// Neither name holds its bytes.
    Unreadable,
}

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

    /// The Index key the files and the records are keyed under: what a lab
    /// reading keys its records' ids with.
    #[must_use]
    pub fn keyed(&self) -> &dyn Keyed {
        self.records.keyed()
    }

    /// The keyed base of the activities the athlete's file `file_sha256`
    /// becomes in this tenant: `upload-k<hex16>`, a MAC of the athlete and
    /// the digest under the Index key ([`keyed::keyed_id`], `UploadRecordId`), so
    /// the id says nothing of the bytes and never moves when the key rotates.
    ///
    /// # Errors
    ///
    /// [`AppError::invalid_input`] when the hash is not 64 lowercase hex
    /// characters; an internal error when the Index key fails.
    pub fn upload_record_base(
        &self,
        tenant_id: &TenantId,
        user_id: Uuid,
        file_sha256: &str,
    ) -> AppResult<String> {
        upload_key(tenant_id, user_id, file_sha256)?;
        keyed::keyed_id(
            self.keyed(),
            KeyedDomain::UploadRecordId,
            UPLOAD_ID_PREFIX,
            &Scope::new(tenant_id.as_uuid(), user_id),
            file_sha256,
        )
        .map_err(|e| AppError::internal(format!("keyed upload id: {e}")))
    }

    /// The keyed name of the athlete's file `file_sha256` in this tenant
    /// ([`keyed_upload_key`]): where it is written, and what a sample row
    /// decoded from it names as its source.
    ///
    /// # Errors
    ///
    /// [`AppError::invalid_input`] when the hash is not 64 hex characters; an
    /// internal error when the Index key fails.
    pub fn keyed_name(
        &self,
        tenant_id: &TenantId,
        user_id: Uuid,
        file_sha256: &str,
    ) -> AppResult<BlobKey> {
        keyed_upload_key(self.records.as_ref(), tenant_id, user_id, file_sha256)
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

    /// Keep the file `file_sha256` the athlete uploaded in this tenant, under
    /// its keyed name, its row recording that and its keyed base
    /// ([`Self::upload_record_base`]). Returns `true` when it was new to them,
    /// `false` when they already held it: a row kept before keeps its digest
    /// ids and gains the keyed base and the keyed name beside them.
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
        let record_base = self.upload_record_base(tenant_id, user_id, file_sha256)?;
        let key = self.keyed_name(tenant_id, user_id, file_sha256)?;
        let lock = self.lock_file(tenant_id, user_id, file_sha256).await?;
        let size = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        put_verified(blobs, &key, file_sha256, FIT_MEDIA_TYPE, bytes).await?;
        self.files.fence_upload_name(&lock).await?;
        let recorded = self
            .files
            .record_uploaded_file(&UploadedFileWrite {
                tenant_id,
                user_id,
                file_sha256,
                byte_size: size,
                origin: UPLOAD_ORIGIN,
                record_base: Some(&record_base),
                blob_keyed: true,
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
    /// (`feedback_messages`), a lab report (`lab_documents`) — under its
    /// keyed name. The caller writes that row once this returns, still
    /// holding the lock, with `blob_keyed` set.
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
        let key = self.keyed_name(lock.tenant_id(), lock.user_id(), lock.file_sha256())?;
        put_verified(self.blobs()?, &key, lock.file_sha256(), media_type, bytes).await?;
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

    /// Copy one page of the files `holder` names under their content name
    /// to their keyed name, after `after`, in every scope: the keyed-name
    /// backfill the `record_log` worker runs (carnet#1701). It never deletes:
    /// the content name stays until carnet#1550.
    ///
    /// A file whose copy fails is counted unreadable and logged, so one file
    /// never holds the sweep.
    ///
    /// # Errors
    ///
    /// [`AppError::resource_unavailable`] when no blob store is placed here;
    /// a database error when the page cannot be read.
    pub async fn rename_uploads_page(
        &self,
        holder: UploadHolder,
        after: Option<&ContentNamedFile>,
        limit: u32,
    ) -> AppResult<UploadRenamePage> {
        self.blobs()?;
        let files = self.files.content_named_files(holder, after, limit).await?;
        let mut page = UploadRenamePage::default();
        for file in &files {
            match self.copy_to_keyed_name(holder, file).await {
                Ok(KeyedCopy::Copied) => page.copied += 1,
                Ok(KeyedCopy::Gone) => {}
                Ok(KeyedCopy::Unreadable) => page.unreadable += 1,
                Err(e) => {
                    page.unreadable += 1;
                    warn!(
                        table = holder.table(),
                        tenant_id = %file.tenant_id,
                        user_id = %file.user_id,
                        error = %e,
                        "uploaded file not copied to its keyed name"
                    );
                }
            }
        }
        if u32::try_from(files.len()).is_ok_and(|read| read >= limit) {
            page.next = files.last().cloned();
        }
        Ok(page)
    }

    /// Copy the content-named file `file` that `holder` names to its keyed
    /// name, under the file's lease: only while a row still names it
    /// unrenamed, so a copy never outlives a delete. The bytes are checked
    /// against the digest before the copy, and every row naming the file is
    /// marked once its lease is fenced.
    async fn copy_to_keyed_name(
        &self,
        holder: UploadHolder,
        file: &ContentNamedFile,
    ) -> AppResult<KeyedCopy> {
        let (tenant_id, user_id, sha) = (&file.tenant_id, file.user_id, file.file_sha256.as_str());
        let blobs = self.blobs()?;
        let key = self.keyed_name(tenant_id, user_id, sha)?;
        let lock = self.lock_file(tenant_id, user_id, sha).await?;
        if !self
            .files
            .names_content_named(holder, tenant_id, user_id, sha)
            .await?
        {
            return Ok(KeyedCopy::Gone);
        }
        let Some(bytes) = self.read_file(tenant_id, user_id, sha).await? else {
            return Ok(KeyedCopy::Unreadable);
        };
        let media_type = match (holder, file.media_type.as_deref()) {
            (_, Some(media_type)) => media_type,
            (UploadHolder::ActivityFiles, None) => FIT_MEDIA_TYPE,
            (UploadHolder::LabDocuments, None) => LAB_DOCUMENT_MEDIA_TYPE,
            // A screenshot row records its type with its digest; one that
            // lost it is copied as bytes of no stated type.
            (UploadHolder::FeedbackImages, None) => UNTYPED_MEDIA_TYPE,
        };
        put_verified(blobs, &key, sha, media_type, bytes).await?;
        self.files.fence_upload_name(&lock).await?;
        self.files
            .mark_blob_keyed(holder, tenant_id, user_id, sha)
            .await?;
        drop(lock);
        Ok(KeyedCopy::Copied)
    }

    /// How many files `holder` names that are not kept under their keyed
    /// name yet, in every scope: what the backfill's done signal compares.
    ///
    /// # Errors
    ///
    /// A database error when the count fails.
    pub async fn count_content_named(&self, holder: UploadHolder) -> AppResult<u64> {
        self.files.count_content_named(holder).await
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

/// Store `bytes` as `media_type` under `key`, a name of the upload
/// `file_sha256`, and refuse unless the blob store reports it stored exactly
/// those bytes: the digest `file_sha256` names and their length. A write
/// that stored anything else is removed.
async fn put_verified(
    blobs: &Arc<dyn BlobStore>,
    key: &BlobKey,
    file_sha256: &str,
    media_type: &str,
    bytes: Bytes,
) -> AppResult<()> {
    let size = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    let stat = blobs
        .put(key, bytes, &BlobMeta::new(media_type))
        .await
        .map_err(|e| blob_error(&e))?;
    if stat.sha256.to_hex() == file_sha256 && stat.size == size {
        return Ok(());
    }
    // The bytes are not the ones the hash names: never leave them under it.
    blobs.delete(key).await.map_err(|e| blob_error(&e))?;
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
