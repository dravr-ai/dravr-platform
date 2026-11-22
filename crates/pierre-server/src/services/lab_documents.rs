// ABOUTME: An uploaded lab report read into pending FHIR Observations: PDF text, barrage's lab reader, the athlete's review
// ABOUTME: The PDF is kept by the originals store; the model reads only where the tier rule lets a model read T2 (carnet#1544)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Lab documents
//!
//! An athlete uploads a lab report as a PDF. It is kept as every file they
//! upload is ([`OriginalsStore::put_named_file`], under the upload key its
//! SHA-256 names), its text is extracted page by page with `pdf-extract`
//! (whose line layout barrage's rules read; `lopdf`'s own text loses it), and
//! barrage's lab reader turns that text into FHIR R4 Observations, each
//! pending the athlete's review, stored as `lab_document` records.
//!
//! - **Isolation**: the PDF is untrusted, so it is parsed in the
//!   `pierre-pdf-text` child process (`extractor`), bounded in time and
//!   memory; whatever a file does to the parser ends the child, and the
//!   upload is refused.
//! - **Bounds**: a body over [`MAX_LAB_DOCUMENT_BYTES`] is refused before it
//!   is read and a body that does not start `%PDF-` before the child runs.
//!   The child refuses a document whose forms draw one another or whose
//!   pages' parents chain in a cycle or past a bound (pdf-extract follows
//!   both by recursion), and one over [`pierre_pdf_text::MAX_PAGES`] pages or
//!   [`pierre_pdf_text::MAX_TEXT_BYTES`] of text. The athlete is told the bound
//!   they passed, or one fixed sentence for every other refusal; the reason
//!   is logged against the file's SHA-256, never its content. A PDF with no
//!   text layer (a scan) is kept, marked `no_text`, and yields nothing: there
//!   is no OCR.
//! - **The same file twice** is not read again: the answer is the first
//!   document and its records as they stand, a rejected one still rejected.
//! - **The model**: barrage's rules read unless `LAB_READER_MODEL` names a
//!   model ([`LabReaderModel`]) and the tier rule lets a model read the
//!   athlete's T2 records, which needs their `sensitive_t2` consent: sending a
//!   lab report's text to a model is an AI read of it. The read is bounded and
//!   recorded; a failing model leaves the rules to read.
//! - **Storing is the athlete's own act**, so it is never gated; who else
//!   reads the records is decided at read time.
//!
//! [`OriginalsStore::put_named_file`]: pierre_providers::originals::OriginalsStore::put_named_file

use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use chrono::Utc;
use dravr_barrage::digest::Sha256;
use dravr_barrage::import::ingest_reader::{DocumentReading, LabDocument, LabDocumentReader};
use dravr_barrage::Tier;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{ConversationTurnId, DataSource, DeviceType, TenantId};
use pierre_core::transport::Transport;
use pierre_database::repositories::{
    LabDocumentWrite, RecordFiling, StoredClinicalRecord, StoredLabDocument, LAB_DOCUMENT_PROVIDER,
    LAB_DOCUMENT_SOURCE_MARKER,
};
use pierre_llm::call_record::LlmCallRecorder;
use pierre_pdf_text::Refusal;
use pierre_providers::ai_scope;
use pierre_providers::originals::LAB_DOCUMENT_MEDIA_TYPE;
use pierre_services::health_records::clinical_write;
use pierre_services::lab_document_model::{LAB_MODEL_TIMEOUT, LAB_READING_CALL_TYPE};
use pierre_services::llm_usage_recorder::{LlmUsageSink, UsageRepoCallRecorder};
use pierre_services::provider_notice::under_ai_consent;
use pierre_services::wellness_store::WellnessStore;
use serde::Serialize;
use tokio::sync::Semaphore;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::mcp::resources::ServerContext;

/// The child process a PDF is parsed in.
mod extractor;

use extractor::ExtractFailure;

/// The largest lab report an upload may hold.
pub const MAX_LAB_DOCUMENT_BYTES: usize = pierre_pdf_text::MAX_PDF_BYTES;

/// The most pages a lab report is read for.
const MAX_LAB_DOCUMENT_PAGES: usize = pierre_pdf_text::MAX_PAGES;

/// What the athlete is told when a file is refused for anything but a bound:
/// it names no parser and no internal reason, which are logged instead.
const UNREADABLE_LAB_DOCUMENT: &str = "This file could not be read as a lab report";

/// What the athlete is told when the server cannot read any lab report (the
/// PDF child is missing or does not start): a fault of this server, not of
/// their file, so never [`UNREADABLE_LAB_DOCUMENT`].
const LAB_READING_UNAVAILABLE: &str = "Lab reports cannot be read right now";

/// What an upload may spend before the athlete's app gives up on it: the
/// client's 150 s (`LAB_DOCUMENT_UPLOAD_TIMEOUT_MS` in `@pierre/api-client`),
/// less a margin for storing the reading and answering.
const UPLOAD_BUDGET: Duration = Duration::from_secs(140);

/// How long an upload may wait for the PDF child's permit: what its budget
/// leaves once the child's own time and, when a model is configured, the
/// model's read are set aside: with a model only a few seconds are left.
fn permit_wait(model_configured: bool) -> Duration {
    let model = if model_configured {
        LAB_MODEL_TIMEOUT
    } else {
        Duration::ZERO
    };
    UPLOAD_BUDGET.saturating_sub(extractor::EXTRACTION_TIMEOUT + model)
}

/// A lab document's row status once its text was read.
const STATUS_READ: &str = "read";

/// A lab document's row status when it held no text layer.
const STATUS_NO_TEXT: &str = "no_text";

/// A lab document and the records read from it.
#[derive(Debug, Clone, Serialize)]
pub struct LabDocumentView {
    /// The document.
    pub document: LabDocumentSummary,
    /// The records read from it, pending the athlete's review unless they
    /// answered.
    pub records: Vec<LabRecordView>,
    /// Whether this upload stored the document, `false` when the athlete
    /// already held the same file.
    pub new: bool,
}

/// A lab document as the athlete sees it.
#[derive(Debug, Clone, Serialize)]
pub struct LabDocumentSummary {
    /// Its id.
    pub id: String,
    /// `read`, or `no_text` for a scan with no text layer.
    pub status: String,
    /// Its pages.
    pub pages: u32,
    /// Its size.
    pub size_bytes: u64,
    /// What read it (`rules`, `model:<name>`).
    pub engine: Option<String>,
    /// Lines the reader distrusted and read nothing from.
    pub flagged_lines: u32,
    /// Result rows seen but not coded.
    pub skipped: u32,
    /// A model's results dropped by verification.
    pub dropped: u32,
    /// Why a configured model's reading was not used.
    pub model_failure: Option<String>,
    /// When it was received.
    pub received_at: chrono::DateTime<Utc>,
}

impl From<StoredLabDocument> for LabDocumentSummary {
    fn from(document: StoredLabDocument) -> Self {
        Self {
            id: document.id,
            status: document.status,
            pages: document.pages,
            size_bytes: document.size_bytes,
            engine: document.engine,
            flagged_lines: document.flagged_lines,
            skipped: document.skipped,
            dropped: document.dropped,
            model_failure: document.model_failure,
            received_at: document.received_at,
        }
    }
}

/// One record read from a lab document, as its review shows it.
#[derive(Debug, Clone, Serialize)]
pub struct LabRecordView {
    /// Its id, which confirm and reject name.
    pub id: String,
    /// Its codes and numbers.
    pub flat: Option<serde_json::Value>,
    /// The checks it failed, shown beside it.
    pub checks: serde_json::Value,
    /// Where in the document it was read (`text:page=P;line=L`).
    pub locator: Option<String>,
    /// The athlete's answer: `confirmed`, `rejected`, or none yet.
    pub review_decision: Option<String>,
    /// When it was effective.
    pub effective_at: Option<chrono::DateTime<Utc>>,
}

/// A stored JSON column, `null` when it does not parse.
fn json_or_null(text: &str) -> serde_json::Value {
    serde_json::from_str(text).unwrap_or(serde_json::Value::Null)
}

impl From<StoredClinicalRecord> for LabRecordView {
    fn from(record: StoredClinicalRecord) -> Self {
        Self {
            flat: record.flat.as_deref().map(json_or_null),
            checks: json_or_null(&record.checks),
            id: record.id,
            locator: record.locator,
            review_decision: record.review_decision,
            effective_at: record.effective_at,
        }
    }
}

/// The error the athlete gets for `failure`: [`LAB_READING_UNAVAILABLE`],
/// a fault of this server, when the child could not run; otherwise an
/// invalid input naming the bound they passed, or [`UNREADABLE_LAB_DOCUMENT`].
fn failure_error(failure: &ExtractFailure) -> AppError {
    let message = match failure {
        ExtractFailure::Unavailable(_) => return AppError::internal(LAB_READING_UNAVAILABLE),
        // Another report is being read: worth trying again once it is done.
        ExtractFailure::Busy => {
            return AppError::resource_unavailable("Another lab report is being read")
                .with_retry_after(extractor::EXTRACTION_TIMEOUT.as_secs());
        }
        ExtractFailure::Refused(Refusal::TooLarge) => format!(
            "A lab report is read up to {} MB",
            MAX_LAB_DOCUMENT_BYTES / (1024 * 1024)
        ),
        ExtractFailure::Refused(Refusal::TooManyPages) => {
            format!("A lab report is read up to {MAX_LAB_DOCUMENT_PAGES} pages")
        }
        ExtractFailure::Refused(Refusal::TooMuchText) => {
            "This PDF holds more text than a lab report is read for".to_owned()
        }
        ExtractFailure::Refused(Refusal::Unreadable | Refusal::TooIndirect)
        | ExtractFailure::TimedOut
        | ExtractFailure::Ended(_)
        | ExtractFailure::Malformed => UNREADABLE_LAB_DOCUMENT.to_owned(),
    };
    AppError::invalid_input(message)
}

/// The PDF child uploaded lab reports are read in: its binary, or why none
/// was found, and the permits that keep one child alive at a time. The
/// server context holds one, so the bound is the process's.
#[derive(Debug, Clone)]
pub struct PdfChild {
    /// The child binary, or why none was found.
    program: Result<PathBuf, String>,
    /// The child permits, an `Arc` because each child's own task holds an
    /// owned permit past the upload that started it.
    permits: Arc<Semaphore>,
}

impl PdfChild {
    /// The child `program`, or no child for the reason it carries.
    #[must_use]
    pub fn new(program: Result<PathBuf, String>) -> Self {
        Self {
            program,
            permits: extractor::child_permits(),
        }
    }

    /// Why there is no child, when there is none.
    #[must_use]
    pub fn missing(&self) -> Option<&str> {
        self.program.as_ref().err().map(String::as_str)
    }
}

/// The `pierre-pdf-text` binary beside the running server, where the image
/// and every cargo build of the server put it, or why it is not there.
///
/// # Errors
///
/// Why no binary is beside the server.
pub(crate) fn pdf_text_bin_beside_server() -> Result<PathBuf, String> {
    extractor::beside_server()
}

/// The text of the PDF `bytes` the athlete uploaded as `file_sha256`, page
/// by page, read in the `pierre-pdf-text` `child` once its permit comes free
/// within `wait`.
///
/// # Errors
///
/// [`AppError::invalid_input`] for a body that is not a readable PDF within
/// the bounds, or that the child failed on; [`AppError::resource_unavailable`]
/// when no permit came free within `wait`; [`AppError::internal`] when the
/// child cannot be started.
async fn pdf_text(
    child: &PdfChild,
    (bytes, wait): (Bytes, Duration),
    file_sha256: &str,
) -> AppResult<Vec<String>> {
    if !bytes.starts_with(pierre_pdf_text::PDF_MAGIC) {
        return Err(AppError::invalid_input(UNREADABLE_LAB_DOCUMENT));
    }
    let extracted = match &child.program {
        Ok(program) => extractor::extract((program, &child.permits), bytes.to_vec(), wait).await,
        Err(reason) => Err(ExtractFailure::Unavailable(io::Error::new(
            io::ErrorKind::NotFound,
            reason.clone(),
        ))),
    };
    let failure = match extracted {
        Ok(pages) => return Ok(pages),
        Err(failure) => failure,
    };
    if let ExtractFailure::Unavailable(error) = &failure {
        error!(
            file_sha256,
            error = %error,
            "The PDF child process could not be started; no lab report can be read"
        );
    } else {
        warn!(file_sha256, failure = %failure, "A lab report was refused");
    }
    Err(failure_error(&failure))
}

/// Whether the athlete's lab report may be read by a model: the tier rule,
/// under a model read and their own consents, lets a model read T2.
///
/// The read is work the platform runs for the athlete, whose records reach
/// only their own review queue: it is served as a platform job, whichever
/// first-party surface the upload came from (an outside credential never
/// reaches the upload route).
async fn model_may_read(resources: &ServerContext, tenant_id: TenantId, user_id: Uuid) -> bool {
    let (readable, _) = ai_scope::serve_over(
        Transport::PlatformJob,
        under_ai_consent(
            &resources.common.repos,
            tenant_id.as_uuid(),
            user_id,
            ai_scope::ai_read(async { ai_scope::tier_readable(Tier::T2) }),
        ),
    )
    .await;
    readable
}

/// The reader for this athlete's upload: the rules, or the configured model
/// when a model may read their T2 records, its call recorded.
async fn reader_for(
    resources: &ServerContext,
    tenant_id: TenantId,
    user_id: Uuid,
) -> LabDocumentReader {
    let reader = LabDocumentReader::new();
    let model = &resources.common.lab_reader;
    if !model.is_configured() || !model_may_read(resources, tenant_id, user_id).await {
        return reader;
    }
    let recorder: Arc<dyn LlmCallRecorder> = Arc::new(UsageRepoCallRecorder::new(
        LlmUsageSink::new(
            Arc::clone(&resources.common.repos.llm_usage),
            resources.common.turns.tracker(),
        ),
        tenant_id.to_string(),
        user_id.to_string(),
        None,
        ConversationTurnId::new(),
        LAB_READING_CALL_TYPE,
    ));
    match model.for_read(Some(recorder)) {
        Some(model) => reader.with_model(model),
        None => reader,
    }
}

/// The data source lab records are filed under, recorded once per athlete.
async fn lab_data_source(
    resources: &ServerContext,
    tenant_id: &TenantId,
    user_id: Uuid,
) -> AppResult<String> {
    let source = DataSource {
        id: Uuid::new_v4().to_string(),
        user_id: user_id.to_string(),
        provider: LAB_DOCUMENT_PROVIDER.to_owned(),
        device_model: None,
        software_version: None,
        source: Some(LAB_DOCUMENT_SOURCE_MARKER.to_owned()),
        device_type: DeviceType::Unknown,
        original_source_name: None,
    };
    WellnessStore::new(&resources.common.repos)
        .record_data_source(tenant_id, &source)
        .await
}

/// A model failure, as stored.
fn failure_name(reading: &DocumentReading) -> Option<String> {
    reading.model_failure.map(|failure| {
        serde_json::to_value(failure)
            .ok()
            .and_then(|value| value.as_str().map(str::to_owned))
            .unwrap_or_else(|| format!("{failure:?}"))
    })
}

/// The document `document` and its records, as the athlete sees them.
async fn view_of(
    resources: &ServerContext,
    tenant_id: &TenantId,
    user_id: Uuid,
    document: StoredLabDocument,
    new: bool,
) -> AppResult<LabDocumentView> {
    let records = resources
        .common
        .repos
        .health_records
        .list_lab_document_records(tenant_id, user_id, &document.id)
        .await?;
    Ok(LabDocumentView {
        document: document.into(),
        records: records.into_iter().map(LabRecordView::from).collect(),
        new,
    })
}

/// The reading of a report's `pages`, or `None` when none holds text (a
/// scan): nothing is read from it.
async fn read_pages(
    resources: &ServerContext,
    (tenant_id, user_id): (TenantId, Uuid),
    pages: Vec<String>,
    sha256: Sha256,
    received: chrono::DateTime<Utc>,
) -> AppResult<Option<DocumentReading>> {
    if !pages.iter().any(|page| !page.trim().is_empty()) {
        return Ok(None);
    }
    let reader = reader_for(resources, tenant_id, user_id).await;
    let document = LabDocument {
        pages,
        sha256,
        received,
    };
    reader
        .read(&document)
        .await
        .map(Some)
        .map_err(|e| AppError::internal(format!("lab reader: {e}")))
}

/// The row a report is stored as: `(file_sha256, size_bytes, pages)` and what
/// its reading yielded.
fn document_write(
    (file_sha256, size_bytes, pages): (String, u64, u32),
    reading: Option<&DocumentReading>,
    received_at: chrono::DateTime<Utc>,
) -> LabDocumentWrite {
    let count = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
    LabDocumentWrite {
        file_sha256,
        size_bytes,
        pages,
        status: if reading.is_some() {
            STATUS_READ
        } else {
            STATUS_NO_TEXT
        }
        .to_owned(),
        engine: reading.map(|reading| reading.engine.label()),
        flagged_lines: reading.map_or(0, |reading| count(reading.flagged_lines.len())),
        skipped: reading.map_or(0, |reading| reading.skipped),
        dropped: reading.map_or(0, |reading| reading.dropped),
        model_failure: reading.and_then(failure_name),
        received_at,
    }
}

/// Store the Observations `reading` yielded as the document's pending records.
async fn store_records(
    resources: &ServerContext,
    (tenant_id, user_id): (&TenantId, Uuid),
    document_id: &str,
    reading: &DocumentReading,
) -> AppResult<()> {
    let records = reading
        .labs
        .iter()
        .map(|lab| clinical_write(lab, Some(document_id)))
        .collect::<AppResult<Vec<_>>>()?;
    let data_source_id = lab_data_source(resources, tenant_id, user_id).await?;
    resources
        .common
        .repos
        .health_records
        .upsert_clinical_records(
            RecordFiling {
                tenant_id,
                user_id,
                provider: LAB_DOCUMENT_PROVIDER,
                data_source_id: &data_source_id,
            },
            &records,
        )
        .await?;
    Ok(())
}

/// Keep the lab report `body` the athlete uploaded and read it into pending
/// records.
///
/// # Errors
///
/// [`AppError::invalid_input`] for a body that is not a readable PDF within
/// the bounds; [`AppError::resource_unavailable`] where no blob store is
/// placed, or while another report holds the PDF child past this upload's
/// wait; the stores' errors.
pub async fn upload_lab_document(
    resources: &Arc<ServerContext>,
    tenant_id: TenantId,
    user_id: Uuid,
    body: Bytes,
) -> AppResult<LabDocumentView> {
    let repos = &resources.common.repos;
    let digest = Sha256::of(&body);
    let file_sha256 = digest.to_hex();
    if let Some(held) = repos
        .health_records
        .find_lab_document_by_file(&tenant_id, user_id, &file_sha256)
        .await?
    {
        return view_of(resources, &tenant_id, user_id, held, false).await;
    }
    let size_bytes = u64::try_from(body.len()).unwrap_or(u64::MAX);
    let wait = permit_wait(resources.common.lab_reader.is_configured());
    let pages = pdf_text(
        &resources.common.pdf_child,
        (body.clone(), wait),
        &file_sha256,
    )
    .await?;
    let page_count = u32::try_from(pages.len()).unwrap_or(u32::MAX);
    resources
        .common
        .originals
        .put_named_file(
            &tenant_id,
            user_id,
            &file_sha256,
            LAB_DOCUMENT_MEDIA_TYPE,
            body,
        )
        .await?;
    let received_at = Utc::now();
    let reading = read_pages(resources, (tenant_id, user_id), pages, digest, received_at).await?;
    let write = document_write(
        (file_sha256.clone(), size_bytes, page_count),
        reading.as_ref(),
        received_at,
    );
    let Some(document_id) = repos
        .health_records
        .insert_lab_document(&tenant_id, user_id, &write)
        .await?
    else {
        // The same file landed from a second request meanwhile: answer it.
        let held = repos
            .health_records
            .find_lab_document_by_file(&tenant_id, user_id, &file_sha256)
            .await?
            .ok_or_else(|| AppError::internal("a lab document vanished while it was stored"))?;
        return view_of(resources, &tenant_id, user_id, held, false).await;
    };
    stored_view(
        resources,
        (&tenant_id, user_id),
        &document_id,
        (reading.as_ref(), &write),
    )
    .await
}

/// Store a new document's records, then answer it as the athlete sees it.
async fn stored_view(
    resources: &ServerContext,
    (tenant_id, user_id): (&TenantId, Uuid),
    document_id: &str,
    (reading, write): (Option<&DocumentReading>, &LabDocumentWrite),
) -> AppResult<LabDocumentView> {
    if let Some(reading) = reading {
        store_records(resources, (tenant_id, user_id), document_id, reading).await?;
        if let Some(failure) = &write.model_failure {
            warn!(%user_id, failure = %failure, "The lab reader's model was not used; the rules read");
        }
    }
    info!(
        %user_id,
        pages = write.pages,
        status = %write.status,
        engine = write.engine.as_deref().unwrap_or("none"),
        records = reading.map_or(0, |reading| reading.labs.len()),
        "lab document stored"
    );
    let stored = resources
        .common
        .repos
        .health_records
        .get_lab_document(tenant_id, user_id, document_id)
        .await?
        .ok_or_else(|| AppError::internal("a lab document vanished once stored"))?;
    view_of(resources, tenant_id, user_id, stored, true).await
}

/// The athlete's lab documents, newest first, each with its records.
///
/// # Errors
///
/// The store's error.
pub async fn list_lab_documents(
    resources: &ServerContext,
    tenant_id: &TenantId,
    user_id: Uuid,
) -> AppResult<Vec<LabDocumentView>> {
    let documents = resources
        .common
        .repos
        .health_records
        .list_lab_documents(tenant_id, user_id)
        .await?;
    let mut views = Vec::with_capacity(documents.len());
    for document in documents {
        views.push(view_of(resources, tenant_id, user_id, document, false).await?);
    }
    Ok(views)
}

/// Delete the athlete's lab document `id`: its bytes, then its records and
/// its row. Returns whether they held it.
///
/// # Errors
///
/// The blob store's or the database's error; the row stays when the bytes
/// could not be deleted, so a retry finds it.
pub async fn delete_lab_document(
    resources: &ServerContext,
    tenant_id: &TenantId,
    user_id: Uuid,
    id: &str,
) -> AppResult<bool> {
    let repos = &resources.common.repos;
    let Some(document) = repos
        .health_records
        .get_lab_document(tenant_id, user_id, id)
        .await?
    else {
        return Ok(false);
    };
    resources
        .common
        .originals
        .delete_named_file(tenant_id, user_id, &document.file_sha256)
        .await?;
    repos
        .health_records
        .delete_lab_document(tenant_id, user_id, id)
        .await
}

#[cfg(test)]
mod tests {
    use std::process::ExitStatus;

    use pierre_core::errors::ErrorCode;

    use super::*;

    #[tokio::test]
    async fn a_body_that_is_not_a_pdf_is_refused_before_the_child_runs() {
        let refused = pdf_text(
            &PdfChild::new(Err("not needed".to_owned())),
            (Bytes::from_static(b"PK\x03\x04 a zip"), Duration::ZERO),
            "sha",
        )
        .await;
        let error = refused.expect_err("bug: a non-PDF body was read");
        assert_eq!(
            error.message, UNREADABLE_LAB_DOCUMENT,
            "bug: a non-PDF body was told more than the fixed sentence"
        );
    }

    #[tokio::test]
    async fn a_missing_child_is_a_server_fault_not_an_unreadable_file() {
        let failed = pdf_text(
            &PdfChild::new(Err("pierre-pdf-text is not beside the server".to_owned())),
            (Bytes::from_static(b"%PDF-1.7\n"), Duration::ZERO),
            "sha",
        )
        .await;
        let error = failed.expect_err("bug: a PDF was read with no child");
        assert_eq!(
            error.code,
            ErrorCode::InternalError,
            "bug: a missing child reads as the athlete's fault"
        );
        assert_eq!(
            error.message, LAB_READING_UNAVAILABLE,
            "bug: a missing child reads as an unreadable file"
        );
    }

    #[tokio::test]
    async fn an_unreadable_file_is_the_athletes_input_error() {
        let refused = pdf_text(
            &PdfChild::new(Err("not needed".to_owned())),
            (Bytes::from_static(b"PK\x03\x04 a zip"), Duration::ZERO),
            "sha",
        )
        .await;
        assert_eq!(
            refused.expect_err("bug: a non-PDF was read").code,
            ErrorCode::InvalidInput,
            "bug: an athlete's bad upload answers a server error"
        );
    }

    #[test]
    fn a_busy_child_asks_the_athlete_to_retry_not_a_server_error() {
        let error = failure_error(&ExtractFailure::Busy);
        assert_eq!(
            error.code,
            ErrorCode::ResourceUnavailable,
            "bug: a busy child answers other than 503"
        );
        assert_eq!(
            error.retry_after_secs(),
            Some(extractor::EXTRACTION_TIMEOUT.as_secs()),
            "bug: a busy child gives no retry window"
        );
    }

    #[test]
    fn an_upload_waits_for_the_child_only_within_its_budget() {
        assert_eq!(
            permit_wait(false),
            UPLOAD_BUDGET.saturating_sub(extractor::EXTRACTION_TIMEOUT),
            "bug: the rules' upload waits past what its budget leaves"
        );
        assert_eq!(
            permit_wait(true),
            UPLOAD_BUDGET.saturating_sub(extractor::EXTRACTION_TIMEOUT + LAB_MODEL_TIMEOUT),
            "bug: a model's upload waits past what its budget leaves"
        );
        assert!(
            permit_wait(true) > Duration::ZERO,
            "bug: with a model, an upload behind another is refused at once"
        );
    }

    #[test]
    fn every_refusal_but_a_bound_reads_the_same_fixed_sentence() {
        let fixed = [
            ExtractFailure::Refused(Refusal::Unreadable),
            ExtractFailure::Refused(Refusal::TooIndirect),
            ExtractFailure::TimedOut,
            ExtractFailure::Ended(ExitStatus::default()),
            ExtractFailure::Malformed,
        ];
        for failure in &fixed {
            let error = failure_error(failure);
            assert_eq!(
                (error.code, error.message.as_str()),
                (ErrorCode::InvalidInput, UNREADABLE_LAB_DOCUMENT),
                "bug: {failure:?} told the athlete more than the fixed sentence"
            );
        }
        let pages = failure_error(&ExtractFailure::Refused(Refusal::TooManyPages)).message;
        assert!(
            pages.contains(&MAX_LAB_DOCUMENT_PAGES.to_string()),
            "bug: the page bound is not named: {pages}"
        );
    }

    #[test]
    fn a_child_that_cannot_run_is_the_servers_fault_with_its_own_sentence() {
        let error = failure_error(&ExtractFailure::Unavailable(io::Error::other("gone")));
        assert_eq!(
            (error.code, error.message.as_str()),
            (ErrorCode::InternalError, LAB_READING_UNAVAILABLE),
            "bug: a child that cannot run reads as the athlete's unreadable file"
        );
    }
}
