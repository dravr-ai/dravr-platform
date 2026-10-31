// ABOUTME: The archive import sweeps: links never pasted expire; a deleted account's imports and uploaded files are erased
// ABOUTME: Each runs on the worker ledger, once across instances; an erasure that fails is retried with backoff

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Import sweeps
//!
//! - **Links never pasted.** An import waiting for its link is cancelled once
//!   it has waited `ops::AWAITING_LINK_DAYS` (barrage's
//!   `ImportJobStore::expire_awaiting`), so a forgotten request does not hold
//!   the source's one active import for ever.
//! - **Deleted accounts.** The account delete queues every deleted account in
//!   `archive_erasures` inside its own transaction; this sweep erases what
//!   barrage holds for each, in every tenant — records, jobs and their
//!   issues, originals and spools — and every file the account uploaded,
//!   which the originals store keeps in the same blob store, and removes the
//!   row once both confirm. It runs wherever a blob store is placed, imports
//!   or not. A failure is kept with its error and tried again later, each
//!   wait twice the last, up to [`MAX_BACKOFF`].

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use dravr_barrage::{Importer, UserId};
use pierre_core::errors::{AppError, AppResult};
use pierre_database::repositories::{ArchiveErasureRepository, PendingArchiveErasure};
use pierre_providers::originals::OriginalsStore;
use pierre_services::periodic::spawn_periodic;
use tracing::{info, warn};
use uuid::Uuid;

use super::ops;
use crate::mcp::resources::ServerContext;

/// The name the expiry sweep's ledger row and log lines carry.
pub const EXPIRY_WORKER: &str = "archive import link expiry";

/// The name the erasure sweep's ledger row and log lines carry.
pub const ERASURE_WORKER: &str = "archive erasure";

/// How often links never pasted are expired.
const EXPIRY_INTERVAL: Duration = Duration::from_hours(1);

/// How often deleted accounts' imported history is erased.
const ERASURE_INTERVAL: Duration = Duration::from_mins(5);

/// The most accounts one erasure pass works through.
const ERASURES_PER_PASS: i64 = 50;

/// The first wait after a failed erasure.
const FIRST_BACKOFF: Duration = Duration::from_mins(5);

/// The longest wait between two attempts at one erasure.
const MAX_BACKOFF: Duration = Duration::from_hours(6);

/// Start the sweeps: the link expiry where imports run, and the erasure of
/// deleted accounts' imported history and uploaded files everywhere.
pub fn start_archive_import_sweeps(resources: &Arc<ServerContext>) {
    let imports = resources.common.archive_imports.is_enabled();
    let ledger = Arc::clone(&resources.common.repos.worker_runs);
    if imports {
        let expiring = Arc::clone(resources);
        spawn_periodic(
            EXPIRY_WORKER,
            EXPIRY_INTERVAL,
            Arc::clone(&ledger),
            move || {
                let resources = Arc::clone(&expiring);
                async move { expire_awaiting_links(&resources).await.map(|_| ()) }
            },
        );
    }
    // The erasure queue is written by every account deletion, so it is
    // drained everywhere: where no blob store is placed, an entry completes
    // with nothing to erase (the uploads' bytes went with the rows).
    let erasing = Arc::clone(resources);
    spawn_periodic(ERASURE_WORKER, ERASURE_INTERVAL, ledger, move || {
        let resources = Arc::clone(&erasing);
        async move { erase_deleted_accounts(&resources).await.map(|_| ()) }
    });
}

/// Cancel every import that waited too long for its link; how many.
///
/// # Errors
///
/// When imports are not available here, or the store fails.
pub async fn expire_awaiting_links(resources: &Arc<ServerContext>) -> AppResult<u64> {
    let wiring = resources.common.archive_imports.wiring()?;
    let expired = wiring
        .stores
        .jobs
        .expire_awaiting(Utc::now())
        .await
        .map_err(|e| AppError::internal(format!("expiring imports: {e}")))?;
    if expired > 0 {
        info!(expired, "imports never given their link were cancelled");
    }
    Ok(expired)
}

/// Erase what barrage and the originals store hold for each deleted account
/// due; how many were erased. One account's failure is recorded and the pass
/// goes on.
///
/// # Errors
///
/// When the outbox cannot be read.
pub async fn erase_deleted_accounts(resources: &Arc<ServerContext>) -> AppResult<usize> {
    let importer = resources
        .common
        .archive_imports
        .wiring()
        .ok()
        .map(|wiring| ops::importer(resources, wiring));
    let originals = resources.common.originals.as_ref();
    let outbox = resources.common.repos.archive_erasures.as_ref();
    let now = Utc::now();
    let mut erased = 0;
    let due = outbox.due(now, ERASURES_PER_PASS).await?;
    for pending in due {
        if erase_one(importer.as_ref(), originals, outbox, &pending, now).await? {
            erased += 1;
        }
    }
    Ok(erased)
}

/// Erase one deleted account; whether it was erased. A failure is recorded
/// with the time of the next attempt.
async fn erase_one(
    importer: Option<&Importer>,
    originals: &OriginalsStore,
    outbox: &dyn ArchiveErasureRepository,
    pending: &PendingArchiveErasure,
    now: DateTime<Utc>,
) -> AppResult<bool> {
    let Ok(account) = Uuid::parse_str(&pending.erased_account_id) else {
        warn!("an archive erasure names no account id; it is dropped");
        outbox.complete(&pending.erased_account_id).await?;
        return Ok(false);
    };
    match erase_account(importer, originals, account).await {
        Ok((records, jobs, blobs)) => {
            outbox.complete(&pending.erased_account_id).await?;
            info!(
                records,
                jobs, blobs, "a deleted account's imported history and uploaded files were erased"
            );
            Ok(true)
        }
        Err(e) => {
            record_failure(outbox, pending, &e, now).await?;
            Ok(false)
        }
    }
}

/// Erase what barrage holds for `account` (when imports run here), then
/// every blob left under the account (its uploaded files): the records,
/// jobs and blobs removed. Both are idempotent, so a retry after a failure
/// part-way finishes the job.
async fn erase_account(
    importer: Option<&Importer>,
    originals: &OriginalsStore,
    account: Uuid,
) -> Result<(u64, u64, u64), String> {
    let (records, jobs, mut blobs) = match importer {
        Some(importer) => {
            let deleted = importer
                .delete_user(UserId(account))
                .await
                .map_err(|e| e.to_string())?;
            (deleted.records, deleted.jobs, deleted.blobs)
        }
        None => (0, 0, 0),
    };
    blobs += originals
        .erase_user(account)
        .await
        .map_err(|e| e.to_string())?;
    Ok((records, jobs, blobs))
}

/// Keep a failed erasure's `error` and hold it until its next attempt.
async fn record_failure(
    outbox: &dyn ArchiveErasureRepository,
    pending: &PendingArchiveErasure,
    error: &str,
    now: DateTime<Utc>,
) -> AppResult<()> {
    let wait = backoff(pending.attempts);
    let next = now + TimeDelta::from_std(wait).unwrap_or_else(|_| TimeDelta::hours(6));
    warn!(
        attempts = pending.attempts + 1,
        error, "a deleted account's imported history could not be erased yet"
    );
    outbox
        .record_failure(&pending.erased_account_id, error, next)
        .await
}

/// The wait after `attempts` failures: doubling from [`FIRST_BACKOFF`],
/// never past [`MAX_BACKOFF`].
fn backoff(attempts: i64) -> Duration {
    let doublings = u32::try_from(attempts.clamp(0, 16)).unwrap_or(16);
    FIRST_BACKOFF
        .saturating_mul(2_u32.saturating_pow(doublings))
        .min(MAX_BACKOFF)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wait_doubles_and_stops_at_its_ceiling() {
        assert_eq!(backoff(0), FIRST_BACKOFF, "the first wait");
        assert_eq!(backoff(1), FIRST_BACKOFF * 2, "doubled once");
        assert_eq!(backoff(40), MAX_BACKOFF, "capped");
    }
}
