// ABOUTME: OriginalsBundleService — builds the athlete's originals bundle in the background, tracks it in a durable row
// ABOUTME: Hands it out as a 15-minute signed URL or a stream; an hourly sweep fails stalled builds and deletes expired bundles

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # The originals bundle
//!
//! `GET /api/user/export` is the athlete's one Law 25 / GDPR Art. 20 export.
//! Its JSON leaves out the bytes of the files Dravr keeps for them, which are
//! unbounded; this service builds those into one ZIP on request
//! (`dravr_barrage::export::write_bundle`) and the export names the latest:
//!
//! - **what goes in**: every original an archive record of the athlete keeps
//!   in each tenant their rows reach (never a live record's, so no
//!   provider-API data), every recording they uploaded
//!   (`uploaded_activity_files`) and every screenshot they attached to
//!   feedback (`feedback_messages.image_sha256`), read through the export's
//!   own table walk;
//! - **where it is tracked**: a `user_export_bundles` row, `building` while a
//!   task writes it (its heartbeat moving forward), `ready` for
//!   seven days, or `failed`. A second unsealed request while an
//!   unsealed one builds gets that one, any other is a conflict; an unsealed
//!   bundle ready for less than an hour is handed back rather than built
//!   again. Only a request that starts a build is metered
//!   ([`OriginalsBundleService::admit`]);
//! - **how it is downloaded**: a [`DOWNLOAD_TTL`] signed URL on GCS, or the
//!   bytes streamed through the API from a store that signs nothing;
//! - **sealing**: the athlete may give a passphrase; the bundle is then
//!   encrypted as it streams ([`crate::originals_seal`]) and the passphrase is
//!   never kept;
//! - **cleanup**: an hourly sweep ([`start_originals_bundle_sweep`]) fails a
//!   build whose heartbeat stopped (its instance died) and deletes its blob,
//!   then deletes every expired bundle's blob and row. The blob also carries a
//!   lifecycle expiry, so the bucket deletes it even if no sweep runs; an
//!   account's deletion or reset takes every blob under the athlete.

use std::collections::HashSet;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use dravr_barrage::blob::{BlobKey, BlobStore, DownloadTarget};
use dravr_barrage::export::{
    self, BundleFile, BundleFileKind, BundleOwner, BundleReport, BundleSeal,
};
use dravr_barrage::store::ActivityRecordStore;
use dravr_barrage::{BarrageError, Scope, UserId};
use futures_util::stream::{self, BoxStream};
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::TenantId;
use pierre_database::repositories::{
    BundleContents, BundleStatus, ExportBundle, UserExportTable, USER_EXPORT_TABLES,
};
use pierre_database::RepositoryRegistry;
use pierre_providers::originals;
use serde::Serialize;
use serde_json::Value;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::task::AbortHandle;
use tokio::time;
use tokio_util::bytes::Bytes;
use tracing::{info, warn};
use uuid::Uuid;

use crate::originals_seal::ExportSeal;
use crate::periodic::spawn_periodic;

/// How many bundles this instance writes at once.
const MAX_CONCURRENT_BUNDLES: usize = 2;

/// How long a ready bundle stays downloadable.
const BUNDLE_TTL: TimeDelta = TimeDelta::days(7);

/// How long an unsealed ready bundle is handed back rather than built again.
const REUSE_WINDOW: TimeDelta = TimeDelta::hours(1);

/// How long a signed download URL works.
pub const DOWNLOAD_TTL: TimeDelta = TimeDelta::minutes(15);

/// How often a building bundle's heartbeat moves.
const HEARTBEAT_EVERY: Duration = Duration::from_secs(60);

/// How long a build may go without a heartbeat before the sweep fails it.
const STALE_AFTER: TimeDelta = TimeDelta::minutes(10);

/// How often the sweep runs.
const SWEEP_EVERY: Duration = Duration::from_hours(1);

/// The sweep's name in the worker ledger.
const SWEEP_WORKER: &str = "originals bundle sweep";

/// Bytes one streamed read takes.
const STREAM_CHUNK: u64 = 8 << 20;

/// What the athlete sees of a bundle, in the export and from its routes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BundleView {
    /// The bundle's id.
    pub bundle_id: Uuid,
    /// `building`, `ready` or `failed`.
    pub status: &'static str,
    /// Whether it was sealed with a passphrase.
    pub sealed: bool,
    /// When it was asked for.
    pub created_at: DateTime<Utc>,
    /// When it stops being downloadable.
    pub expires_at: DateTime<Utc>,
    /// Files it holds, once ready.
    pub files: Option<u64>,
    /// Files the store no longer held, once ready.
    pub missing: Option<u64>,
    /// Files whose bytes did not match their digest, once ready.
    pub damaged: Option<u64>,
    /// The download's size, once ready.
    pub byte_size: Option<u64>,
    /// The download's SHA-256, once ready.
    pub sha256: Option<String>,
    /// Where the athlete downloads it, once ready.
    pub download: Option<String>,
}

impl BundleView {
    fn of(bundle: &ExportBundle) -> Self {
        let ready = bundle.status == BundleStatus::Ready;
        Self {
            bundle_id: bundle.id,
            status: bundle.status.as_str(),
            sealed: bundle.sealed,
            created_at: bundle.created_at,
            expires_at: bundle.expires_at,
            files: bundle.files,
            missing: bundle.missing,
            damaged: bundle.damaged,
            byte_size: bundle.byte_size,
            sha256: bundle.sha256.clone(),
            download: ready.then(|| format!("/api/user/export/originals/{}/download", bundle.id)),
        }
    }
}

/// How a ready bundle is downloaded.
pub enum BundleDownload {
    /// From the store, at a signed URL, until `expires_at`.
    Signed {
        /// The URL: a bearer credential, handed only to the bundle's owner.
        url: String,
        /// When it stops working.
        expires_at: DateTime<Utc>,
    },
    /// Through the API: the bundle's bytes.
    Stream {
        /// Its size.
        size: u64,
        /// Whether it is sealed, so the file is typed for what it is.
        sealed: bool,
        /// The name the file is saved as.
        filename: String,
        /// The bytes.
        body: BoxStream<'static, Result<Bytes, BarrageError>>,
    },
}

impl fmt::Debug for BundleDownload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Signed { expires_at, .. } => f
                .debug_struct("Signed")
                .field("expires_at", expires_at)
                .finish_non_exhaustive(),
            Self::Stream {
                size,
                sealed,
                filename,
                ..
            } => f
                .debug_struct("Stream")
                .field("size", size)
                .field("sealed", sealed)
                .field("filename", filename)
                .finish_non_exhaustive(),
        }
    }
}

/// What a request for a bundle got.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BundleRequest {
    /// The bundle the athlete gets: one started now, the unsealed one
    /// already building, or an unsealed one ready for less than an hour.
    Bundle(BundleView),
    /// This instance builds as many bundles as it allows; retry shortly.
    Busy,
    /// A bundle that cannot stand for this one is building now (one of the
    /// two is sealed); retry once it is ready.
    OtherBuilding(BundleView),
}

/// What [`OriginalsBundleService::admit`] decided.
#[derive(Debug)]
pub enum Admission {
    /// No new build: this is the answer, and nothing was spent on it.
    Answered(BundleRequest),
    /// A new build may start in this slot ([`OriginalsBundleService::start`]).
    Admitted(BuildSlot),
}

/// One of the instance's build slots, held from admission until the build it
/// was taken for ends; dropping it unused gives it back.
#[derive(Debug)]
pub struct BuildSlot(OwnedSemaphorePermit);

/// The answer a building `bundle` gives a request (`sealed` when it carries
/// a passphrase): that bundle when both are unsealed, otherwise a conflict.
fn in_flight(bundle: &ExportBundle, sealed: bool) -> BundleRequest {
    let view = BundleView::of(bundle);
    if bundle.sealed || sealed {
        BundleRequest::OtherBuilding(view)
    } else {
        BundleRequest::Bundle(view)
    }
}

/// Aborts its task when dropped: a build that ends, fails or panics stops
/// its heartbeat, so a dead build's row goes stale and the sweep fails it.
struct AbortOnDrop(AbortHandle);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// What the sweep did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct BundleSweep {
    /// Builds failed because their heartbeat stopped.
    pub stalled: u64,
    /// Expired bundles deleted.
    pub expired: u64,
}

/// The file name a download is saved as.
fn download_filename(created_at: DateTime<Utc>, sealed: bool) -> String {
    let extension = if sealed { "zip.age" } else { "zip" };
    format!(
        "dravr-originals-{}.{extension}",
        created_at.format("%Y-%m-%d")
    )
}

/// Builds, tracks and hands out the athletes' originals bundles.
pub struct OriginalsBundleService {
    /// The context's one record store: the originals the bundle holds are the
    /// ones its archive records keep.
    records: Arc<dyn ActivityRecordStore>,
    /// The deployment's blob store: originals, uploads and bundles.
    blobs: Arc<dyn BlobStore>,
    /// The context's repositories, shared with every route and with each
    /// build and heartbeat task this service spawns.
    repos: Arc<RepositoryRegistry>,
    /// Build slots; a permit is held for as long as its bundle is written.
    slots: Arc<Semaphore>,
}

impl fmt::Debug for OriginalsBundleService {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OriginalsBundleService")
            .field("free_slots", &self.slots.available_permits())
            .finish_non_exhaustive()
    }
}

/// The table `name` the export walks.
fn export_table(name: &str) -> AppResult<&'static UserExportTable> {
    USER_EXPORT_TABLES
        .iter()
        .find(|table| table.name == name)
        .ok_or_else(|| AppError::internal(format!("the export walks no table `{name}`")))
}

/// `row[column]` as text.
fn text<'row>(row: &'row Value, column: &str) -> Option<&'row str> {
    row.get(column).and_then(Value::as_str)
}

/// The upload key `row` names: its tenant, the athlete and a digest, keyed
/// as the originals store keys every upload.
fn upload_key(row: &Value, user_id: Uuid, digest_column: &str) -> Option<BlobKey> {
    let tenant = TenantId::parse_str(text(row, "tenant_id")?).ok()?;
    originals::upload_key(&tenant, user_id, text(row, digest_column)?).ok()
}

impl OriginalsBundleService {
    /// The service over the context's one record store, the deployment's blob
    /// store and its repositories.
    fn new(
        records: Arc<dyn ActivityRecordStore>,
        blobs: Arc<dyn BlobStore>,
        repos: Arc<RepositoryRegistry>,
    ) -> Self {
        Self {
            records,
            blobs,
            repos,
            slots: Arc::new(Semaphore::new(MAX_CONCURRENT_BUNDLES)),
        }
    }

    /// The service over the context's one record store and the deployment's
    /// blob store, when one is placed: without one no bundle can be stored.
    #[must_use]
    pub fn over(
        records: &Arc<dyn ActivityRecordStore>,
        blobs: &Result<Arc<dyn BlobStore>, String>,
        repos: &Arc<RepositoryRegistry>,
    ) -> Option<Arc<Self>> {
        let blobs = blobs.as_ref().ok()?;
        Some(Arc::new(Self::new(
            Arc::clone(records),
            Arc::clone(blobs),
            Arc::clone(repos),
        )))
    }

    /// The athlete's latest bundle in `tenant_id`, as they see it.
    ///
    /// # Errors
    /// A database error when the read fails.
    pub async fn latest(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Option<BundleView>> {
        Ok(self
            .repos
            .user_export_bundles
            .latest(tenant_id, user_id)
            .await?
            .as_ref()
            .map(BundleView::of))
    }

    /// The athlete's bundle `id`, as they see it.
    ///
    /// # Errors
    /// Not found when it is not theirs; a database error when the read fails.
    pub async fn view(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        id: Uuid,
    ) -> AppResult<BundleView> {
        Ok(BundleView::of(&self.owned(user_id, tenant_id, id).await?))
    }

    async fn owned(&self, user_id: Uuid, tenant_id: TenantId, id: Uuid) -> AppResult<ExportBundle> {
        self.repos
            .user_export_bundles
            .get(tenant_id, user_id, id)
            .await?
            .ok_or_else(|| AppError::not_found("Export bundle"))
    }

    /// Whether a request for a bundle (`sealed` when the athlete gave a
    /// passphrase) needs a new build, and the slot to write it in.
    ///
    /// Costs nothing the athlete is counted for: the one already building
    /// when it can stand for this one, an unsealed one ready for less than
    /// an hour when an unsealed one is asked for, a sealed build in
    /// flight, or a full instance are each answered here. Only an
    /// [`Admission::Admitted`] request goes on to the meter, the key
    /// derivation and [`Self::start`], in that order, so a refused request
    /// never spends the athlete's allowance or the KDF's memory.
    ///
    /// A sealed build never stands for another sealed request: the server
    /// cannot tell whether the two passphrases match, and answering it would
    /// hand the athlete a file their new passphrase does not open.
    ///
    /// # Errors
    /// A database error.
    pub async fn admit(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        sealed: bool,
    ) -> AppResult<Admission> {
        let latest = self
            .repos
            .user_export_bundles
            .latest(tenant_id, user_id)
            .await?;
        if let Some(latest) = latest {
            // The athlete's own build answers before any slot is asked for:
            // a full instance is never "busy" for the bundle already theirs.
            if latest.status == BundleStatus::Building {
                return Ok(Admission::Answered(in_flight(&latest, sealed)));
            }
            let fresh_plain = latest.status == BundleStatus::Ready
                && !latest.sealed
                && !sealed
                && latest.finished_at.unwrap_or(latest.created_at) > Utc::now() - REUSE_WINDOW;
            if fresh_plain {
                return Ok(Admission::Answered(BundleRequest::Bundle(BundleView::of(
                    &latest,
                ))));
            }
        }
        Ok(Arc::clone(&self.slots).try_acquire_owned().map_or_else(
            |_| Admission::Answered(BundleRequest::Busy),
            |permit| Admission::Admitted(BuildSlot(permit)),
        ))
    }

    /// Start the build `slot` was admitted for, sealed by `seal` when the
    /// athlete gave a passphrase, and answer it; a build another request
    /// started meanwhile answers as [`Self::admit`] would.
    ///
    /// # Errors
    /// A database error.
    pub async fn start(
        self: &Arc<Self>,
        slot: BuildSlot,
        user_id: Uuid,
        tenant_id: TenantId,
        seal: Option<ExportSeal>,
    ) -> AppResult<BundleRequest> {
        let BuildSlot(permit) = slot;
        let sealed = seal.is_some();
        let bundles = &self.repos.user_export_bundles;
        let now = Utc::now();
        let asked = ExportBundle {
            id: Uuid::new_v4(),
            tenant_id,
            user_id,
            status: BundleStatus::Building,
            sealed,
            files: None,
            missing: None,
            damaged: None,
            byte_size: None,
            sha256: None,
            error: None,
            created_at: now,
            heartbeat_at: now,
            finished_at: None,
            // The sweep's backstop for a build nothing finishes.
            expires_at: now + BUNDLE_TTL,
        };
        let row = bundles.insert_building(&asked).await?;
        if row.id != asked.id {
            // One already building: it keeps its slot, this request needs none.
            drop(permit);
            return Ok(in_flight(&row, sealed));
        }
        let service = Arc::clone(self);
        let building = row.clone();
        let build = tokio::spawn(async move {
            service.build(building, seal, permit).await;
        });
        let id = row.id;
        tokio::spawn(async move {
            // A build that panicked left its row building: its heartbeat
            // stopped with it, so the sweep fails the row once it is stale.
            if let Err(error) = build.await {
                warn!(bundle_id = %id, error = %error, "Originals bundle build ended abnormally");
            }
        });
        Ok(BundleRequest::Bundle(BundleView::of(&row)))
    }

    /// Every file the athlete's bundle holds, in every tenant their rows reach.
    async fn files(&self, user_id: Uuid) -> AppResult<Vec<BundleFile>> {
        let mut files = Vec::new();
        let tenants = self
            .repos
            .tenants
            .list_membership_tenant_ids(user_id)
            .await?;
        for tenant in tenants {
            let scope = Scope::new(Uuid::from(tenant), user_id);
            files.extend(
                export::original_files(self.records.as_ref(), &scope)
                    .await
                    .map_err(|e| AppError::internal(format!("bundle originals: {e}")))?,
            );
        }
        self.walk(
            user_id,
            "uploaded_activity_files",
            |row| {
                upload_key(row, user_id, "file_sha256").map(|key| BundleFile {
                    key,
                    kind: BundleFileKind::Upload,
                    media_type: None,
                    records: Vec::new(),
                })
            },
            &mut files,
        )
        .await?;
        self.walk(
            user_id,
            "feedback_messages",
            |row| {
                upload_key(row, user_id, "image_sha256").map(|key| BundleFile {
                    key,
                    kind: BundleFileKind::FeedbackImage,
                    media_type: text(row, "image_media_type").map(str::to_owned),
                    records: Vec::new(),
                })
            },
            &mut files,
        )
        .await?;
        // One screenshot attached to two messages is one file.
        let mut seen = HashSet::new();
        files.retain(|file| seen.insert(file.key));
        Ok(files)
    }

    /// Walk the export's table `name` for the athlete, a page at a time,
    /// adding the file each row names.
    async fn walk(
        &self,
        user_id: Uuid,
        name: &str,
        file_of: impl Fn(&Value) -> Option<BundleFile> + Send + Sync,
        files: &mut Vec<BundleFile>,
    ) -> AppResult<()> {
        let table = export_table(name)?;
        let mut after = String::new();
        loop {
            let page = self
                .repos
                .user_export
                .export_page(table, user_id, &after)
                .await?;
            let read = i64::try_from(page.rows.len()).unwrap_or(i64::MAX);
            files.extend(page.rows.iter().filter_map(&file_of));
            match page.last_key {
                Some(last) if read >= table.page_rows => after = last,
                _ => return Ok(()),
            }
        }
    }

    /// Build `bundle`, then mark it ready, or failed with its blob deleted.
    /// A row gone meanwhile (the account was deleted or reset) takes the
    /// blob with it.
    async fn build(
        &self,
        bundle: ExportBundle,
        seal: Option<ExportSeal>,
        permit: OwnedSemaphorePermit,
    ) {
        let key = BlobKey::export_bundle(
            Scope::new(Uuid::from(bundle.tenant_id), bundle.user_id),
            bundle.id,
        );
        let written = Arc::new(AtomicU64::new(0));
        let heartbeat = AbortOnDrop(self.heartbeat(&bundle, Arc::clone(&written)));
        let outcome = self.write(&bundle, &key, seal, &written).await;
        drop(heartbeat);
        let kept = self.settle(&bundle, &outcome).await;
        if !kept {
            // Failed, or the row is gone or no longer building: the blob must
            // not outlive the row that names it.
            if let Err(error) = self.blobs.delete(&key).await {
                warn!(bundle_id = %bundle.id, error = %error, "An unkept bundle's blob could not be deleted; the sweep or the bucket lifecycle will");
            }
        }
        if let Ok(report) = &outcome {
            info!(
                bundle_id = %bundle.id,
                files = report.files,
                missing = report.missing,
                damaged = report.damaged,
                bytes = report.stat.size,
                kept,
                "Originals bundle built"
            );
        }
        drop(permit);
    }

    /// Record how the build of `bundle` ended; whether its blob is kept: it
    /// was written and its row, still building, is now ready.
    async fn settle(&self, bundle: &ExportBundle, outcome: &AppResult<BundleReport>) -> bool {
        let bundles = &self.repos.user_export_bundles;
        let (tenant, user, id) = (bundle.tenant_id, bundle.user_id, bundle.id);
        let finished = match outcome {
            Ok(report) => {
                let contents = BundleContents {
                    files: report.files,
                    missing: report.missing,
                    damaged: report.damaged,
                    byte_size: report.stat.size,
                    sha256: report.stat.sha256.to_hex(),
                };
                bundles
                    .finish_ready(tenant, user, id, &contents, Utc::now() + BUNDLE_TTL)
                    .await
            }
            Err(error) => {
                warn!(bundle_id = %id, error = %error, "Originals bundle build failed");
                bundles
                    .finish_failed(tenant, user, id, "the bundle could not be built")
                    .await
            }
        };
        match finished {
            Ok(done) => outcome.is_ok() && done,
            Err(error) => {
                warn!(bundle_id = %id, error = %error, "Originals bundle state could not be recorded");
                false
            }
        }
    }

    async fn write(
        &self,
        bundle: &ExportBundle,
        key: &BlobKey,
        seal: Option<ExportSeal>,
        written: &AtomicU64,
    ) -> AppResult<BundleReport> {
        let files = self.files(bundle.user_id).await?;
        let owner = BundleOwner {
            user_id: UserId(bundle.user_id),
            bundle_id: bundle.id,
            generated_at: bundle.created_at,
        };
        let mut seal = seal;
        let seal_ref = seal.as_mut().map(|seal| -> &mut dyn BundleSeal { seal });
        export::write_bundle(self.blobs.as_ref(), key, &owner, files, seal_ref, &|n| {
            written.store(n, Ordering::Relaxed);
        })
        .await
        .map_err(|e| AppError::internal(format!("bundle write: {e}")))
    }

    /// Move the bundle's heartbeat every [`HEARTBEAT_EVERY`] until aborted.
    fn heartbeat(&self, bundle: &ExportBundle, written: Arc<AtomicU64>) -> AbortHandle {
        let repos = Arc::clone(&self.repos);
        let (tenant, user, id) = (bundle.tenant_id, bundle.user_id, bundle.id);
        tokio::spawn(async move {
            let mut ticks = time::interval(HEARTBEAT_EVERY);
            ticks.tick().await;
            loop {
                ticks.tick().await;
                let files = written.load(Ordering::Relaxed);
                match repos
                    .user_export_bundles
                    .heartbeat(tenant, user, id, Utc::now())
                    .await
                {
                    Ok(true) => info!(bundle_id = %id, files, "Originals bundle still building"),
                    Ok(false) => return,
                    Err(error) => {
                        warn!(bundle_id = %id, error = %error, "Originals bundle heartbeat failed");
                    }
                }
            }
        })
        .abort_handle()
    }

    /// Say how the athlete downloads their ready bundle `id`.
    ///
    /// # Errors
    /// Not found when it is not theirs; a conflict when it is not ready; a
    /// resource-unavailable error when the store cannot hand it out.
    pub async fn download(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        id: Uuid,
    ) -> AppResult<BundleDownload> {
        let bundle = self.owned(user_id, tenant_id, id).await?;
        if bundle.status != BundleStatus::Ready {
            return Err(AppError::new(
                ErrorCode::ResourceLocked,
                "This bundle is not ready to download",
            ));
        }
        let key = BlobKey::export_bundle(Scope::new(Uuid::from(tenant_id), user_id), id);
        let filename = download_filename(bundle.created_at, bundle.sealed);
        let unavailable = |e: BarrageError| {
            warn!(bundle_id = %id, error = %e, "Originals bundle download unavailable");
            AppError::resource_unavailable("The bundle cannot be downloaded right now; retry later")
        };
        match self
            .blobs
            .open_download(&key, DOWNLOAD_TTL, &filename)
            .await
            .map_err(unavailable)?
        {
            DownloadTarget::Signed { url, expires_at } => {
                Ok(BundleDownload::Signed { url, expires_at })
            }
            DownloadTarget::ThroughApi => {
                let size = self
                    .blobs
                    .size(&key)
                    .await
                    .map_err(unavailable)?
                    .ok_or_else(|| AppError::not_found("Export bundle"))?;
                Ok(BundleDownload::Stream {
                    size,
                    sealed: bundle.sealed,
                    filename,
                    body: stream_blob(Arc::clone(&self.blobs), key, size),
                })
            }
        }
    }

    /// Fail every build whose heartbeat stopped, deleting its blob, then
    /// delete every expired bundle's blob and then its row.
    ///
    /// # Errors
    /// A database or store failure; the next pass picks up what this one did
    /// not reach, every step being idempotent.
    pub async fn sweep(&self, now: DateTime<Utc>) -> AppResult<BundleSweep> {
        let bundles = &self.repos.user_export_bundles;
        let mut done = BundleSweep::default();
        for stalled in bundles.stale_building(now - STALE_AFTER).await? {
            if bundles
                .finish_failed(
                    stalled.tenant_id,
                    stalled.user_id,
                    stalled.id,
                    "the build stopped",
                )
                .await?
            {
                self.delete_blob(&stalled).await?;
                done.stalled += 1;
            }
        }
        for expired in bundles.expired(now).await? {
            self.delete_blob(&expired).await?;
            bundles
                .delete(expired.tenant_id, expired.user_id, expired.id)
                .await?;
            done.expired += 1;
        }
        Ok(done)
    }

    async fn delete_blob(&self, bundle: &ExportBundle) -> AppResult<()> {
        let key = BlobKey::export_bundle(
            Scope::new(Uuid::from(bundle.tenant_id), bundle.user_id),
            bundle.id,
        );
        self.blobs
            .delete(&key)
            .await
            .map_err(|e| AppError::internal(format!("deleting a bundle's blob: {e}")))
    }
}

/// The blob at `key`, `size` bytes, read [`STREAM_CHUNK`] at a time.
fn stream_blob(
    blobs: Arc<dyn BlobStore>,
    key: BlobKey,
    size: u64,
) -> BoxStream<'static, Result<Bytes, BarrageError>> {
    Box::pin(stream::try_unfold(0_u64, move |at| {
        let blobs = Arc::clone(&blobs);
        async move {
            if at >= size {
                return Ok(None);
            }
            let end = size.min(at + STREAM_CHUNK);
            let chunk = blobs.get_range(&key, at..end).await?;
            Ok(Some((chunk, end)))
        }
    }))
}

/// Start the hourly sweep of `service`'s bundles, recorded in `repos`' worker
/// ledger so it runs once across instances.
#[must_use]
pub fn start_originals_bundle_sweep(
    service: &Arc<OriginalsBundleService>,
    repos: &Arc<RepositoryRegistry>,
) -> AbortHandle {
    let service = Arc::clone(service);
    spawn_periodic(
        SWEEP_WORKER,
        SWEEP_EVERY,
        Arc::clone(&repos.worker_runs),
        move || {
            let service = Arc::clone(&service);
            async move {
                let done = service.sweep(Utc::now()).await?;
                if done != BundleSweep::default() {
                    info!(
                        stalled = done.stalled,
                        expired = done.expired,
                        "Originals bundles swept"
                    );
                }
                Ok(())
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use std::future;

    use dravr_barrage::blob::memory::MemoryBlobStore;
    use dravr_barrage::store::memory::MemoryActivityRecordStore;
    use pierre_core::models::{Tenant, User};
    use pierre_test_support::db::create_test_db;

    use super::*;

    #[tokio::test]
    async fn a_build_that_panics_stops_its_heartbeat() {
        let beating = tokio::spawn(future::pending::<()>());
        let heartbeat = AbortOnDrop(beating.abort_handle());
        let build = tokio::spawn(async move {
            let _heartbeat = heartbeat;
            panic!("the build died");
        });
        assert!(
            build.await.unwrap_err().is_panic(),
            "bug: the test build was meant to panic"
        );
        let stopped = time::timeout(Duration::from_secs(5), beating)
            .await
            .expect("bug: a dead build's heartbeat kept its row fresh");
        assert!(
            stopped.unwrap_err().is_cancelled(),
            "bug: a dead build's heartbeat ended some other way than aborted"
        );
    }

    #[tokio::test]
    async fn an_athletes_own_build_answers_even_when_no_slot_is_free() {
        let db = create_test_db().await.unwrap();
        let repos = Arc::clone(db.repositories());
        let user = User::new("slots@example.com".to_owned(), "hash".to_owned(), None);
        repos.users.create(&user).await.unwrap();
        let tenant = Tenant::new(
            "Slots".to_owned(),
            format!("slots-{}", Uuid::new_v4()),
            None,
            "starter".to_owned(),
            user.id,
        );
        repos.tenants.create(&tenant).await.unwrap();
        let now = Utc::now();
        let building = repos
            .user_export_bundles
            .insert_building(&ExportBundle {
                id: Uuid::new_v4(),
                tenant_id: tenant.id,
                user_id: user.id,
                status: BundleStatus::Building,
                sealed: false,
                files: None,
                missing: None,
                damaged: None,
                byte_size: None,
                sha256: None,
                error: None,
                created_at: now,
                heartbeat_at: now,
                finished_at: None,
                expires_at: now + BUNDLE_TTL,
            })
            .await
            .unwrap();
        // Every slot held by other athletes' builds.
        let service = Arc::new(OriginalsBundleService {
            records: Arc::new(MemoryActivityRecordStore::new()),
            blobs: Arc::new(MemoryBlobStore::new()),
            repos,
            slots: Arc::new(Semaphore::new(0)),
        });
        let answer = service.admit(user.id, tenant.id, false).await.unwrap();
        assert!(
            matches!(&answer, Admission::Answered(BundleRequest::Bundle(view)) if *view == BundleView::of(&building)),
            "bug: the athlete's own build was answered busy: {answer:?}"
        );
        let stranger = Uuid::new_v4();
        assert!(
            matches!(
                service.admit(stranger, tenant.id, false).await.unwrap(),
                Admission::Answered(BundleRequest::Busy)
            ),
            "bug: a new build was admitted with no slot free"
        );
    }
}
