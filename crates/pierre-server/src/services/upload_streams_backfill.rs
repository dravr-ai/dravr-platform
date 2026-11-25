// ABOUTME: The backfill that writes current sample rows for uploads that lack one or hold an earlier format version
// ABOUTME: A bounded, idempotent pass on the worker ledger: decode each owed file once, store its sessions' series

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Upload samples backfill (carnet#1219).
//!
//! An upload writes each kept session's series to the one per-activity sample
//! store as it is decoded ([`crate::services::activity_upload`]). Activities
//! uploaded before that have only their file, and those whose row is at an
//! earlier format version (written before GPS and distance kept their gaps,
//! carnet#1226) are replayed from it: both are read from the file, decoded
//! again on every read. This worker writes their current rows.
//!
//! Why a periodic worker rather than a migration or a one-shot at boot: SQL
//! cannot decode a FIT file, so no migration can write the series; a one-shot
//! at boot would miss what a still-running older instance uploads during a
//! rollout. A pass on [`spawn_periodic`] runs once across instances (the
//! worker ledger), reads at most [`ACTIVITIES_PER_PASS`] owed activities,
//! decodes each owed file once on the blocking pool under the process-wide
//! decode bound, and writes its sessions' rows. It is idempotent — an
//! activity whose row is at the current version is no longer listed, and a
//! row written twice is the same row — and once nothing is owed, a pass is
//! one indexed read that returns nothing.
//!
//! Every file a pass reads was decoded by the same decoder when it was
//! uploaded, so a file that no longer decodes is a fault worth a warning,
//! not an expected state: it is logged and the pass goes on, and its
//! activities keep being read from the file, which fails the same way.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use dravr_barrage::parse::fit::FitActivityFile;
use dravr_tronc::dates::Timestamp;
use pierre_core::constants::oauth_providers::UPLOAD;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use pierre_database::repositories::ActivityWithoutStreams;
use pierre_database::RepositoryRegistry;
use pierre_providers::fit_decode::decode_bounded;
use pierre_providers::originals::OriginalsStore;
use pierre_providers::upload_provider::{
    parse_upload_activity_id, store_upload_series, upload_session_series, UploadActivityId,
    UploadedFileIds,
};
use pierre_services::periodic::spawn_periodic;
use tracing::{info, warn};
use uuid::Uuid;

use crate::mcp::resources::ServerContext;

/// The name every log line and the worker ledger row carry.
pub const WORKER_NAME: &str = "upload streams backfill";

/// Pass cadence: frequent enough that the backlog clears within hours of a
/// deploy, while a pass with nothing owed costs one indexed read.
pub const TICK_INTERVAL: Duration = Duration::from_mins(15);

/// The most owed activities one pass reads; the next pass takes the rest.
pub const ACTIVITIES_PER_PASS: i64 = 200;

/// What one pass did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UploadStreamsBackfillReport {
    /// Files decoded and their owed sessions' series stored.
    pub files: usize,
    /// Activities whose series were stored.
    pub activities: usize,
    /// Owed activities passed over: an id, tenant or user that does not
    /// parse, a file that is gone or no longer decodes, or a session the
    /// file does not hold.
    pub unreadable: usize,
}

/// What an owed activity's id names its file by.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum OwedBase {
    /// The file's digest, which the id carries.
    Sha(String),
    /// The keyed base the file's row records.
    Keyed(String),
}

impl OwedBase {
    /// The base the owed activities' ids carry.
    fn id_base(&self) -> &str {
        match self {
            Self::Sha(base) | Self::Keyed(base) => base,
        }
    }
}

/// One owed file: its tenant, owner and the base its ids name it by.
type OwedFile = (String, String, OwedBase);

/// Start the backfill on the worker ledger.
///
/// Skipped for an in-memory database, like the other workers, so a
/// throwaway server never writes behind a test's back.
pub fn start_upload_streams_backfill(resources: &Arc<ServerContext>) {
    if resources.common.config.database.url.is_memory() {
        return;
    }
    let ledger = Arc::clone(&resources.common.repos.worker_runs);
    let repos = Arc::clone(&resources.common.repos);
    let originals = Arc::clone(&resources.common.originals);
    spawn_periodic(WORKER_NAME, TICK_INTERVAL, ledger, move || {
        let repos = Arc::clone(&repos);
        let originals = Arc::clone(&originals);
        async move {
            let report = run_upload_streams_backfill_pass(&repos, &originals).await?;
            if report != UploadStreamsBackfillReport::default() {
                info!(
                    worker = WORKER_NAME,
                    files = report.files,
                    activities = report.activities,
                    unreadable = report.unreadable,
                    "upload streams backfill pass finished"
                );
            }
            Ok(())
        }
    });
}

/// Store the series of up to [`ACTIVITIES_PER_PASS`] uploaded activities no
/// sample row is stored for, decoding each of their files once.
///
/// # Errors
/// Returns a database error when the owed activities cannot be listed; one
/// file's failure is logged and the pass goes on.
pub async fn run_upload_streams_backfill_pass(
    repos: &RepositoryRegistry,
    originals: &OriginalsStore,
) -> AppResult<UploadStreamsBackfillReport> {
    let owed = repos
        .activity_streams
        .list_activities_without_current_streams(UPLOAD, ACTIVITIES_PER_PASS)
        .await?;
    let mut report = UploadStreamsBackfillReport::default();
    for ((tenant, user, base), indices) in by_file(owed, &mut report) {
        let owed_here = indices.len();
        match backfill_file(repos, originals, &tenant, &user, &base, indices).await {
            Ok(stored) => {
                report.files += 1;
                report.activities += stored;
                report.unreadable += owed_here - stored;
            }
            Err(e) => {
                report.unreadable += owed_here;
                warn!(
                    worker = WORKER_NAME,
                    user_id = %user,
                    tenant_id = %tenant,
                    id_base = %base.id_base(),
                    error = %e,
                    "uploaded file's samples not backfilled"
                );
            }
        }
    }
    Ok(report)
}

/// The owed activities grouped by the file they came from, each with the
/// session indices it owes; an id that names no file is counted unreadable.
fn by_file(
    owed: Vec<ActivityWithoutStreams>,
    report: &mut UploadStreamsBackfillReport,
) -> BTreeMap<OwedFile, Vec<usize>> {
    let mut files: BTreeMap<OwedFile, Vec<usize>> = BTreeMap::new();
    for activity in owed {
        let (base, index) = match parse_upload_activity_id(&activity.activity_id) {
            Some(UploadActivityId::Digest { sha, index }) => (OwedBase::Sha(sha.to_owned()), index),
            Some(UploadActivityId::Keyed { base, index }) => {
                (OwedBase::Keyed(base.to_owned()), index)
            }
            None => {
                report.unreadable += 1;
                continue;
            }
        };
        files
            .entry((activity.tenant_id, activity.user_id, base))
            .or_default()
            .push(index);
    }
    files
}

/// Decode one owed file and store the series of the sessions it owes;
/// returns how many were stored.
async fn backfill_file(
    repos: &RepositoryRegistry,
    originals: &OriginalsStore,
    tenant: &str,
    user: &str,
    base: &OwedBase,
    indices: Vec<usize>,
) -> AppResult<usize> {
    let tenant_id = TenantId::parse_str(tenant)
        .map_err(|e| AppError::invalid_input(format!("cached tenant id {tenant}: {e}")))?;
    let user_id = Uuid::parse_str(user)
        .map_err(|e| AppError::invalid_input(format!("cached user id {user}: {e}")))?;
    let sha = match base {
        OwedBase::Sha(sha) => sha.clone(),
        OwedBase::Keyed(keyed) => repos
            .uploaded_activity_files
            .file_of_record_base(&tenant_id, user_id, keyed)
            .await?
            .ok_or_else(|| AppError::not_found(format!("uploaded file of {keyed}")))?,
    };
    let bytes = originals
        .get_upload(&tenant_id, user_id, &sha)
        .await?
        .ok_or_else(|| AppError::not_found(format!("uploaded file {sha}")))?;
    let series = decode_bounded(move || {
        FitActivityFile::parse(&bytes).map(|file| upload_session_series(&file, indices))
    })
    .await?
    .map_err(|e| AppError::internal(format!("stored upload no longer decodes: {e}")))?;
    // A keyed file's samples name its keyed blob, never its digest.
    let keyed_name = match base {
        OwedBase::Sha(_) => None,
        OwedBase::Keyed(_) => Some(originals.keyed_name(&tenant_id, user_id, &sha)?.to_string()),
    };
    let ids = keyed_name.as_deref().map_or_else(
        || UploadedFileIds::by_digest(&sha),
        |source_ref| UploadedFileIds {
            id_base: base.id_base(),
            source_ref,
        },
    );
    store_upload_series(
        repos.activity_streams.as_ref(),
        &tenant_id,
        user_id,
        ids,
        &series,
        Timestamp::now(),
    )
    .await?;
    Ok(series.len())
}
