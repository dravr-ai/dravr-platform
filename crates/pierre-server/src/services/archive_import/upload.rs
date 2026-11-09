// ABOUTME: An export the athlete uploads themselves: open the store's session, read it back, write through the API, finish
// ABOUTME: The session URL is a bearer credential for the write: sealed on the job, served only to its owner, in no log or error

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Uploading an export
//!
//! Some services hand the athlete a file rather than a link: an Apple Health
//! export is saved from the phone. Such a source's import is a job waiting in
//! `awaiting_link` that holds its spool key from the start, so it takes the
//! source's one active slot and expires like any other job that waits:
//!
//! 1. [`start_upload`] asks the blob store where the bytes go. On GCS that is
//!    a resumable session the client writes to directly, because a
//!    multi-gigabyte upload from a phone must resume and must not pass through
//!    an API instance; the session's URL is sealed onto the job, as a link is
//!    ([`super::link`]). A store that takes no direct writes (a local
//!    directory, memory) answers [`UploadView::ThroughApi`], and the client
//!    sends the whole file to [`write_through_api`] instead.
//! 2. [`session_of`] serves that target back to the job's owner, which is how a
//!    restarted client resumes.
//! 3. [`finish_upload`] checks the object landed within the cap, since a client
//!    writing to the store bypasses every running count here, and schedules
//!    the step that adopts it ([`super::ops::run_step`]). Reading the export
//!    is a whole pass over it, so it never runs inside this request. Asked
//!    again it schedules nothing while that step is queued, and a new step once
//!    the queue has given up on it ([`super::runner::ImportTasks::enqueue`]).
//!
//! An upload abandoned at any point is deleted by the expiry sweep
//! ([`super::sweeps`]), because a live session can still complete after its
//! job ended.

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use axum::http::Uri;
use bytes::Bytes;
use chrono::{Duration, Utc};
use dravr_barrage::blob::{BlobKey, BlobMeta, UploadTarget};
use dravr_barrage::import::archive::ArchiveLimits;
use dravr_barrage::store::{ImportJob, NewImportJob};
use dravr_barrage::{BarrageError, FailureCode, ImportId, JobStatus, Scope, SourceKey};
use futures_util::stream::BoxStream;
use futures_util::StreamExt;
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use serde::Serialize;
use tracing::info;
use uuid::Uuid;

use super::link::{link_digest, open, seal};
use super::ops::{barrage, importer, job_of, wiring, ImportView};
use super::runner::StepTask;
use crate::mcp::resources::ServerContext;

/// The sources whose export the athlete uploads rather than links.
pub const UPLOAD_SOURCES: &[SourceKey] = &[
    SourceKey::AppleHealthArchive,
    SourceKey::CorosArchive,
    SourceKey::WhoopArchive,
];

/// How long an upload may wait before the expiry sweep cancels it: inside the
/// week a resumable session lives per GCS's documentation, so a job never
/// offers a session that has already gone.
const UPLOAD_WAIT_DAYS: i64 = 6;

/// The media type every upload is stored under: an export is a ZIP archive.
const UPLOAD_CONTENT_TYPE: &str = "application/zip";

/// Whether `source`'s export is uploaded rather than linked.
#[must_use]
pub fn takes_upload(source: SourceKey) -> bool {
    UPLOAD_SOURCES.contains(&source)
}

/// The largest export an upload may hold: the cap every import's spool has.
#[must_use]
pub fn max_upload_bytes() -> u64 {
    ArchiveLimits::default().max_archive_bytes
}

/// Where the client sends an upload's bytes.
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UploadView {
    /// Straight to the store, at this resumable session.
    Session {
        /// The session's URL, a bearer credential for the write.
        url: String,
    },
    /// To `PUT /api/imports/{source}/{id}/upload`, whole.
    ThroughApi,
}

impl fmt::Debug for UploadView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Session { .. } => "Session(..)",
            Self::ThroughApi => "ThroughApi",
        })
    }
}

/// An upload just opened: the import that waits for it, and where its bytes go.
#[derive(Debug, Clone, Serialize)]
pub struct StartedUpload {
    /// The import, as every other route serves it.
    #[serde(flatten)]
    pub import: ImportView,
    /// Where the client sends the bytes.
    pub upload: UploadView,
}

/// What finishing an upload found.
#[derive(Debug, Clone)]
pub enum FinishedUpload {
    /// The object landed and the step that reads it is scheduled.
    Started(ImportView),
    /// The object was over the cap: it is deleted and the import has failed.
    TooLarge,
}

/// The `Origin` a session may be opened for, from the request's header.
///
/// A web client names its origin, and the store answers that origin's
/// cross-origin requests, so it must be a bare `scheme://host[:port]`, on a
/// deployment's own list when the deployment keeps one. A native client names
/// none.
///
/// # Errors
///
/// [`ErrorCode::PermissionDenied`] for an origin refused by either rule.
pub fn upload_origin(header: Option<&str>, allowed: &str) -> AppResult<Option<String>> {
    let Some(origin) = header else {
        return Ok(None);
    };
    let refused = || AppError::new(ErrorCode::PermissionDenied, "This origin may not upload");
    let bare = origin.parse::<Uri>().is_ok_and(|uri| {
        matches!(uri.scheme_str(), Some("http" | "https"))
            && uri.authority().is_some_and(|a| !a.as_str().contains('@'))
            && uri.path_and_query().is_none_or(|p| p.as_str() == "/")
            && !origin.ends_with('/')
            && !origin.contains(['?', '#'])
    });
    if !bare {
        return Err(refused());
    }
    let allowed = allowed.trim();
    let listed = allowed.is_empty()
        || allowed == "*"
        || allowed.split(',').any(|entry| entry.trim() == origin);
    if listed {
        Ok(Some(origin.to_owned()))
    } else {
        Err(refused())
    }
}

/// Open an upload of `length` bytes for `scope`'s import of `source`.
///
/// # Errors
///
/// - [`AppError::invalid_input`] for an empty upload;
/// - [`AppError::already_exists`] while another import of the source runs,
///   before anything is opened in the store;
/// - a store failure.
pub async fn start_upload(
    resources: &ServerContext,
    scope: &Scope,
    source: SourceKey,
    length: u64,
    origin: Option<&str>,
) -> AppResult<StartedUpload> {
    if length == 0 {
        return Err(AppError::invalid_input("An upload holds at least one byte"));
    }
    let wiring = wiring(resources)?;
    let running = wiring
        .stores
        .jobs
        .current_all(scope)
        .await
        .map_err(barrage)?;
    if running.iter().any(|job| job.source == source) {
        return Err(barrage(BarrageError::ImportInProgress(source.to_string())));
    }
    let spool = BlobKey::spool(*scope);
    let target = wiring
        .stores
        .blobs
        .open_upload(&spool, &BlobMeta::new(UPLOAD_CONTENT_TYPE), length, origin)
        .await
        .map_err(barrage)?;
    let (url_sha256, url_ciphertext, upload) = match target {
        UploadTarget::Session(url) => {
            let sealed = seal(
                resources.common.repos.security.as_ref(),
                scope,
                source,
                &url,
            )?;
            (
                Some(link_digest(&url)),
                Some(sealed),
                UploadView::Session { url },
            )
        }
        UploadTarget::ThroughApi => (None, None, UploadView::ThroughApi),
    };
    let job = wiring
        .stores
        .jobs
        .create(
            scope,
            NewImportJob {
                source,
                status: JobStatus::AwaitingLink,
                url_sha256,
                url_ciphertext,
                spool: Some(spool),
                expires_at: Some(Utc::now() + Duration::days(UPLOAD_WAIT_DAYS)),
            },
        )
        .await
        .map_err(barrage)?;
    info!(import = %job.id, source = %source, length, "upload opened");
    Ok(StartedUpload {
        import: ImportView::of(&job, false),
        upload,
    })
}

/// Where the upload of import `id` sends its bytes, for a client resuming it.
///
/// # Errors
///
/// [`AppError::not_found`] for an import that is not the athlete's, and
/// [`ErrorCode::ResourceLocked`] for one that does not wait for an upload.
pub async fn session_of(
    resources: &ServerContext,
    scope: &Scope,
    source: SourceKey,
    id: Uuid,
) -> AppResult<UploadView> {
    let wiring = wiring(resources)?;
    let job = job_of(wiring, scope, source, id).await?;
    waiting_spool(&job)?;
    match (&job.url_sha256, &job.url_ciphertext) {
        (Some(digest), Some(ciphertext)) => Ok(UploadView::Session {
            url: open(
                resources.common.repos.security.as_ref(),
                scope,
                source,
                digest,
                ciphertext,
            )?,
        }),
        _ => Ok(UploadView::ThroughApi),
    }
}

/// Write the whole upload of import `id` from `body`, on a store that takes no
/// direct writes; `false` when it ran past the cap and nothing was stored.
///
/// # Errors
///
/// - [`AppError::not_found`] for an import that is not the athlete's;
/// - [`ErrorCode::ResourceLocked`] for one that does not wait for an upload,
///   writes straight to the store, or already holds its bytes;
/// - [`AppError::invalid_input`] when the body broke off;
/// - a store failure.
pub async fn write_through_api(
    resources: &ServerContext,
    scope: &Scope,
    source: SourceKey,
    id: Uuid,
    body: BoxStream<'_, Result<Bytes, BarrageError>>,
) -> AppResult<bool> {
    let wiring = wiring(resources)?;
    let job = job_of(wiring, scope, source, id).await?;
    let spool = waiting_spool(&job)?;
    if job.url_ciphertext.is_some() {
        return Err(locked("This upload writes straight to the store"));
    }
    if wiring
        .stores
        .blobs
        .size(&spool)
        .await
        .map_err(barrage)?
        .is_some()
    {
        return Err(locked("This upload already holds its file"));
    }
    let over = Arc::new(AtomicBool::new(false));
    let capped = capped(body, max_upload_bytes(), Arc::clone(&over));
    let written = wiring
        .stores
        .blobs
        .put_stream(&spool, capped, &BlobMeta::new(UPLOAD_CONTENT_TYPE))
        .await;
    match written {
        Ok(_) => Ok(true),
        Err(_) if over.load(Ordering::SeqCst) => Ok(false),
        Err(BarrageError::Archive { .. }) => Err(AppError::invalid_input(
            "The upload broke off; send it again",
        )),
        Err(other) => Err(barrage(other)),
    }
}

/// `body`, failing as soon as it passes `max` bytes and setting `over` when it
/// does; any other failure of the body fails as an archive error.
fn capped(
    body: BoxStream<'_, Result<Bytes, BarrageError>>,
    max: u64,
    over: Arc<AtomicBool>,
) -> BoxStream<'_, Result<Bytes, BarrageError>> {
    let mut seen = 0_u64;
    body.map(move |chunk| {
        let chunk = chunk.map_err(|e| {
            BarrageError::archive(FailureCode::DownloadFailed, format!("upload body: {e}"))
        })?;
        seen = seen.saturating_add(chunk.len() as u64);
        if seen > max {
            over.store(true, Ordering::SeqCst);
            return Err(BarrageError::archive(
                FailureCode::ArchiveTooLarge,
                "the upload is larger than an export may be",
            ));
        }
        Ok(chunk)
    })
    .boxed()
}

/// Finish the upload of import `id`: refuse it over the cap, else schedule the
/// step that reads it.
///
/// # Errors
///
/// [`AppError::not_found`] for an import that is not the athlete's, and
/// [`ErrorCode::ResourceLocked`] for one that does not wait for an upload or
/// whose object has not landed: the session stays usable.
pub async fn finish_upload(
    resources: &Arc<ServerContext>,
    scope: &Scope,
    source: SourceKey,
    id: Uuid,
) -> AppResult<FinishedUpload> {
    let wiring = wiring(resources)?;
    let job = job_of(wiring, scope, source, id).await?;
    let spool = waiting_spool(&job)?;
    let Some(size) = wiring.stores.blobs.size(&spool).await.map_err(barrage)? else {
        return Err(locked("The upload has not finished"));
    };
    if size > max_upload_bytes() {
        // No pass runs here: the importer refuses this size before reading.
        importer(resources, wiring)
            .adopt_upload(scope, ImportId(id))
            .await
            .map_err(barrage)?;
        return Ok(FinishedUpload::TooLarge);
    }
    let task = StepTask {
        import_id: id,
        tenant_id: scope.tenant_id.0,
        user_id: scope.user_id.0,
        expect_cursor: None,
    };
    wiring.runner.schedule(resources, task).await?;
    info!(import = %job.id, source = %source, size, "upload finished");
    Ok(FinishedUpload::Started(ImportView::of(&job, false)))
}

/// The spool `job` waits to receive.
fn waiting_spool(job: &ImportJob) -> AppResult<BlobKey> {
    match (job.status, job.spool) {
        (JobStatus::AwaitingLink, Some(spool)) => Ok(spool),
        _ => Err(locked("This import does not wait for an upload")),
    }
}

/// The refusal of a request this import is not in the state to take.
fn locked(why: &str) -> AppError {
    AppError::new(ErrorCode::ResourceLocked, why)
}

#[cfg(test)]
mod tests {
    use futures_util::stream;

    use super::*;

    #[test]
    fn a_bare_origin_is_accepted_and_anything_else_is_refused() {
        let good = "https://app.dravr.ai";
        assert_eq!(upload_origin(None, "*").unwrap(), None, "a native client");
        assert_eq!(
            upload_origin(Some(good), "*").unwrap().as_deref(),
            Some(good)
        );
        assert_eq!(
            upload_origin(Some("http://localhost:3000"), "")
                .unwrap()
                .as_deref(),
            Some("http://localhost:3000"),
        );
        for bad in [
            "",
            "null",
            "app.dravr.ai",
            "ftp://app.dravr.ai",
            "https://app.dravr.ai/",
            "https://app.dravr.ai/path",
            "https://app.dravr.ai?x=1",
            "https://app.dravr.ai#fragment",
            "https://user:pw@app.dravr.ai",
            "https://app.dravr.ai\r\nx-evil: 1",
        ] {
            assert!(upload_origin(Some(bad), "*").is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_kept_list_refuses_an_origin_off_it() {
        let list = "https://a.example.com, https://b.example.com";
        assert!(upload_origin(Some("https://b.example.com"), list).is_ok());
        assert!(upload_origin(Some("https://c.example.com"), list).is_err());
    }

    #[test]
    fn the_session_url_never_prints() {
        let view = UploadView::Session {
            url: "https://storage.example.com/session-secret".to_owned(),
        };
        assert_eq!(format!("{view:?}"), "Session(..)");
    }

    #[tokio::test]
    async fn a_body_past_the_cap_fails_and_says_so() {
        let over = Arc::new(AtomicBool::new(false));
        let chunks = stream::iter([
            Ok(Bytes::from_static(b"12345")),
            Ok(Bytes::from_static(b"678")),
            Ok(Bytes::from_static(b"9")),
        ])
        .boxed();
        let read: Vec<_> = capped(chunks, 6, Arc::clone(&over)).collect().await;
        assert!(read[0].is_ok(), "within the cap");
        assert!(read[1].is_err(), "past the cap");
        assert!(over.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn a_broken_body_is_not_called_too_large() {
        let over = Arc::new(AtomicBool::new(false));
        let chunks = stream::iter([Err(BarrageError::Blob("gone".to_owned()))]).boxed();
        let read: Vec<_> = capped(chunks, 6, Arc::clone(&over)).collect().await;
        assert!(matches!(read[0], Err(BarrageError::Archive { .. })));
        assert!(!over.load(Ordering::SeqCst));
    }
}
