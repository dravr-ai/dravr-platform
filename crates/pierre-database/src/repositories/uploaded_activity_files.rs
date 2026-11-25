// ABOUTME: UploadedActivityFileRepository trait plus the one shared implementation both backends emit
// ABOUTME: What names each .fit file an athlete uploaded — owner, hash, size, origin; its bytes live in the blob store
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Uploaded activity files.
//!
//! An athlete can upload the `.fit` file of a completed workout (carnet#818).
//! Each session in it becomes a cached activity under the provider key
//! `upload`. The file itself is kept in the blob store by the originals store
//! (`pierre_providers::originals`); a row here names it: its owner, the
//! SHA-256 of its bytes, its size, its origin and when it came in. A file is
//! addressed by its hash, so the same file uploaded twice is one row, and the
//! row is what scopes a read of the file to the athlete who uploaded it.
//!
//! The table holds no bytes: the column that kept them before the blob store
//! did was drained into it and dropped (Athlete Data Consolidation, S12;
//! carnet#1309).
//!
//! Every statement is scoped to the tenant and the user. `$n` placeholders
//! throughout: sqlx accepts them on `SQLite` as well as Postgres, and every
//! id column is `TEXT` on both engines.
//!
//! A file is kept under its keyed name (`blob_keyed`) or, uploaded before
//! that name was written, under its content name (carnet#1701). Its
//! activities' ids carry its digest (`<sha>-<index>`) or, for a file whose
//! row has a `record_base`, that keyed base (`upload-k<hex16>-<index>`).
//!
//! The writes, deletes and the keyed-name copy of one file run under one
//! lease per athlete and file ([`UploadedActivityFileRepository::lock_upload_name`]),
//! a row in `upload_name_leases`, so neither can interleave with the other.
//! A claim, its fence and its release are each one short statement: no
//! connection is held while the holder writes the blob store.

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use dravr_tronc::dates::Timestamp;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use tokio::runtime::Handle;
use tokio::time::{self, Instant};
use tracing::warn;
use uuid::Uuid;

/// The origin of a file the athlete uploaded from the app.
pub const UPLOAD_ORIGIN: &str = "upload";

/// The metadata of the activity files athletes uploaded.
#[async_trait]
pub trait UploadedActivityFileRepository: Send + Sync {
    /// Record one uploaded file whose bytes the blob store now holds. A file
    /// the user already holds in this tenant (the same hash) keeps its row,
    /// gaining `record_base` when it had none and `blob_keyed` once its
    /// keyed copy is written: a file kept before keeps answering its old
    /// ids, and answers the keyed ones too.
    ///
    /// Returns `true` when the row is new, `false` when it was already held.
    /// The caller holds the file's lock ([`Self::lock_upload_name`]), so the
    /// two cannot be told apart wrongly by a concurrent write.
    ///
    /// # Errors
    /// Returns a database error when the write fails.
    async fn record_uploaded_file(&self, file: &UploadedFileWrite<'_>) -> AppResult<bool>;

    /// The digest of the file whose activities' keyed base is `record_base`
    /// (`upload-k<hex16>`), when the user holds it in this tenant.
    ///
    /// # Errors
    /// Returns a database error when the read fails.
    async fn file_of_record_base(
        &self,
        tenant_id: &TenantId,
        user_id: Uuid,
        record_base: &str,
    ) -> AppResult<Option<String>>;

    /// Wait for, then hold, the lease on the user's file `file_sha256` in
    /// this tenant, until the returned guard is dropped.
    ///
    /// One lease serializes every write, delete and keyed-name copy of one
    /// file, whichever table names it: an uploaded activity file, a feedback
    /// screenshot or a lab report share the upload key (carnet#1701). It is
    /// claimed when free or lapsed ([`UPLOAD_NAME_LEASE`]), retried until
    /// [`UPLOAD_NAME_LEASE_WAIT`], and released when the guard drops.
    ///
    /// # Errors
    /// [`AppError::resource_unavailable`] when another holder keeps it past
    /// the wait; a database error when a claim fails.
    async fn lock_upload_name(
        &self,
        tenant_id: &TenantId,
        user_id: Uuid,
        file_sha256: &str,
    ) -> AppResult<UploadNameLock>;

    /// Check that `lock` still holds its lease and extend it: the fence a
    /// holder passes before writing the row naming the file, so a holder
    /// whose lease lapsed and was taken over writes nothing.
    ///
    /// # Errors
    /// [`AppError::resource_unavailable`] when the lease lapsed and another
    /// claim took it; a database error when the renewal fails.
    async fn fence_upload_name(&self, lock: &UploadNameLock) -> AppResult<()>;

    /// Whether the user holds the file `file_sha256` in this tenant.
    ///
    /// # Errors
    /// Returns a database error when the read fails.
    async fn holds_uploaded_file(
        &self,
        tenant_id: &TenantId,
        user_id: Uuid,
        file_sha256: &str,
    ) -> AppResult<bool>;

    /// Whether any activity the file `file_sha256` became is still cached for
    /// the user in this tenant: an `upload` row whose id is the hash or the
    /// file row's keyed base followed by a session index (`<sha>-<index>`,
    /// `upload-k<hex16>-<index>`).
    ///
    /// Read from the ids the cache already holds, so the stored file is not
    /// decoded again to learn how many sessions it named.
    ///
    /// # Errors
    /// Returns a database error when the read fails.
    async fn file_backs_cached_activity(
        &self,
        tenant_id: &TenantId,
        user_id: Uuid,
        file_sha256: &str,
    ) -> AppResult<bool>;

    /// Delete one uploaded file's row, once the athlete has deleted every
    /// activity it became and its bytes are gone from the blob store.
    ///
    /// Returns `true` when a row was removed, `false` when the user held no
    /// file with that hash in this tenant.
    ///
    /// # Errors
    /// Returns a database error when the delete fails.
    async fn delete_uploaded_file(
        &self,
        tenant_id: &TenantId,
        user_id: Uuid,
        file_sha256: &str,
    ) -> AppResult<bool>;

    /// One page of the files `holder` names that are not kept under their
    /// keyed name yet (`blob_keyed = FALSE`), in every scope, after `after`,
    /// in `(tenant_id, user_id, digest)` order: what the keyed-name backfill
    /// copies (carnet#1701). A digest several rows of one athlete name is
    /// one file.
    ///
    /// # Errors
    /// Returns a database error when the read fails, or a row's ids do not
    /// parse.
    async fn content_named_files(
        &self,
        holder: UploadHolder,
        after: Option<&ContentNamedFile>,
        limit: u32,
    ) -> AppResult<Vec<ContentNamedFile>>;

    /// How many files `holder` names that are not kept under their keyed
    /// name yet, in every scope.
    ///
    /// # Errors
    /// Returns a database error when the count fails.
    async fn count_content_named(&self, holder: UploadHolder) -> AppResult<u64>;

    /// Whether a row of `holder` still names the athlete's file
    /// `file_sha256` in this tenant and says it is not kept under its keyed
    /// name: what the backfill checks under the file's lease before it
    /// copies.
    ///
    /// # Errors
    /// Returns a database error when the read fails.
    async fn names_content_named(
        &self,
        holder: UploadHolder,
        tenant_id: &TenantId,
        user_id: Uuid,
        file_sha256: &str,
    ) -> AppResult<bool>;

    /// Mark every row of `holder` naming the athlete's file `file_sha256` in
    /// this tenant as kept under its keyed name. Returns how many rows it
    /// marked.
    ///
    /// # Errors
    /// Returns a database error when the update fails.
    async fn mark_blob_keyed(
        &self,
        holder: UploadHolder,
        tenant_id: &TenantId,
        user_id: Uuid,
        file_sha256: &str,
    ) -> AppResult<u64>;
}

/// A table whose rows name a file the athlete uploaded by its digest, kept
/// under the upload key ([`UploadedActivityFileRepository::content_named_files`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UploadHolder {
    /// `uploaded_activity_files`: an uploaded activity file.
    ActivityFiles,
    /// `feedback_messages`: a screenshot attached to a feedback message.
    FeedbackImages,
    /// `lab_documents`: a lab report's PDF.
    LabDocuments,
}

impl UploadHolder {
    /// Every holder, in the order the backfill sweeps them.
    pub const ALL: [Self; 3] = [
        Self::ActivityFiles,
        Self::FeedbackImages,
        Self::LabDocuments,
    ];

    /// The holder's table.
    #[must_use]
    pub const fn table(self) -> &'static str {
        match self {
            Self::ActivityFiles => "uploaded_activity_files",
            Self::FeedbackImages => "feedback_messages",
            Self::LabDocuments => "lab_documents",
        }
    }

    /// The statements over the holder's table.
    pub(crate) const fn sql(self) -> &'static HolderSql {
        match self {
            Self::ActivityFiles => &ACTIVITY_FILES_SQL,
            Self::FeedbackImages => &FEEDBACK_IMAGES_SQL,
            Self::LabDocuments => &LAB_DOCUMENTS_SQL,
        }
    }

    /// Whether the table keeps its owner ids as `TEXT` on both engines
    /// (`uploaded_activity_files`); the others are `uuid` on Postgres.
    pub(crate) const fn text_ids(self) -> bool {
        matches!(self, Self::ActivityFiles)
    }
}

/// A file a holder table names that is not kept under its keyed name yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentNamedFile {
    /// The tenant it was uploaded in.
    pub tenant_id: TenantId,
    /// The athlete who uploaded it.
    pub user_id: Uuid,
    /// Its SHA-256, lowercase hex.
    pub file_sha256: String,
    /// The media type its row records, when the table records one
    /// (`feedback_messages`).
    pub media_type: Option<String>,
}

/// One uploaded file to record.
#[derive(Debug, Clone, Copy)]
pub struct UploadedFileWrite<'file> {
    /// The tenant it was uploaded in.
    pub tenant_id: &'file TenantId,
    /// The athlete who uploaded it.
    pub user_id: Uuid,
    /// The SHA-256 of its bytes.
    pub file_sha256: &'file str,
    /// Its size in bytes.
    pub byte_size: u64,
    /// Where it came from ([`UPLOAD_ORIGIN`]).
    pub origin: &'file str,
    /// The keyed base its activities' ids carry, or `None` for ids that
    /// carry its digest.
    pub record_base: Option<&'file str>,
    /// Whether its keyed copy is written.
    pub blob_keyed: bool,
}

/// The release of a lease, run once its lock is dropped.
pub(crate) type LeaseRelease = Pin<Box<dyn Future<Output = ()> + Send>>;

/// How long a claimed lease holds a file before another claim may take it
/// over: past the longest blob write of one file, so only a holder that
/// died lets it lapse.
pub const UPLOAD_NAME_LEASE: Duration = Duration::from_secs(120);

/// How long a claim waits for a held lease before giving up.
pub const UPLOAD_NAME_LEASE_WAIT: Duration = Duration::from_secs(150);

/// The first pause between two claims of a held lease; it doubles up to
/// [`UPLOAD_NAME_LEASE_MAX_PAUSE`].
const UPLOAD_NAME_LEASE_PAUSE: Duration = Duration::from_millis(25);

/// The longest pause between two claims.
const UPLOAD_NAME_LEASE_MAX_PAUSE: Duration = Duration::from_millis(500);

/// A held lease on one user's file ([`UploadedActivityFileRepository::lock_upload_name`]),
/// released when dropped. It names the file it holds.
pub struct UploadNameLock {
    tenant_id: TenantId,
    user_id: Uuid,
    file_sha256: String,
    token: String,
    /// Deletes the lease row this lock's token holds; spawned on drop. In a
    /// mutex so the lock is `Sync` while held across an await.
    release: Mutex<Option<LeaseRelease>>,
}

impl fmt::Debug for UploadNameLock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UploadNameLock")
            .field("tenant_id", &self.tenant_id)
            .field("user_id", &self.user_id)
            .field("file_sha256", &self.file_sha256)
            .finish_non_exhaustive()
    }
}

impl Drop for UploadNameLock {
    fn drop(&mut self) {
        let release = self
            .release
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        let Some(release) = release else {
            return;
        };
        // Outside a runtime the lease is left to lapse at its expiry.
        if let Ok(runtime) = Handle::try_current() {
            runtime.spawn(release);
        }
    }
}

impl UploadNameLock {
    /// The tenant the locked file was uploaded in.
    #[must_use]
    pub const fn tenant_id(&self) -> &TenantId {
        &self.tenant_id
    }

    /// The athlete who uploaded it.
    #[must_use]
    pub const fn user_id(&self) -> Uuid {
        self.user_id
    }

    /// Its digest.
    #[must_use]
    pub fn file_sha256(&self) -> &str {
        &self.file_sha256
    }

    /// The lease `token` holds on `file_sha256`, released by `release` when
    /// dropped.
    pub(crate) fn new(
        tenant_id: &TenantId,
        user_id: Uuid,
        file_sha256: &str,
        token: String,
        release: LeaseRelease,
    ) -> Self {
        Self {
            tenant_id: *tenant_id,
            user_id,
            file_sha256: file_sha256.to_owned(),
            token,
            release: Mutex::new(Some(release)),
        }
    }

    /// The token of the claim holding it.
    pub(crate) fn token(&self) -> &str {
        &self.token
    }
}

/// When a lease claimed or renewed now lapses, in epoch milliseconds.
pub(crate) fn lease_until(now: Timestamp) -> i64 {
    let lease = i64::try_from(UPLOAD_NAME_LEASE.as_millis()).unwrap_or(i64::MAX);
    now.epoch_millis().saturating_add(lease)
}

/// Claim one lease until it is taken or [`UPLOAD_NAME_LEASE_WAIT`] passes.
/// `claim` makes one attempt with a fresh token and answers whether it took
/// the lease.
pub(crate) async fn claim_upload_name<F, Fut>(file_sha256: &str, mut claim: F) -> AppResult<String>
where
    F: FnMut(String) -> Fut,
    Fut: Future<Output = AppResult<bool>>,
{
    let deadline = Instant::now() + UPLOAD_NAME_LEASE_WAIT;
    let mut pause = UPLOAD_NAME_LEASE_PAUSE;
    loop {
        let token = Uuid::new_v4().to_string();
        if claim(token.clone()).await? {
            return Ok(token);
        }
        if Instant::now() >= deadline {
            return Err(AppError::resource_unavailable(format!(
                "uploaded file {file_sha256} is held by another write"
            )));
        }
        time::sleep(pause).await;
        pause = (pause * 2).min(UPLOAD_NAME_LEASE_MAX_PAUSE);
    }
}

/// Log a release that failed: its lease lapses at its expiry instead.
pub(crate) fn release_failed(error: &sqlx::Error) {
    warn!(error = %error, "an upload-name lease was not released; it lapses at its expiry");
}

/// Take the lease on a file when no one holds it, or its holder's lapsed.
pub(crate) const CLAIM_UPLOAD_NAME_SQL: &str = r"
    INSERT INTO upload_name_leases (tenant_id, user_id, file_sha256, lease_token, leased_until_ms)
    VALUES ($1, $2, $3, $4, $5)
    ON CONFLICT (tenant_id, user_id, file_sha256) DO UPDATE SET
        lease_token = excluded.lease_token,
        leased_until_ms = excluded.leased_until_ms
    WHERE upload_name_leases.leased_until_ms <= $6";

/// Extend a lease its token still holds.
pub(crate) const FENCE_UPLOAD_NAME_SQL: &str = r"
    UPDATE upload_name_leases SET leased_until_ms = $5
    WHERE tenant_id = $1 AND user_id = $2 AND file_sha256 = $3 AND lease_token = $4";

/// Release a lease its token still holds.
pub(crate) const RELEASE_UPLOAD_NAME_SQL: &str = r"
    DELETE FROM upload_name_leases
    WHERE tenant_id = $1 AND user_id = $2 AND file_sha256 = $3 AND lease_token = $4";

/// Record a file. A second upload of the same bytes keeps the row, filling
/// its keyed base when it had none and its keyed copy once written.
pub(crate) const RECORD_UPLOADED_FILE_SQL: &str = r"
    INSERT INTO uploaded_activity_files (
        tenant_id, user_id, file_sha256, byte_size, origin, uploaded_at, record_base, blob_keyed
    )
    VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
    ON CONFLICT (tenant_id, user_id, file_sha256) DO UPDATE SET
        record_base = COALESCE(uploaded_activity_files.record_base, excluded.record_base),
        blob_keyed = uploaded_activity_files.blob_keyed OR excluded.blob_keyed";

/// The file whose keyed base is `$3`.
pub(crate) const FILE_OF_RECORD_BASE_SQL: &str = r"
    SELECT file_sha256
    FROM uploaded_activity_files
    WHERE tenant_id = $1 AND user_id = $2 AND record_base = $3";

/// The keyed base of a held file, when it has one.
pub(crate) const RECORD_BASE_OF_FILE_SQL: &str = r"
    SELECT record_base
    FROM uploaded_activity_files
    WHERE tenant_id = $1 AND user_id = $2 AND file_sha256 = $3";

/// Whether the owner holds the file.
pub(crate) const HOLDS_UPLOADED_FILE_SQL: &str = r"
    SELECT 1 AS held
    FROM uploaded_activity_files
    WHERE tenant_id = $1 AND user_id = $2 AND file_sha256 = $3";

/// Whether any cached `upload` activity of the owner's carries an id
/// starting with `$3`, a LIKE prefix whose wildcards are escaped. `SQLite`'s
/// LIKE ignores ASCII case, which changes nothing here: every id is a
/// lowercase hex hash, a dash and a decimal index.
pub(crate) const FILE_BACKS_CACHED_ACTIVITY_SQL: &str = r"
    SELECT 1 AS held
    FROM cached_activities
    WHERE user_id = $1 AND tenant_id = $2 AND provider = 'upload'
      AND activity_id LIKE $3 ESCAPE '\'
    LIMIT 1";

/// The LIKE pattern matching every activity id whose base is `base`: a
/// file's digest, or its keyed base.
pub(crate) fn file_activity_id_pattern(base: &str) -> String {
    let mut pattern = String::with_capacity(base.len() + 2);
    for c in base.chars() {
        if matches!(c, '\\' | '%' | '_') {
            pattern.push('\\');
        }
        pattern.push(c);
    }
    pattern.push_str("-%");
    pattern
}

/// The statements the keyed-name backfill runs over one holder table.
#[derive(Debug)]
pub(crate) struct HolderSql {
    /// The first page of content-named files, in every scope.
    pub(crate) first_page: &'static str,
    /// A page after the cursor `($2, $3, $4)`, in every scope.
    pub(crate) next_page: &'static str,
    /// How many content-named files, in every scope.
    pub(crate) count: &'static str,
    /// Whether a content-named row names the file, in its scope.
    pub(crate) names: &'static str,
    /// Mark every row naming the file, in its scope.
    pub(crate) mark: &'static str,
}

/// The [`HolderSql`] of `$table`, whose digest column is `$digest` and
/// whose media type is `$media`.
///
/// ISOLATION EXEMPTION: `first_page`, `next_page` and `count` have no
/// `tenant_id` or `user_id` filter, on purpose. They serve only the
/// `record_log` worker's keyed-name backfill, which copies every athlete's
/// files (carnet#1701); every write it makes (`names`, `mark`) filters on the
/// tenant and the user. No tenant- or user-facing path may run them. The
/// owner ids are compared and read as text, so the order is one on both
/// engines.
macro_rules! holder_sql {
    ($table:literal, $digest:literal, $media:literal) => {
        HolderSql {
            first_page: concat!(
                "SELECT CAST(tenant_id AS TEXT) AS tenant_text, CAST(user_id AS TEXT) AS user_text, ",
                $digest, " AS file_sha256, ", $media, " AS media_type FROM ", $table,
                " WHERE blob_keyed = FALSE AND ", $digest, " IS NOT NULL",
                " GROUP BY tenant_id, user_id, ", $digest,
                " ORDER BY tenant_text, user_text, file_sha256 LIMIT $1"
            ),
            next_page: concat!(
                "SELECT CAST(tenant_id AS TEXT) AS tenant_text, CAST(user_id AS TEXT) AS user_text, ",
                $digest, " AS file_sha256, ", $media, " AS media_type FROM ", $table,
                " WHERE blob_keyed = FALSE AND ", $digest, " IS NOT NULL",
                " AND (CAST(tenant_id AS TEXT), CAST(user_id AS TEXT), ", $digest, ") > ($2, $3, $4)",
                " GROUP BY tenant_id, user_id, ", $digest,
                " ORDER BY tenant_text, user_text, file_sha256 LIMIT $1"
            ),
            count: concat!(
                "SELECT COUNT(*) FROM (SELECT 1 AS one FROM ", $table,
                " WHERE blob_keyed = FALSE AND ", $digest, " IS NOT NULL",
                " GROUP BY tenant_id, user_id, ", $digest, ") AS files"
            ),
            names: concat!(
                "SELECT 1 AS held FROM ", $table,
                " WHERE tenant_id = $1 AND user_id = $2 AND ", $digest, " = $3",
                " AND blob_keyed = FALSE LIMIT 1"
            ),
            mark: concat!(
                "UPDATE ", $table, " SET blob_keyed = TRUE",
                " WHERE tenant_id = $1 AND user_id = $2 AND ", $digest, " = $3"
            ),
        }
    };
}

/// The backfill's statements over `uploaded_activity_files`.
const ACTIVITY_FILES_SQL: HolderSql = holder_sql!(
    "uploaded_activity_files",
    "file_sha256",
    "CAST(NULL AS TEXT)"
);

/// The backfill's statements over `feedback_messages`: one athlete may
/// attach the same screenshot to several messages.
const FEEDBACK_IMAGES_SQL: HolderSql =
    holder_sql!("feedback_messages", "image_sha256", "MIN(image_media_type)");

/// The backfill's statements over `lab_documents`.
const LAB_DOCUMENTS_SQL: HolderSql =
    holder_sql!("lab_documents", "file_sha256", "CAST(NULL AS TEXT)");

/// One content-named file of a page, from its text columns.
///
/// # Errors
/// Returns a database error when an owner id does not parse.
pub(crate) fn content_named_file(
    (tenant, user, file_sha256, media_type): (String, String, String, Option<String>),
) -> AppResult<ContentNamedFile> {
    let tenant_id = TenantId::parse_str(&tenant)
        .map_err(|e| AppError::database(format!("content-named file tenant {tenant}: {e}")))?;
    let user_id = Uuid::parse_str(&user)
        .map_err(|e| AppError::database(format!("content-named file user {user}: {e}")))?;
    Ok(ContentNamedFile {
        tenant_id,
        user_id,
        file_sha256,
        media_type,
    })
}

/// Delete one file's row, by its owner and hash.
pub(crate) const DELETE_UPLOADED_FILE_SQL: &str = r"
    DELETE FROM uploaded_activity_files
    WHERE tenant_id = $1 AND user_id = $2 AND file_sha256 = $3";

/// Emit the whole [`UploadedActivityFileRepository`] implementation for one
/// backend type. The body is written once here; each backend's shell invokes
/// it with its own type, and sqlx resolves the driver from `self.pool()` per
/// expansion. The body names its consts and types unqualified, so the
/// invoking shell must `use` every one of them.
macro_rules! impl_uploaded_activity_file_repository {
    ($ty:ty, $ids:ident) => {
        #[async_trait::async_trait]
        impl UploadedActivityFileRepository for $ty {
            async fn record_uploaded_file(&self, file: &UploadedFileWrite<'_>) -> AppResult<bool> {
                let held = self
                    .holds_uploaded_file(file.tenant_id, file.user_id, file.file_sha256)
                    .await?;
                sqlx::query(RECORD_UPLOADED_FILE_SQL)
                    .bind(file.tenant_id.to_string())
                    .bind(file.user_id.to_string())
                    .bind(file.file_sha256)
                    .bind(i64::try_from(file.byte_size).unwrap_or(i64::MAX))
                    .bind(file.origin)
                    .bind(Timestamp::now())
                    .bind(file.record_base)
                    .bind(file.blob_keyed)
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("record_uploaded_file: {e}")))?;
                Ok(!held)
            }

            async fn file_of_record_base(
                &self,
                tenant_id: &TenantId,
                user_id: Uuid,
                record_base: &str,
            ) -> AppResult<Option<String>> {
                sqlx::query_scalar(FILE_OF_RECORD_BASE_SQL)
                    .bind(tenant_id.to_string())
                    .bind(user_id.to_string())
                    .bind(record_base)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("file_of_record_base: {e}")))
            }

            async fn lock_upload_name(
                &self,
                tenant_id: &TenantId,
                user_id: Uuid,
                file_sha256: &str,
            ) -> AppResult<UploadNameLock> {
                let failed = |e: sqlx::Error| AppError::database(format!("lock_upload_name: {e}"));
                let token = claim_upload_name(file_sha256, |token| async move {
                    let now = Timestamp::now();
                    let claimed = sqlx::query(CLAIM_UPLOAD_NAME_SQL)
                        .bind(tenant_id.to_string())
                        .bind(user_id.to_string())
                        .bind(file_sha256)
                        .bind(token)
                        .bind(lease_until(now))
                        .bind(now.epoch_millis())
                        .execute(self.pool())
                        .await
                        .map_err(failed)?;
                    Ok(claimed.rows_affected() > 0)
                })
                .await?;
                let pool = self.pool().clone();
                let key = (
                    tenant_id.to_string(),
                    user_id.to_string(),
                    file_sha256.to_owned(),
                );
                let held = token.clone();
                let release: LeaseRelease = Box::pin(async move {
                    let released = sqlx::query(RELEASE_UPLOAD_NAME_SQL)
                        .bind(key.0)
                        .bind(key.1)
                        .bind(key.2)
                        .bind(held)
                        .execute(&pool)
                        .await;
                    if let Err(e) = released {
                        release_failed(&e);
                    }
                });
                Ok(UploadNameLock::new(
                    tenant_id,
                    user_id,
                    file_sha256,
                    token,
                    release,
                ))
            }

            async fn fence_upload_name(&self, lock: &UploadNameLock) -> AppResult<()> {
                let renewed = sqlx::query(FENCE_UPLOAD_NAME_SQL)
                    .bind(lock.tenant_id().to_string())
                    .bind(lock.user_id().to_string())
                    .bind(lock.file_sha256())
                    .bind(lock.token())
                    .bind(lease_until(Timestamp::now()))
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("fence_upload_name: {e}")))?;
                if renewed.rows_affected() == 0 {
                    return Err(AppError::resource_unavailable(format!(
                        "the lease on uploaded file {} lapsed and was taken over",
                        lock.file_sha256()
                    )));
                }
                Ok(())
            }

            async fn holds_uploaded_file(
                &self,
                tenant_id: &TenantId,
                user_id: Uuid,
                file_sha256: &str,
            ) -> AppResult<bool> {
                let row = sqlx::query(HOLDS_UPLOADED_FILE_SQL)
                    .bind(tenant_id.to_string())
                    .bind(user_id.to_string())
                    .bind(file_sha256)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("holds_uploaded_file: {e}")))?;
                Ok(row.is_some())
            }

            async fn file_backs_cached_activity(
                &self,
                tenant_id: &TenantId,
                user_id: Uuid,
                file_sha256: &str,
            ) -> AppResult<bool> {
                let failed =
                    |e: sqlx::Error| AppError::database(format!("file_backs_cached_activity: {e}"));
                let record_base: Option<Option<String>> =
                    sqlx::query_scalar(RECORD_BASE_OF_FILE_SQL)
                        .bind(tenant_id.to_string())
                        .bind(user_id.to_string())
                        .bind(file_sha256)
                        .fetch_optional(self.pool())
                        .await
                        .map_err(failed)?;
                let record_base = record_base.flatten();
                let bases = iter::once(file_sha256).chain(record_base.as_deref());
                for base in bases {
                    let row = sqlx::query(FILE_BACKS_CACHED_ACTIVITY_SQL)
                        .bind(user_id.to_string())
                        .bind(tenant_id.to_string())
                        .bind(file_activity_id_pattern(base))
                        .fetch_optional(self.pool())
                        .await
                        .map_err(failed)?;
                    if row.is_some() {
                        return Ok(true);
                    }
                }
                Ok(false)
            }

            async fn delete_uploaded_file(
                &self,
                tenant_id: &TenantId,
                user_id: Uuid,
                file_sha256: &str,
            ) -> AppResult<bool> {
                let deleted = sqlx::query(DELETE_UPLOADED_FILE_SQL)
                    .bind(tenant_id.to_string())
                    .bind(user_id.to_string())
                    .bind(file_sha256)
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("delete_uploaded_file: {e}")))?;
                Ok(deleted.rows_affected() > 0)
            }

            async fn content_named_files(
                &self,
                holder: UploadHolder,
                after: Option<&ContentNamedFile>,
                limit: u32,
            ) -> AppResult<Vec<ContentNamedFile>> {
                let sql = holder.sql();
                let query = match after {
                    None => sqlx::query_as(sql.first_page).bind(i64::from(limit)),
                    Some(cursor) => sqlx::query_as(sql.next_page)
                        .bind(i64::from(limit))
                        .bind(cursor.tenant_id.to_string())
                        .bind(cursor.user_id.to_string())
                        .bind(cursor.file_sha256.as_str()),
                };
                let rows: Vec<(String, String, String, Option<String>)> = query
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("content_named_files: {e}")))?;
                rows.into_iter().map(content_named_file).collect()
            }

            async fn count_content_named(&self, holder: UploadHolder) -> AppResult<u64> {
                let count: i64 = sqlx::query_scalar(holder.sql().count)
                    .fetch_one(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("count_content_named: {e}")))?;
                Ok(u64::try_from(count).unwrap_or(0))
            }

            async fn names_content_named(
                &self,
                holder: UploadHolder,
                tenant_id: &TenantId,
                user_id: Uuid,
                file_sha256: &str,
            ) -> AppResult<bool> {
                let query = sqlx::query(holder.sql().names);
                let query = if holder.text_ids() {
                    query.bind(tenant_id.to_string()).bind(user_id.to_string())
                } else {
                    query
                        .bind($ids::bind(tenant_id.as_uuid()))
                        .bind($ids::bind(user_id))
                };
                let row = query
                    .bind(file_sha256)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("names_content_named: {e}")))?;
                Ok(row.is_some())
            }

            async fn mark_blob_keyed(
                &self,
                holder: UploadHolder,
                tenant_id: &TenantId,
                user_id: Uuid,
                file_sha256: &str,
            ) -> AppResult<u64> {
                let query = sqlx::query(holder.sql().mark);
                let query = if holder.text_ids() {
                    query.bind(tenant_id.to_string()).bind(user_id.to_string())
                } else {
                    query
                        .bind($ids::bind(tenant_id.as_uuid()))
                        .bind($ids::bind(user_id))
                };
                let marked = query
                    .bind(file_sha256)
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("mark_blob_keyed: {e}")))?;
                Ok(marked.rows_affected())
            }
        }
    };
}
pub(crate) use impl_uploaded_activity_file_repository;

#[cfg(test)]
mod tests {
    use super::file_activity_id_pattern;

    #[test]
    fn the_pattern_is_the_hash_a_dash_and_any_index() {
        assert_eq!(file_activity_id_pattern("ab12"), "ab12-%");
    }

    #[test]
    fn like_wildcards_in_the_hash_are_matched_literally() {
        assert_eq!(file_activity_id_pattern("a_b%c\\d"), "a\\_b\\%c\\\\d-%");
    }
}
