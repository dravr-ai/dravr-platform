// ABOUTME: Import your own data (carnet#1218): dravr-barrage wired into the platform — stores, link custody, steps
// ABOUTME: Chooses barrage's stores from the deployment, and is disabled, saying why, where it cannot run safely

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Importing an athlete's own export
//!
//! An athlete brings their history in from a service's bulk export (Strava
//! first): they paste the link the service emailed them, and dravr-barrage
//! spools the export, keeps each recording file as the original, and imports
//! it a chunk of rows per step. The platform owns everything around that:
//!
//! - **the stores** barrage runs on, chosen here from the deployment
//!   ([`ArchiveImports::from_env`]): Postgres schema `barrage` on the
//!   platform's own pool with GCS blobs when deployed, a separate SQLite file
//!   with blobs on disk locally, memory in tests;
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
//! with no blob bucket configured (`BARRAGE_GCS_BUCKET`), or one whose
//! `barrage` schema could not be migrated.

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
/// The periodic sweeps: links never pasted, and deleted accounts' originals.
pub mod sweeps;

use std::env;
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use dravr_barrage::blob::fs::FsBlobStore;
use dravr_barrage::blob::gcs::{AccessTokenSource, GcsBlobStore};
use dravr_barrage::blob::memory::MemoryBlobStore;
use dravr_barrage::blob::BlobStore;
use dravr_barrage::store::memory::{MemoryActivityRecordStore, MemoryImportJobStore};
use dravr_barrage::store::sql::sqlite::{self, SqliteJobStore, SqliteRecordStore};
use dravr_barrage::{BarrageError, BarrageStores};
use pierre_config::database::DatabaseUrl;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::gcp_token::{MetadataTokenProvider, TokenProvider};
use pierre_database::backends::factory::Database;
use tracing::{info, warn};

use self::fetch::{ExportFetcher, GuardedExportFetcher};
use self::runner::ImportRunner;

/// The GCS bucket a deployment keeps barrage's blobs in. Set, it selects the
/// GCS blob store; unset on a Postgres deployment, imports are disabled.
pub const ENV_GCS_BUCKET: &str = "BARRAGE_GCS_BUCKET";

/// The directory a local run keeps barrage's blobs in.
pub const ENV_BLOB_DIR: &str = "BARRAGE_BLOB_DIR";

/// The SQLite database a local run keeps barrage's records and jobs in: a
/// file of its own, never the platform's (the platform's migrator refuses a
/// history holding versions it does not know).
pub const ENV_SQLITE_URL: &str = "BARRAGE_SQLITE_URL";

/// Where a local run keeps blobs when [`ENV_BLOB_DIR`] is unset.
const DEFAULT_BLOB_DIR: &str = "./data/barrage-blobs";

/// Where a local run keeps records and jobs when [`ENV_SQLITE_URL`] is unset.
const DEFAULT_SQLITE_URL: &str = "sqlite:./data/barrage.db";

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

    /// The imports this deployment can run, chosen from its database and the
    /// environment, and the `barrage` schema migrated when on Postgres.
    ///
    /// Never fails the boot: a deployment that cannot place the stores runs
    /// with imports disabled, and says why once here.
    pub async fn from_env(database: &Database, url: &DatabaseUrl) -> Self {
        let imports = match build(PlatformPool::of(database), url.clone()).await {
            Ok(wiring) => Self::new(wiring),
            Err(reason) => Self::disabled(reason),
        };
        match &imports.wiring {
            Ok(wiring) => info!(runner = wiring.runner.label(), "Archive imports enabled"),
            Err(reason) => warn!(reason = %reason, "Archive imports disabled"),
        }
        imports
    }
}

/// The platform's Postgres pool, when the platform runs on Postgres: the
/// pool barrage's record and job stores run their statements on.
#[derive(Clone)]
struct PlatformPool(#[cfg(feature = "postgresql")] Option<sqlx::PgPool>);

impl PlatformPool {
    #[cfg(feature = "postgresql")]
    fn of(database: &Database) -> Self {
        Self(database.postgres_pool().cloned())
    }

    #[cfg(not(feature = "postgresql"))]
    const fn of(_database: &Database) -> Self {
        Self()
    }
}

/// The wiring the deployment selects.
///
/// Awaited by the binary before the server context is built, never inside
/// a future that must be `Send`: sqlx's migrator holds a connection across
/// awaits, which the compiler cannot prove `Send` for every lifetime.
async fn build(pool: PlatformPool, url: DatabaseUrl) -> Result<ImportWiring, String> {
    let stores = stores_for(pool, &url).await?;
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

/// The stores the deployment's database selects.
async fn stores_for(pool: PlatformPool, url: &DatabaseUrl) -> Result<BarrageStores, String> {
    let blobs = blob_store_for(url).await?;
    if url.is_memory() {
        return Ok(BarrageStores {
            blobs,
            records: Arc::new(MemoryActivityRecordStore::new()),
            jobs: Arc::new(MemoryImportJobStore::new()),
        });
    }
    if url.is_postgresql() {
        return postgres_stores(pool, url, blobs).await;
    }
    let sqlite_url = env::var(ENV_SQLITE_URL).unwrap_or_else(|_| DEFAULT_SQLITE_URL.to_owned());
    let pool = sqlite::open(&sqlite_url)
        .await
        .map_err(|e| format!("barrage SQLite database: {e}"))?;
    Ok(BarrageStores {
        blobs,
        records: Arc::new(SqliteRecordStore::new(pool.clone())),
        jobs: Arc::new(SqliteJobStore::new(pool)),
    })
}

/// The `barrage` schema migrated on a connection of its own, then the record
/// and job stores on the platform's pool: no connection slot is added.
#[cfg(feature = "postgresql")]
async fn postgres_stores(
    pool: PlatformPool,
    url: &DatabaseUrl,
    blobs: Arc<dyn BlobStore>,
) -> Result<BarrageStores, String> {
    use std::str::FromStr;

    use dravr_barrage::store::sql::postgres::{migrate, PostgresJobStore, PostgresRecordStore};
    use secrecy::ExposeSecret;
    use sqlx::postgres::PgConnectOptions;

    let pool = pool
        .0
        .ok_or_else(|| "the platform database is not Postgres".to_owned())?;
    let options = PgConnectOptions::from_str(url.connection_string().expose_secret())
        .map_err(|e| format!("the database URL does not parse: {e}"))?;
    migrate(&options)
        .await
        .map_err(|e| format!("the barrage schema could not be migrated: {e}"))?;
    Ok(BarrageStores {
        blobs,
        records: Arc::new(PostgresRecordStore::new(pool.clone())),
        jobs: Arc::new(PostgresJobStore::new(pool)),
    })
}

/// A build without Postgres support has no Postgres deployment to serve.
#[cfg(not(feature = "postgresql"))]
async fn postgres_stores(
    _pool: PlatformPool,
    _url: &DatabaseUrl,
    _blobs: Arc<dyn BlobStore>,
) -> Result<BarrageStores, String> {
    Err("this build has no Postgres support".to_owned())
}

/// The blob store: GCS when a bucket is configured, memory for an in-memory
/// database, a local directory for a SQLite run, and none for a Postgres
/// deployment without a bucket (its instances share no disk).
async fn blob_store_for(url: &DatabaseUrl) -> Result<Arc<dyn BlobStore>, String> {
    if let Some(bucket) = env::var(ENV_GCS_BUCKET)
        .ok()
        .filter(|b| !b.trim().is_empty())
    {
        let tokens: Arc<dyn AccessTokenSource> =
            Arc::new(PlatformTokens(Arc::new(MetadataTokenProvider::default())));
        let store = GcsBlobStore::try_new(bucket.trim(), tokens)
            .map_err(|e| format!("GCS blob store: {e}"))?;
        return Ok(Arc::new(store));
    }
    if url.is_memory() {
        return Ok(Arc::new(MemoryBlobStore::new()));
    }
    if url.is_postgresql() {
        return Err(format!(
            "{ENV_GCS_BUCKET} is unset, and a Postgres deployment keeps blobs only in GCS"
        ));
    }
    let dir =
        env::var(ENV_BLOB_DIR).map_or_else(|_| PathBuf::from(DEFAULT_BLOB_DIR), PathBuf::from);
    let store = FsBlobStore::open(dir)
        .await
        .map_err(|e| format!("blob directory: {e}"))?;
    Ok(Arc::new(store))
}

/// The platform's metadata-server tokens, as barrage's GCS store asks for
/// them: the provider caches each token until a minute before it expires.
struct PlatformTokens(Arc<dyn TokenProvider>);

#[async_trait]
impl AccessTokenSource for PlatformTokens {
    async fn access_token(&self) -> Result<String, BarrageError> {
        self.0
            .access_token()
            .await
            .map_err(|e| BarrageError::Blob(format!("access token: {e}")))
    }
}
