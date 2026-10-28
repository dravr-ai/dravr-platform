// ABOUTME: The athlete's import operations: check a link, start, follow, cancel, read the report, delete the history
// ABOUTME: And run_step, the one step a Cloud Tasks delivery or the in-process runner executes, naming the next

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Import operations
//!
//! Every operation acts in the athlete's [`Scope`] (their tenant and account)
//! and on one source; only `strava_archive` is offered. One import per source
//! runs at a time: a second start answers a conflict while the first is not
//! final (barrage's partial unique index).
//!
//! An athlete who has asked the service for their export but not yet received
//! the link has an import *waiting for its link* (`awaiting_link`), which
//! expires after [`AWAITING_LINK_DAYS`]. Pasting the link cancels that
//! placeholder and starts the import with the link, in one request.

use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use dravr_barrage::store::{ImportIssue, ImportJob, NewImportJob, PageReq};
use dravr_barrage::{
    ActivityId, BarrageError, FailureCode, ImportId, Importer, JobStatus, Scope, SourceKey,
    StepOutcome,
};
use futures_util::stream;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use pierre_providers::fit_decode::ImportDecodeGate;
use serde::Serialize;
use tracing::{info, warn};
use uuid::Uuid;

use super::fetch::LinkRefusal;
use super::link::{link_digest, open, seal};
use super::runner::StepTask;
use super::sink::CachedActivitiesSink;
use super::ImportWiring;
use crate::mcp::resources::ServerContext;

/// How long an import waits for its link before the expiry sweep cancels it.
pub const AWAITING_LINK_DAYS: i64 = 14;

/// The sources an athlete may import today.
pub const OFFERED_SOURCES: &[SourceKey] = &[SourceKey::StravaArchive];

/// One import as the athlete's screens read it. The link is never part of it.
#[derive(Debug, Clone, Serialize)]
pub struct ImportView {
    /// Its id.
    pub id: Uuid,
    /// The source it imports (`strava_archive`).
    pub source: &'static str,
    /// Where it is: `awaiting_link`, `queued`, `downloading`, `importing`,
    /// `completed`, `failed` or `cancelled`.
    pub status: &'static str,
    /// Rows in the export's activity list, once downloaded.
    pub total_rows: Option<u64>,
    /// Rows read so far.
    pub processed_rows: u64,
    /// Activities stored or changed.
    pub imported: u64,
    /// Rows already held unchanged, or skipped by rule.
    pub skipped: u64,
    /// Rows that could not be imported; the report lists them.
    pub failed: u64,
    /// Why it failed, as a stable code.
    pub error_code: Option<&'static str>,
    /// The earliest activity start imported.
    pub earliest_start: Option<DateTime<Utc>>,
    /// The latest activity start imported.
    pub latest_start: Option<DateTime<Utc>>,
    /// When it was created.
    pub created_at: DateTime<Utc>,
    /// When it last changed.
    pub updated_at: DateTime<Utc>,
    /// When it reached a final state.
    pub finished_at: Option<DateTime<Utc>>,
    /// When a job waiting for its link is cancelled.
    pub expires_at: Option<DateTime<Utc>>,
    /// Whether the athlete still holds activities imported from the source:
    /// `false` once they deleted the history a completed import brought in.
    pub history_held: bool,
}

impl ImportView {
    /// The view of `job`, given whether the athlete still holds its source's history.
    fn of(job: &ImportJob, history_held: bool) -> Self {
        Self {
            id: job.id.0,
            source: job.source.as_str(),
            status: job.status.as_str(),
            total_rows: job.total_rows,
            processed_rows: job.processed_rows,
            imported: job.imported,
            skipped: job.skipped,
            failed: job.failed,
            error_code: job.error_code.map(FailureCode::as_str),
            earliest_start: job.earliest_start,
            latest_start: job.latest_start,
            created_at: job.created_at,
            updated_at: job.updated_at,
            finished_at: job.finished_at,
            expires_at: job.expires_at,
            history_held,
        }
    }
}

/// What checking a pasted link found, before any import is started.
#[derive(Debug, Clone, Serialize)]
pub struct LinkCheckView {
    /// Whether an import can start from it.
    pub accepted: bool,
    /// Why not, as a stable code (`link_invalid`, `link_host_not_allowed`,
    /// `link_expired`, `link_unreachable`).
    pub refusal: Option<&'static str>,
    /// The export's size, when the service says.
    pub size_bytes: Option<u64>,
}

/// One row of an import's report.
#[derive(Debug, Clone, Serialize)]
pub struct IssueView {
    /// Its line in the export's activity list.
    pub row: u64,
    /// The activity id the row gives.
    pub activity_id: Option<String>,
    /// Why it was not imported.
    pub outcome: &'static str,
    /// What was found.
    pub detail: Option<String>,
}

impl From<&ImportIssue> for IssueView {
    fn from(issue: &ImportIssue) -> Self {
        Self {
            row: issue.csv_row,
            activity_id: issue.activity_id.clone(),
            outcome: issue.outcome.as_str(),
            detail: issue.detail.clone(),
        }
    }
}

/// A page of an import's report.
#[derive(Debug, Clone, Serialize)]
pub struct ReportPage {
    /// The rows not imported.
    pub issues: Vec<IssueView>,
    /// Where the next page starts; `None` on the last.
    pub next_offset: Option<u64>,
}

/// The athlete's scope: their tenant and account.
#[must_use]
pub fn scope_of(tenant_id: TenantId, user_id: Uuid) -> Scope {
    Scope::new(tenant_id.as_uuid(), user_id)
}

/// The offered source `key` names.
///
/// # Errors
///
/// [`AppError::not_found`] for any key that is not an offered source.
pub fn offered_source(key: &str) -> AppResult<SourceKey> {
    SourceKey::parse(key)
        .ok()
        .filter(|source| OFFERED_SOURCES.contains(source))
        .ok_or_else(|| AppError::not_found(format!("import source {key}")))
}

/// The wiring imports run on here.
fn wiring(resources: &ServerContext) -> AppResult<&Arc<ImportWiring>> {
    resources.common.archive_imports.wiring()
}

/// An importer over the wiring's stores, projecting into `resources`, each
/// recording it decodes admitted by the process's FIT decode permits.
pub(super) fn importer(resources: &Arc<ServerContext>, wiring: &ImportWiring) -> Importer {
    Importer::new(
        wiring.stores.clone(),
        Arc::new(CachedActivitiesSink::new(Arc::clone(resources))),
    )
    .with_decode_gate(Arc::new(ImportDecodeGate))
}

/// Barrage's error as the platform's.
fn barrage(error: BarrageError) -> AppError {
    match error {
        BarrageError::ImportInProgress(source) => {
            AppError::already_exists(format!("an import of {source} in progress"))
        }
        BarrageError::Invalid { what, why } => AppError::invalid_input(format!("{what}: {why}")),
        other => AppError::internal(format!("import store: {other}")),
    }
}

/// Check that an import can start from `link`, reading none of the export.
///
/// # Errors
///
/// When imports are not available here.
pub async fn check_link(resources: &ServerContext, link: &str) -> AppResult<LinkCheckView> {
    let wiring = wiring(resources)?;
    Ok(match wiring.fetcher.check(link.trim()).await {
        Ok(check) => LinkCheckView {
            accepted: true,
            refusal: None,
            size_bytes: check.size_bytes,
        },
        Err(refusal) => LinkCheckView {
            accepted: false,
            refusal: Some(refusal.code()),
            size_bytes: None,
        },
    })
}

/// Start an import of `source` for `scope`: one waiting for its link when
/// `link` is `None`, else one that downloads it now, replacing a placeholder
/// that was waiting for it.
///
/// # Errors
///
/// - [`AppError::already_exists`] while another import of the source runs;
/// - [`AppError::invalid_input`] naming the refusal's code when the link is
///   refused;
/// - a store or queue error.
pub async fn start(
    resources: &Arc<ServerContext>,
    scope: &Scope,
    source: SourceKey,
    link: Option<&str>,
) -> AppResult<ImportView> {
    let wiring = wiring(resources)?;
    let importer = importer(resources, wiring);
    let Some(link) = link.map(str::trim).filter(|link| !link.is_empty()) else {
        let job = importer
            .create(
                scope,
                NewImportJob {
                    source,
                    status: JobStatus::AwaitingLink,
                    url_sha256: None,
                    url_ciphertext: None,
                    expires_at: Some(Utc::now() + Duration::days(AWAITING_LINK_DAYS)),
                },
            )
            .await
            .map_err(barrage)?;
        return Ok(ImportView::of(&job, false));
    };
    if let Err(refusal) = wiring.fetcher.check(link).await {
        return Err(refused(refusal));
    }
    let running = wiring
        .stores
        .jobs
        .current_all(scope)
        .await
        .map_err(barrage)?;
    for job in running {
        if job.source == source && job.status == JobStatus::AwaitingLink {
            importer.cancel(scope, job.id).await.map_err(barrage)?;
        }
    }
    let ciphertext = seal(
        resources.common.repos.security.as_ref(),
        scope,
        source,
        link,
    )?;
    let job = importer
        .create(
            scope,
            NewImportJob {
                source,
                status: JobStatus::Queued,
                url_sha256: Some(link_digest(link)),
                url_ciphertext: Some(ciphertext),
                expires_at: None,
            },
        )
        .await
        .map_err(barrage)?;
    let task = StepTask {
        import_id: job.id.0,
        tenant_id: scope.tenant_id.0,
        user_id: scope.user_id.0,
        expect_cursor: None,
    };
    wiring.runner.schedule(resources, task).await?;
    info!(import = %job.id, source = %source, link_sha256 = %link_digest(link).to_hex(), "archive import started");
    Ok(ImportView::of(&job, false))
}

/// A link refusal as the error a start answers with: its code leads the
/// message, for the client to read.
fn refused(refusal: LinkRefusal) -> AppError {
    AppError::invalid_input(format!("{}: the export link was refused", refusal.code()))
}

/// The import of each offered source the athlete has: the one running, else
/// the latest.
///
/// # Errors
///
/// When imports are not available here, or a store fails.
pub async fn latest_per_source(
    resources: &ServerContext,
    scope: &Scope,
) -> AppResult<Vec<ImportView>> {
    let wiring = wiring(resources)?;
    let page = wiring
        .stores
        .jobs
        .list(scope, PageReq::first(50))
        .await
        .map_err(barrage)?;
    let held = resources
        .common
        .repos
        .activity_cache
        .held_user_provided(scope.user_id.0, &TenantId::from_uuid(scope.tenant_id.0))
        .await?;
    let mut latest = Vec::new();
    for source in OFFERED_SOURCES {
        let jobs = page.items.iter().filter(|job| job.source == *source);
        let current = jobs.clone().find(|job| !job.status.is_final());
        if let Some(job) = current.or_else(|| page.items.iter().find(|job| job.source == *source)) {
            let history_held = held.iter().any(|key| key == source.as_str());
            latest.push(ImportView::of(job, history_held));
        }
    }
    Ok(latest)
}

/// The import of `source` the athlete has: the one running, else the latest.
///
/// # Errors
///
/// As [`latest_per_source`].
pub async fn latest(
    resources: &ServerContext,
    scope: &Scope,
    source: SourceKey,
) -> AppResult<Option<ImportView>> {
    Ok(latest_per_source(resources, scope)
        .await?
        .into_iter()
        .find(|view| view.source == source.as_str()))
}

/// The job `id` of `source` in `scope`.
async fn job_of(
    wiring: &ImportWiring,
    scope: &Scope,
    source: SourceKey,
    id: Uuid,
) -> AppResult<ImportJob> {
    wiring
        .stores
        .jobs
        .get(scope, ImportId(id))
        .await
        .map_err(barrage)?
        .filter(|job| job.source == source)
        .ok_or_else(|| AppError::not_found(format!("import {id}")))
}

/// A page of the rows import `id` did not import.
///
/// # Errors
///
/// [`AppError::not_found`] for an import that is not the athlete's.
pub async fn report(
    resources: &ServerContext,
    scope: &Scope,
    source: SourceKey,
    id: Uuid,
    page: PageReq,
) -> AppResult<ReportPage> {
    let wiring = wiring(resources)?;
    job_of(wiring, scope, source, id).await?;
    let issues = wiring
        .stores
        .jobs
        .issues(scope, ImportId(id), page)
        .await
        .map_err(barrage)?;
    Ok(ReportPage {
        issues: issues.items.iter().map(IssueView::from).collect(),
        next_offset: issues.next_offset,
    })
}

/// Cancel import `id`; whether it was still running.
///
/// # Errors
///
/// [`AppError::not_found`] for an import that is not the athlete's.
pub async fn cancel(
    resources: &Arc<ServerContext>,
    scope: &Scope,
    source: SourceKey,
    id: Uuid,
) -> AppResult<bool> {
    let wiring = wiring(resources)?;
    job_of(wiring, scope, source, id).await?;
    importer(resources, wiring)
        .cancel(scope, ImportId(id))
        .await
        .map_err(barrage)
}

/// Delete everything the athlete imported from `source`; how many originals went.
///
/// A running import is cancelled first, so no later step writes again, then
/// the projection, the records and the kept originals go.
///
/// # Errors
///
/// When imports are not available here, or a store fails.
pub async fn delete_history(
    resources: &Arc<ServerContext>,
    scope: &Scope,
    source: SourceKey,
) -> AppResult<u64> {
    let wiring = wiring(resources)?;
    let importer = importer(resources, wiring);
    let running = wiring
        .stores
        .jobs
        .current_all(scope)
        .await
        .map_err(barrage)?;
    for job in running {
        if job.source == source {
            importer.cancel(scope, job.id).await.map_err(barrage)?;
        }
    }
    let originals = importer
        .delete_source(scope, source)
        .await
        .map_err(barrage)?;
    info!(user_id = %scope.user_id, source = %source, originals, "imported history deleted");
    Ok(originals)
}

/// Delete one imported activity, its samples and, when no other activity
/// keeps it, its original. Whether one was deleted.
///
/// # Errors
///
/// When imports are not available here, the id is not one, or a store fails.
pub async fn delete_activity(
    resources: &Arc<ServerContext>,
    scope: &Scope,
    source: SourceKey,
    activity_id: &str,
) -> AppResult<bool> {
    let wiring = wiring(resources)?;
    let id = ActivityId::parse(activity_id).map_err(barrage)?;
    importer(resources, wiring)
        .delete_one(scope, source, &id)
        .await
        .map_err(barrage)
}

/// Run one step of an import, and name the step that follows it.
///
/// The download step decrypts the link, streams the export through the
/// guarded fetcher into barrage's spool and readies the first chunk; a chunk
/// step imports up to `dravr_barrage::import::CHUNK_ROWS` rows. A step that finds the import
/// elsewhere than it expected (a duplicate delivery, a retry after the
/// commit) commits nothing and names the step the import is actually at, so
/// a lost enqueue is repaired by any later delivery; a final import names
/// none.
///
/// # Errors
///
/// A store, projection or queue failure: the step is retried, and every
/// write before its commit is idempotent.
pub async fn run_step(
    resources: &Arc<ServerContext>,
    task: &StepTask,
) -> AppResult<Option<StepTask>> {
    let wiring = wiring(resources)?;
    let scope = Scope::new(task.tenant_id, task.user_id);
    let id = ImportId(task.import_id);
    let importer = importer(resources, wiring);
    let outcome = match task.expect_cursor {
        None => download(resources, wiring, &importer, &scope, id).await?,
        Some(cursor) => importer.step(&scope, id, cursor).await.map_err(barrage)?,
    };
    let next = match outcome {
        StepOutcome::Next { expect_cursor } => Some(expect_cursor),
        StepOutcome::Finished(status) => {
            info!(import = %id, status = status.as_str(), "archive import finished");
            None
        }
        StepOutcome::Stale => resume_cursor(wiring, &scope, id).await?,
    };
    Ok(next.map(|cursor| StepTask {
        expect_cursor: Some(cursor),
        ..*task
    }))
}

/// The download step of import `id`.
async fn download(
    resources: &ServerContext,
    wiring: &ImportWiring,
    importer: &Importer,
    scope: &Scope,
    id: ImportId,
) -> AppResult<StepOutcome> {
    use futures_util::StreamExt;

    let Some(job) = wiring.stores.jobs.get(scope, id).await.map_err(barrage)? else {
        return Ok(StepOutcome::Stale);
    };
    if job.status != JobStatus::Queued {
        return Ok(StepOutcome::Stale);
    }
    let link = match (&job.url_sha256, &job.url_ciphertext) {
        (Some(digest), Some(ciphertext)) => open(
            resources.common.repos.security.as_ref(),
            scope,
            job.source,
            digest,
            ciphertext,
        ),
        _ => Err(AppError::internal("the import holds no link")),
    };
    let body = match link {
        Ok(link) => wiring.fetcher.download(&link).await,
        Err(e) => {
            warn!(import = %id, error = %e, "the import's link could not be read; the import fails");
            stream::once(async {
                Err(BarrageError::archive(
                    FailureCode::DownloadFailed,
                    "the export link could not be read",
                ))
            })
            .boxed()
        }
    };
    importer.download(scope, id, body).await.map_err(barrage)
}

/// The cursor import `id` is at when it is importing: the step a stale
/// delivery hands on, so an enqueue lost after a commit is repaired.
async fn resume_cursor(
    wiring: &ImportWiring,
    scope: &Scope,
    id: ImportId,
) -> AppResult<Option<u64>> {
    let job = wiring.stores.jobs.get(scope, id).await.map_err(barrage)?;
    Ok(job
        .filter(|job| job.status == JobStatus::Importing)
        .map(|job| job.cursor_row))
}
