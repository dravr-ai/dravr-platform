// ABOUTME: The extraction job ledger: record the owed run, keep its lease alive, finish it, and log how each run ended
// ABOUTME: A ledger write that fails is logged and never decides the run; the resume sweep reads what these rows leave
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use pierre_core::errors::AppResult;
use pierre_core::models::TenantId;
use pierre_database::repositories::{
    ExtractionRunOutcome, ExtractionRunRecord, JobLease, MemoryExtractionJobRepository,
    MemoryExtractionJobRow,
};
use tokio::task::JoinHandle;
use tokio::time::sleep;
use tracing::{info, warn};
use uuid::Uuid;

use super::{CountedRun, SpawnedExtractionRequest, EXTRACTION_JOB_LEASE};

/// Write how a run ended to the run ledger, which is what the admin memory
/// worker screen measures. The ledger reports on the extraction and never
/// decides it: a write that fails is logged and the run stands.
///
/// A run that lost a write is failed, and it still counts the facts it did
/// write; a run whose provider failed wrote none.
pub(super) async fn record_extraction_run(
    jobs: &dyn MemoryExtractionJobRepository,
    tenant_id: TenantId,
    result: &AppResult<CountedRun>,
) {
    let written = |run: &CountedRun| i64::try_from(run.outcome.persisted.len()).unwrap_or(i64::MAX);
    let (outcome, facts) = match result {
        Ok(run) if run.lost_writes > 0 => (ExtractionRunOutcome::Failed, written(run)),
        Ok(run) if run.outcome.persisted.is_empty() => (ExtractionRunOutcome::Empty, 0),
        Ok(run) => (ExtractionRunOutcome::Persisted, written(run)),
        Err(_) => (ExtractionRunOutcome::Failed, 0),
    };
    let record = ExtractionRunRecord {
        tenant_id,
        outcome,
        facts_persisted: facts,
        finished_at_ms: Utc::now().timestamp_millis(),
    };
    if let Err(e) = jobs.record_extraction_run(&record).await {
        warn!(error = %e, "extraction run could not be recorded");
    }
}

/// Record the extraction a spawned task is about to run.
///
/// The row is what the resume sweep re-runs if this instance dies with the
/// extraction in flight. `None` when it could not be written — the run still
/// happens, unrecorded, exactly as it did before the ledger existed.
pub(super) async fn record_extraction_job(
    jobs: &dyn MemoryExtractionJobRepository,
    req: &SpawnedExtractionRequest,
) -> Option<JobLease> {
    let payload = match serde_json::to_string(&req.payload) {
        Ok(json) => json,
        Err(e) => {
            warn!(error = %e, "extraction payload could not be serialised; running it unrecorded");
            return None;
        }
    };
    let now = Utc::now().timestamp_millis();
    let lease_ms = i64::try_from(EXTRACTION_JOB_LEASE.as_millis()).unwrap_or(i64::MAX);
    let row = MemoryExtractionJobRow {
        id: Uuid::new_v4().to_string(),
        tenant_id: req.tenant_id,
        usage_tenant_id: Some(req.usage_tenant_id),
        payload,
        created_at_ms: now,
        leased_until_ms: now.saturating_add(lease_ms),
        attempts: 1,
        lease_token: Some(Uuid::new_v4().to_string()),
    };
    match jobs.record_extraction_job(&row).await {
        Ok(()) => Some(JobLease {
            token: row.lease_token.unwrap_or_default(),
            id: row.id,
        }),
        Err(e) => {
            warn!(error = %e, "extraction job could not be recorded; running it unrecorded");
            None
        }
    }
}

/// How often a live extraction renews its job's lease.
///
/// The lease starts when the row is recorded, before the run waits for one of
/// [`MAX_CONCURRENT_EXTRACTIONS`](super::MAX_CONCURRENT_EXTRACTIONS) permits;
/// a run queued behind a burst of turns, or held by a slow model, outlived
/// [`EXTRACTION_JOB_LEASE`] while it
/// was still alive, and the resume sweep ran it a second time beside it. A
/// third of the lease leaves two renewals of slack before it can lapse.
const EXTRACTION_LEASE_RENEWAL: Duration = Duration::from_secs(EXTRACTION_JOB_LEASE.as_secs() / 3);

/// The lease renewal a live extraction keeps running; aborted when dropped.
pub(crate) struct LeaseRenewal(JoinHandle<()>);

impl Drop for LeaseRenewal {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Renew `lease` every [`EXTRACTION_LEASE_RENEWAL`] until the guard drops. A
/// refused renewal means the row is gone or another claim took it: the
/// renewals stop and the run finishes.
pub(crate) fn keep_extraction_lease(
    jobs: Arc<dyn MemoryExtractionJobRepository>,
    lease: Option<JobLease>,
) -> Option<LeaseRenewal> {
    let lease = lease?;
    Some(LeaseRenewal(tokio::spawn(async move {
        let lease_ms = i64::try_from(EXTRACTION_JOB_LEASE.as_millis()).unwrap_or(i64::MAX);
        loop {
            sleep(EXTRACTION_LEASE_RENEWAL).await;
            let until = Utc::now().timestamp_millis().saturating_add(lease_ms);
            match jobs.renew_extraction_job_lease(&lease, until).await {
                Ok(true) => {}
                Ok(false) => {
                    warn!(job_id = %lease.id, "extraction job is no longer this runner's; letting the run finish");
                    return;
                }
                Err(e) => {
                    warn!(job_id = %lease.id, error = %e, "extraction job lease could not be renewed");
                }
            }
        }
    })))
}

/// Delete the job row once nothing is owed on it any more, if this run's
/// lease still holds it; a claim that took it over finishes it instead.
pub(super) async fn finish_extraction_job(
    jobs: &dyn MemoryExtractionJobRepository,
    lease: Option<&JobLease>,
) {
    let Some(lease) = lease else {
        return;
    };
    match jobs.finish_extraction_job(lease).await {
        Ok(true) => {}
        Ok(false) => {
            info!(job_id = %lease.id, "extraction job was claimed by another run; leaving it to that run");
        }
        Err(e) => {
            warn!(error = %e, job_id = %lease.id, "extraction job could not be finished; the sweep will re-run it after its lease");
        }
    }
}
