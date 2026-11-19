// ABOUTME: Import your own data (carnet#1218): dravr-barrage wired into the platform — stores, link custody, steps
// ABOUTME: Chooses barrage's stores from the deployment, and is disabled, saying why, where it cannot run safely

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Importing an athlete's own export
//!
//! An athlete brings their history in from a service's bulk export (Strava's
//! or Garmin Connect's): they paste the link the service emailed them, and
//! dravr-barrage spools the export, keeps each recording file as the
//! original, and imports it a chunk of rows per step. The platform owns
//! everything around that:
//!
//! - **the stores** barrage runs on, chosen here from the deployment
//!   ([`ArchiveImports::from_env`]): Postgres schema `barrage` on the
//!   platform's own pool with GCS blobs when deployed, a separate SQLite file
//!   with blobs on disk locally, memory in tests. The record and job stores
//!   are placed once by `pierre_services::barrage_live_store::record_stores`,
//!   and the context hands imports the same record store its live store and
//!   export read ([`ArchiveImports::record_store`]);
//! - **the projection** of each chunk into the activity cache and the one
//!   per-activity sample store ([`sink::CachedActivitiesSink`]);
//! - **the link**, encrypted bound to the athlete it was pasted by
//!   ([`link`]) and fetched only through tronc's `archive_fetch` guard
//!   ([`fetch`]);
//! - **the steps**, scheduled on a Cloud Tasks queue with one dispatch at a
//!   time, or in this process locally ([`runner`]);
//! - **the athlete's operations** — start, follow, cancel, read the report,
//!   delete the imported history ([`ops`]) — and the sweeps that expire a
//!   link never pasted and erase a deleted account's originals ([`sweeps`]).
//!
//! Imports are disabled, with the reason logged once at boot and answered to
//! every request, where the stores cannot be placed: a Postgres deployment
//! with no blob bucket configured (`BARRAGE_GCS_BUCKET`, see
//! [`crate::services::originals`]), or one whose `barrage` schema could not
//! be migrated.

/// Fetching an export through tronc's `archive_fetch` guard.
pub mod fetch;
/// The link an athlete pasted: encrypted bound to their account, digested for logs.
pub mod link;
/// The athlete's import operations behind the `/api/imports` routes.
pub mod ops;
/// How import steps are scheduled: Cloud Tasks when deployed, in this process locally.
pub mod runner;
/// The projection of imported records into the activity cache and sample store.
pub mod sink;
/// The periodic sweeps: links never pasted, uploads abandoned, and deleted accounts' originals.
pub mod sweeps;
/// An export the athlete uploads themselves rather than links.
pub mod upload;

use std::fmt;
use std::sync::Arc;

use dravr_barrage::blob::BlobStore;
use dravr_barrage::store::ActivityRecordStore;
use dravr_barrage::BarrageStores;
use pierre_core::errors::{AppError, AppResult};
use pierre_services::barrage_live_store::RecordStores;
use tracing::{info, warn};

use self::fetch::{ExportFetcher, GuardedExportFetcher};
use self::runner::ImportRunner;

/// What an import needs, once it can run.
pub struct ImportWiring {
    /// The stores barrage reads and writes.
    pub stores: BarrageStores,
    /// How an export is downloaded.
    pub fetcher: Arc<dyn ExportFetcher>,
    /// How steps are scheduled.
    pub runner: ImportRunner,
}

impl fmt::Debug for ImportWiring {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ImportWiring")
            .field("runner", &self.runner)
            .finish_non_exhaustive()
    }
}

/// Archive imports in this process: wired, or disabled with the reason.
#[derive(Debug, Clone)]
pub struct ArchiveImports {
    wiring: Result<Arc<ImportWiring>, String>,
}

impl ArchiveImports {
    /// Imports that run on `wiring`.
    #[must_use]
    pub fn new(wiring: ImportWiring) -> Self {
        Self {
            wiring: Ok(Arc::new(wiring)),
        }
    }

    /// Imports that are not available here, for `reason`.
    #[must_use]
    pub fn disabled(reason: impl Into<String>) -> Self {
        Self {
            wiring: Err(reason.into()),
        }
    }

    /// The wiring, or the refusal every import request answers with.
    ///
    /// # Errors
    ///
    /// [`AppError::resource_unavailable`] when imports are disabled here.
    pub fn wiring(&self) -> AppResult<&Arc<ImportWiring>> {
        self.wiring.as_ref().map_err(|reason| {
            AppError::resource_unavailable(format!(
                "Importing your own data is not available here: {reason}"
            ))
        })
    }

    /// Whether imports run here.
    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        self.wiring.is_ok()
    }

    /// The imports this deployment can run, on its record and job stores
    /// `stores` (the ones its live store runs on) and its blob store `blobs`
    /// ([`crate::services::originals::blob_store_from_env`], which the
    /// originals store shares), with the fetcher and runner the environment
    /// selects.
    ///
    /// Never fails the boot: a deployment that cannot run them runs with
    /// imports disabled, and says why once here.
    pub fn from_env(stores: &RecordStores, blobs: Result<Arc<dyn BlobStore>, String>) -> Self {
        let imports = match build(stores, blobs) {
            Ok(wiring) => Self::new(wiring),
            Err(reason) => Self::disabled(reason),
        };
        match &imports.wiring {
            Ok(wiring) => info!(runner = wiring.runner.label(), "Archive imports enabled"),
            Err(reason) => warn!(reason = %reason, "Archive imports disabled"),
        }
        imports
    }

    /// The context's one record store: the one these imports write, which
    /// the live store and the export must read, or `live` when imports do not
    /// run here, or a memory store when neither names one.
    ///
    /// # Errors
    ///
    /// An internal error when imports and `live` name two stores: an import's
    /// records would be invisible to the export and the live store.
    pub fn record_store(
        &self,
        live: Option<Arc<dyn ActivityRecordStore>>,
    ) -> AppResult<Arc<dyn ActivityRecordStore>> {
        let imported = self
            .wiring
            .as_ref()
            .ok()
            .map(|wiring| Arc::clone(&wiring.stores.records));
        match (imported, live) {
            (Some(imported), Some(live)) if !Arc::ptr_eq(&imported, &live) => Err(
                AppError::internal("archive imports and live reads were given two record stores"),
            ),
            (Some(records), _) | (None, Some(records)) => Ok(records),
            (None, None) => Ok(RecordStores::in_memory().records),
        }
    }
}

/// The wiring the deployment selects, on its record and job `stores`.
fn build(
    stores: &RecordStores,
    blobs: Result<Arc<dyn BlobStore>, String>,
) -> Result<ImportWiring, String> {
    let stores = stores.with_blobs(blobs?);
    let fetcher: Arc<dyn ExportFetcher> = Arc::new(
        GuardedExportFetcher::try_from_env().map_err(|e| format!("export fetch client: {e}"))?,
    );
    let runner = ImportRunner::from_env().map_err(|e| e.to_string())?;
    Ok(ImportWiring {
        stores,
        fetcher,
        runner,
    })
}
