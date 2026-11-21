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
//! - **Bounds**: a body over [`MAX_LAB_DOCUMENT_BYTES`] is refused before it
//!   is read, a body that does not start `%PDF-` is refused, and a document
//!   over [`MAX_LAB_DOCUMENT_PAGES`] pages or [`MAX_LAB_DOCUMENT_TEXT_BYTES`]
//!   of text is refused once read. A PDF with no text layer (a scan) is kept,
//!   marked `no_text`, and yields nothing: there is no OCR.
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

use std::sync::Arc;

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
};
use pierre_llm::call_record::LlmCallRecorder;
use pierre_providers::ai_scope;
use pierre_providers::originals::LAB_DOCUMENT_MEDIA_TYPE;
use pierre_services::health_records::clinical_write;
use pierre_services::lab_document_model::LAB_READING_CALL_TYPE;
use pierre_services::llm_usage_recorder::{LlmUsageSink, UsageRepoCallRecorder};
use pierre_services::provider_notice::under_ai_consent;
use pierre_services::wellness_store::WellnessStore;
use serde::Serialize;
use tokio::task::spawn_blocking;
use tracing::{info, warn};
use uuid::Uuid;

use crate::mcp::resources::ServerContext;

/// The largest lab report an upload may hold.
pub const MAX_LAB_DOCUMENT_BYTES: usize = 10 * 1024 * 1024;

/// The most pages a lab report is read for.
pub const MAX_LAB_DOCUMENT_PAGES: usize = 200;

/// The most text a lab report is read for.
pub const MAX_LAB_DOCUMENT_TEXT_BYTES: usize = 2 * 1024 * 1024;

/// What every PDF starts with.
const PDF_MAGIC: &[u8] = b"%PDF-";

/// The `source` marker of the data source lab records are filed under.
const LAB_DOCUMENT_SOURCE_MARKER: &str = "upload";

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

/// A PDF's text, page by page.
///
/// # Errors
///
/// [`AppError::invalid_input`] for a body that is not a readable PDF, or one
/// over the page or text bounds.
pub async fn pdf_text(bytes: Bytes) -> AppResult<Vec<String>> {
    if !bytes.starts_with(PDF_MAGIC) {
        return Err(AppError::invalid_input("A lab report must be a PDF file"));
    }
    // The extractor parses an untrusted file: off the async workers, and a
    // panic in it is the file's fault, not the server's.
    let pages = spawn_blocking(move || pdf_extract::extract_text_from_mem_by_pages(&bytes))
        .await
        .map_err(|_| AppError::invalid_input("The PDF could not be read"))?
        .map_err(|e| AppError::invalid_input(format!("The PDF could not be read: {e}")))?;
    if pages.len() > MAX_LAB_DOCUMENT_PAGES {
        return Err(AppError::invalid_input(format!(
            "A lab report is read up to {MAX_LAB_DOCUMENT_PAGES} pages; this one has {}",
            pages.len()
        )));
    }
    if pages.iter().map(String::len).sum::<usize>() > MAX_LAB_DOCUMENT_TEXT_BYTES {
        return Err(AppError::invalid_input(
            "This PDF holds more text than a lab report is read for",
        ));
    }
    Ok(pages)
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
/// placed; the stores' errors.
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
    let pages = pdf_text(body.clone()).await?;
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
    use super::*;

    #[tokio::test]
    async fn a_body_that_is_not_a_pdf_is_refused_before_it_is_parsed() {
        let refused = pdf_text(Bytes::from_static(b"PK\x03\x04 a zip")).await;
        assert!(refused.is_err(), "bug: a non-PDF body was read");
    }

    #[tokio::test]
    async fn a_truncated_pdf_is_refused_not_a_panic() {
        let refused = pdf_text(Bytes::from_static(b"%PDF-1.7\n1 0 obj <<")).await;
        assert!(refused.is_err(), "bug: a broken PDF was read");
    }
}
