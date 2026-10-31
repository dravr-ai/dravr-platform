// ABOUTME: The deployment's blob store — GCS deployed, a directory locally, memory in tests — and the originals drain
// ABOUTME: One store serves archive imports and uploaded files; the drain moves table-held upload bytes into it

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Where original files are kept
//!
//! One blob store per deployment ([`blob_store_from_env`]) keeps every
//! original file Dravr holds: the recordings an archive import keeps and the
//! files athletes upload ([`pierre_providers::originals::OriginalsStore`]).
//! GCS when a bucket is configured; memory for an in-memory database (tests
//! and throwaway servers, whose files go with the process); a local
//! directory for a SQLite run; and none for a Postgres deployment with no
//! bucket, whose instances share no disk — there imports are disabled,
//! saying why, and uploads stay in the table until a bucket is applied.
//!
//! [`start_originals_drain`] runs the drain: files uploaded before the blob
//! store kept them still hold their bytes in `uploaded_activity_files`, and
//! each pass copies a batch out, verified against its hash and size, and
//! clears the table's copy. The first pass waits until this process has
//! served for [`DRAIN_SETTLE`]: an older revision still serving beside a
//! fresh one during a rollout reads uploaded files from the table only, so
//! the table's copies stay until that revision is gone. The column is
//! dropped once no row holds bytes (Athlete Data Consolidation, S12;
//! carnet#1309).

use std::env;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use dravr_barrage::blob::fs::FsBlobStore;
use dravr_barrage::blob::gcs::{AccessTokenSource, GcsBlobStore};
use dravr_barrage::blob::memory::MemoryBlobStore;
use dravr_barrage::blob::BlobStore;
use dravr_barrage::BarrageError;
use pierre_config::database::DatabaseUrl;
use pierre_core::gcp_token::{MetadataTokenProvider, TokenProvider};
use pierre_services::periodic::spawn_periodic;
use tracing::debug;

use crate::mcp::resources::ServerContext;

/// The GCS bucket a deployment keeps its blobs in. Set, it selects the GCS
/// blob store; unset on a Postgres deployment, there is no blob store.
pub const ENV_GCS_BUCKET: &str = "BARRAGE_GCS_BUCKET";

/// The directory a local run keeps its blobs in.
pub const ENV_BLOB_DIR: &str = "BARRAGE_BLOB_DIR";

/// Where a local run keeps blobs when [`ENV_BLOB_DIR`] is unset.
const DEFAULT_BLOB_DIR: &str = "./data/barrage-blobs";

/// The name the drain's ledger row and log lines carry.
pub const DRAIN_WORKER: &str = "originals drain";

/// How often the drain runs: often enough that the files uploaded before a
/// deploy are out of the table within the hour.
const DRAIN_INTERVAL: Duration = Duration::from_mins(10);

/// How long this process serves before its drain clears any table copy.
///
/// A rollout's canary moves the rest of the traffic within minutes of the
/// new revision's first pod reading its key; a quarter hour outlasts the old
/// revision's last in-flight request with room to spare.
pub const DRAIN_SETTLE: Duration = Duration::from_mins(15);

/// The most files one drain pass copies.
pub const FILES_PER_DRAIN: i64 = 50;

/// The blob store the deployment selects.
///
/// GCS when a bucket is configured, memory for an in-memory database, a
/// local directory for a SQLite run, and none for a Postgres deployment
/// without a bucket.
///
/// # Errors
///
/// Why there is none: a Postgres deployment with no bucket, a GCS client
/// that cannot be built, or a blob directory that cannot be opened.
pub async fn blob_store_from_env(url: &DatabaseUrl) -> Result<Arc<dyn BlobStore>, String> {
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

/// The blob store of a context built without one.
///
/// Memory for an in-memory database, whose files go with the process anyway,
/// and none otherwise, so a database that outlives the process never names
/// bytes only its memory held.
///
/// # Errors
///
/// Why there is none, for every database but an in-memory one.
pub fn blob_store_when_unset(url: &DatabaseUrl) -> Result<Arc<dyn BlobStore>, String> {
    if url.is_memory() {
        let memory: Arc<dyn BlobStore> = Arc::new(MemoryBlobStore::new());
        return Ok(memory);
    }
    Err("this server was started without a blob store".to_owned())
}

/// Whether a process that has served for `uptime` may clear table copies:
/// once [`DRAIN_SETTLE`] has passed.
#[must_use]
pub fn drain_settled(uptime: Duration) -> bool {
    uptime >= DRAIN_SETTLE
}

/// Start the drain on the worker ledger, where a blob store is placed.
///
/// Skipped for an in-memory database, like the other workers, so a
/// throwaway server never writes behind a test's back.
pub fn start_originals_drain(resources: &Arc<ServerContext>) {
    if resources.common.config.database.url.is_memory() || !resources.common.originals.keeps_blobs()
    {
        return;
    }
    let ledger = Arc::clone(&resources.common.repos.worker_runs);
    let originals = Arc::clone(&resources.common.originals);
    let started = Instant::now();
    spawn_periodic(DRAIN_WORKER, DRAIN_INTERVAL, ledger, move || {
        let originals = Arc::clone(&originals);
        async move {
            if !drain_settled(started.elapsed()) {
                debug!("originals drain waits for the rollout to settle");
                return Ok(());
            }
            originals.drain_pending(FILES_PER_DRAIN).await.map(|_| ())
        }
    });
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_drain_waits_out_a_rollout_before_clearing_anything() {
        assert!(
            !drain_settled(Duration::ZERO),
            "bug: a fresh process cleared table copies an older revision still reads"
        );
        assert!(
            !drain_settled(DRAIN_SETTLE.saturating_sub(Duration::from_secs(1))),
            "bug: the drain cleared table copies before the rollout settled"
        );
        assert!(
            drain_settled(DRAIN_SETTLE),
            "bug: the drain never starts once the rollout has settled"
        );
    }

    #[test]
    fn a_context_without_a_blob_store_keeps_blobs_in_memory_only_for_a_memory_database() {
        let memory = DatabaseUrl::parse_url("sqlite::memory:").expect("memory url");
        assert!(
            blob_store_when_unset(&memory).is_ok(),
            "bug: an in-memory database keeps blobs in memory"
        );
        let postgres = DatabaseUrl::parse_url("postgresql://u:p@localhost/db").expect("pg url");
        assert!(
            blob_store_when_unset(&postgres).is_err(),
            "bug: Postgres must never keep blobs only in process memory"
        );
        let file = DatabaseUrl::parse_url("sqlite:./data/x.db").expect("file url");
        assert!(
            blob_store_when_unset(&file).is_err(),
            "bug: a SQLite file outlives the process, so its blobs must not live in memory"
        );
    }
}
