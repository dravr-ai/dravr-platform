// ABOUTME: Re-runs the post-turn memory extractions an instance died owing — claimed from the job ledger, run, finished
// ABOUTME: One periodic sweep per instance; a row survives a failed run and is dropped only when its payload can never run
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Memory extraction resume.
//!
//! A turn records the extraction it owes in `memory_extraction_jobs` before
//! spawning it (see [`crate::memory_extraction`]). When the instance running
//! that spawn is taken down first — the Cloud Run idle scaledown reads the
//! instance as idle from the moment the reply is flushed — the row is left
//! with a lease that lapses. This sweep is what takes such a row: it claims
//! what is stale, runs each extraction through the same
//! [`run_extraction_job`] body the turn would have, and deletes the row on
//! success. A run that fails keeps its row; the claim already counted the
//! attempt and set a fresh lease, so the next sweep after that lease retries
//! it, and the ledger's attempt cap stops one that keeps dying.
//!
//! The one row this sweep drops without running is a payload that does not
//! parse. Nothing will ever be able to run it, and leaving it would have the
//! sweep re-claim it every lease until the cap for no gain.

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use futures_util::future::join_all;
use pierre_contremaitre::harness_config_registry::HarnessConfigRegistry;
use pierre_core::errors::AppResult;
use pierre_database::repositories::{ExtractionJobClaim, MemoryExtractionJobRow};
use pierre_database::RepositoryRegistry;
use pierre_llm::ChatProvider;
use tracing::{info, warn};

use crate::memory_dedup::DedupConfig;
use crate::memory_extraction::{run_extraction_job, ExtractionJobPayload, EXTRACTION_JOB_LEASE};
use crate::periodic::spawn_periodic;

/// How often each instance sweeps. An extraction is one LLM call, so a
/// minute is the granularity at which a lost one is worth noticing; the
/// worker ledger keeps the schedule across instances.
pub const RESUME_INTERVAL: Duration = Duration::from_mins(1);

/// A row younger than this is one its own spawn is still running: the
/// record is written, then the run queues for a permit and makes its call.
/// Only a row older than this AND past its lease is presumed abandoned.
const QUEUED_GRACE: Duration = Duration::from_mins(2);

/// Runs a job gets before the ledger stops offering it, counting the turn's
/// own spawn. Three is two retries: an extraction that dies three times is
/// failing on its content, not on the instance.
const MAX_ATTEMPTS: i64 = 3;

/// Rows one sweep takes. Bounded by the extraction permits — a sweep that
/// claimed more than fit in flight would hold rows it could not start
/// before their lease lapsed.
const SWEEP_BATCH: i64 = 20;

/// Claim and re-run every extraction whose spawn is presumed dead.
///
/// Returns how many extractions this pass ran to the end. A claimed row
/// whose run fails is left leased for the next sweep after its lease; a row
/// whose payload does not parse is finished, with a warning, because no
/// sweep could ever run it.
///
/// The claimed rows run concurrently: each is one LLM call, and the process-
/// wide permit bound in [`run_extraction_job`] is what keeps a full batch
/// from racing the provider. Sequential runs would let a slow provider walk
/// the batch past its own lease.
///
/// # Errors
///
/// The ledger could not be claimed against. A failed run is logged, never
/// propagated: one bad extraction does not stop the rest of the pass.
pub async fn resume_extraction_jobs(
    repos: &RepositoryRegistry,
    chat_provider: &ChatProvider,
    dedup: DedupConfig,
    system_prompt: &str,
) -> AppResult<usize> {
    let now_ms = Utc::now().timestamp_millis();
    let claim = ExtractionJobClaim {
        now_ms,
        queued_older_than_ms: millis(QUEUED_GRACE),
        lease_ms: millis(EXTRACTION_JOB_LEASE),
        max_attempts: MAX_ATTEMPTS,
        limit: SWEEP_BATCH,
    };
    reap_exhausted_jobs(repos, now_ms).await;
    let claimed = repos
        .memory_extraction_jobs
        .claim_stale_extraction_jobs(claim)
        .await?;
    let runs = claimed
        .into_iter()
        .map(|row| resume_one(repos, chat_provider, dedup, system_prompt, row));
    Ok(join_all(runs).await.into_iter().filter(|ran| *ran).count())
}

/// Drop the rows past the attempt cap whose last runner died too: nothing
/// claims them again, so they would only ever accumulate.
async fn reap_exhausted_jobs(repos: &RepositoryRegistry, now_ms: i64) {
    match repos
        .memory_extraction_jobs
        .reap_exhausted_extraction_jobs(now_ms, MAX_ATTEMPTS)
        .await
    {
        Ok(0) => {}
        Ok(reaped) => warn!(
            reaped,
            "memory extraction resume: dropped jobs past the attempt cap whose last runner died; those turns' facts are lost"
        ),
        Err(e) => {
            warn!(error = %e, "memory extraction resume: exhausted jobs could not be reaped");
        }
    }
}

/// Run one claimed row; `true` when the extraction landed.
async fn resume_one(
    repos: &RepositoryRegistry,
    chat_provider: &ChatProvider,
    dedup: DedupConfig,
    system_prompt: &str,
    row: MemoryExtractionJobRow,
) -> bool {
    let Some(payload) = parse_payload(&row) else {
        finish(repos, &row.id).await;
        return false;
    };
    info!(
        job_id = %row.id,
        attempts = row.attempts,
        "resuming a memory extraction its instance left behind"
    );
    let run = run_extraction_job(
        repos.memory.as_ref(),
        chat_provider,
        dedup,
        system_prompt,
        row.tenant_id,
        &payload,
    )
    .await;
    match run {
        Ok(()) => {
            finish(repos, &row.id).await;
            true
        }
        Err(e) => {
            warn!(
                job_id = %row.id,
                attempts = row.attempts,
                error = %e,
                "resumed memory extraction failed; its row waits for the lease to lapse"
            );
            false
        }
    }
}

/// The payload a row carries, or `None` — logged — when it cannot be read.
fn parse_payload(row: &MemoryExtractionJobRow) -> Option<ExtractionJobPayload> {
    match serde_json::from_str(&row.payload) {
        Ok(payload) => Some(payload),
        Err(e) => {
            warn!(
                job_id = %row.id,
                error = %e,
                "recorded extraction payload does not parse; dropping the job, nothing could ever run it"
            );
            None
        }
    }
}

/// Delete a row nothing is owed on any more.
async fn finish(repos: &RepositoryRegistry, job_id: &str) {
    if let Err(e) = repos
        .memory_extraction_jobs
        .finish_extraction_job(job_id)
        .await
    {
        warn!(job_id, error = %e, "extraction job could not be finished; the sweep will see it again after its lease");
    }
}

/// Start the resume sweep for the life of the process.
///
/// One pass every [`RESUME_INTERVAL`], scheduled through the worker ledger so
/// a fresh instance picks up where the last one stopped rather than waiting a
/// full interval. The de-dup cap is read from the harness config on every
/// pass, exactly as a turn reads it, so a change through contremaitre reaches
/// the next resumed extraction without a deploy.
///
/// Without a chat provider nothing could run an extraction, so no sweep is
/// started: the rows would be claimed, fail, and burn their attempts for
/// nothing. The turn path finishes its own rows in that configuration.
pub fn start_memory_extraction_resume(
    repos: Arc<RepositoryRegistry>,
    chat_provider: Option<Arc<ChatProvider>>,
    harness_config: Arc<HarnessConfigRegistry>,
    system_prompt: String,
) {
    let Some(chat_provider) = chat_provider else {
        info!("memory extraction resume not started: no chat provider is wired, so no owed extraction could run");
        return;
    };
    let ledger = Arc::clone(&repos.worker_runs);
    spawn_periodic(
        "memory extraction resume",
        RESUME_INTERVAL,
        ledger,
        move || {
            let repos = Arc::clone(&repos);
            let chat_provider = Arc::clone(&chat_provider);
            let harness_config = Arc::clone(&harness_config);
            let system_prompt = system_prompt.clone();
            async move {
                let dedup = DedupConfig {
                    candidate_limit: usize::try_from(
                        harness_config.current_memory().dedup_candidate_limit,
                    )
                    .unwrap_or(usize::MAX),
                };
                let ran =
                    resume_extraction_jobs(&repos, &chat_provider, dedup, &system_prompt).await?;
                if ran > 0 {
                    info!(
                        ran,
                        "resume sweep ran the memory extractions turns still owed"
                    );
                }
                Ok(())
            }
        },
    );
}

/// A duration as the ledger's unit, saturating rather than wrapping.
fn millis(duration: Duration) -> i64 {
    i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
}
