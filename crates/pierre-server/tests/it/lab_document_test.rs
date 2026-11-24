// ABOUTME: An uploaded lab report becomes pending records the athlete confirms or rejects; a scan, a non-PDF and a re-upload
// ABOUTME: The configured model reads only under the athlete's sensitive_t2, and a failing one leaves the rules to read (carnet#1544)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! Owl phase 5, slice 2: lab documents.
//!
//! `tests/fixtures/owl-labs/t25-000-table-fr.pdf` and its `.truth.json` are
//! copied unchanged from dravr-owl `crates/owl-extraction-bench/dataset/sample/`
//! (MIT OR Apache-2.0); the report is synthetic, every name and value in it
//! fictional. It is read here through the platform's PDF text extraction and
//! barrage's lab reader, and its result count checked against owl's truth.

use pierre_core::constants::oauth_providers::NoticeKind;
use std::fmt::Write as _;
use std::fs;
use std::io::Write as _;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Duration as StdDuration;

use dravr_tronc::dates::Timestamp;

use bytes::Bytes;
use dravr_barrage::blob::memory::MemoryBlobStore;
use dravr_barrage::digest::Sha256;
use pierre_auth::auth::AuthManager;
use pierre_cache::{Cache, CacheConfig};
use pierre_core::constants::oauth::providers::SENSITIVE_T2;
use pierre_core::errors::ErrorCode;
use pierre_core::models::TenantId;
use pierre_database::repositories::{
    ClinicalSelection, PurgeReason, RecordWindow, ReviewDecision, LAB_DOCUMENT_PROVIDER,
};
use pierre_mcp_server::mcp::resources::{ServerContext, ServerContextOptions};
use pierre_mcp_server::services::lab_documents::{delete_lab_document, upload_lab_document};
use pierre_services::lab_document_model::LabReaderModel;
use pierre_services::provider_notice::grant_consent;
use serde_json::Value;
use uuid::Uuid;

use crate::archive_import_test::seed_athlete;
use crate::common::{self, TestLlmProvider};

/// owl's synthetic French table report.
const REPORT: &[u8] = include_bytes!("../fixtures/owl-labs/t25-000-table-fr.pdf");

/// The one sentence the athlete reads for every refusal but a bound, as the
/// upload answers it: no parser, no internal reason.
const FIXED_REFUSAL: &str = "This file could not be read as a lab report";

/// Its ground truth.
const TRUTH: &str = include_str!("../fixtures/owl-labs/t25-000-table-fr.truth.json");

/// How many results owl's truth lists for the report.
fn truth_count() -> usize {
    let truth: Value = serde_json::from_str(TRUTH).unwrap();
    truth["results"].as_array().unwrap().len()
}

/// The PDF child cargo built for these tests. Every upload runs it, so a run
/// that cannot find it fails here, naming it, rather than reading every PDF
/// as unreadable: a lane that ships test binaries without it
/// (`scripts/ci/test-binaries.sh` packs it from this `env!`).
fn pdf_child() -> PathBuf {
    let child = PathBuf::from(env!("CARGO_BIN_EXE_pierre-pdf-text"));
    assert!(
        child.is_file(),
        "bug: the PDF child is not where cargo built it ({}): pack it with the test binaries",
        child.display()
    );
    child
}

#[test]
fn the_pdf_child_is_where_cargo_built_it() {
    pdf_child();
}

/// A server context whose lab reader reads with `model`.
async fn context(model: LabReaderModel) -> Arc<ServerContext> {
    context_with(model, pdf_child()).await
}

/// [`context`], reading PDFs with the child `pdf_text_bin`.
async fn context_with(model: LabReaderModel, pdf_text_bin: PathBuf) -> Arc<ServerContext> {
    common::init_server_config();
    let database = common::create_test_database().await.unwrap();
    let cache = Cache::new(CacheConfig {
        max_entries: 1000,
        redis_url: None,
        cleanup_interval: StdDuration::from_mins(1),
        enable_background_cleanup: false,
        ..Default::default()
    })
    .await
    .unwrap();
    Arc::new(
        ServerContext::new(
            (*database).clone(),
            AuthManager::new(24),
            "test_admin_secret",
            common::offline_config(Arc::new(common::test_server_config())),
            cache,
            ServerContextOptions {
                jwks_manager: Some(common::get_shared_test_jwks()),
                lab_reader: Some(model),
                pdf_text_bin: Some(pdf_text_bin),
                blobs: Some(Ok(Arc::new(MemoryBlobStore::default()))),
                ..ServerContextOptions::testing(common::offline_adapters())
            },
        )
        .await
        .unwrap(),
    )
}

/// A one-page PDF with no text layer, as a scan is.
fn textless_pdf() -> Bytes {
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << >> >>",
        "<< /Length 0 >>\nstream\n\nendstream",
    ];
    let mut pdf = String::from("%PDF-1.4\n");
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        write!(pdf, "{} 0 obj\n{object}\nendobj\n", index + 1).unwrap();
    }
    let xref = pdf.len();
    write!(pdf, "xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).unwrap();
    for offset in offsets {
        writeln!(pdf, "{offset:010} 00000 n ").unwrap();
    }
    write!(
        pdf,
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
        objects.len() + 1
    )
    .unwrap();
    Bytes::from(pdf)
}

fn window() -> RecordWindow {
    RecordWindow {
        since: None,
        until: None,
        limit: 200,
    }
}

#[tokio::test]
async fn a_text_report_becomes_pending_records_the_athlete_confirms_or_rejects() {
    let resources = context(LabReaderModel::rules()).await;
    let athlete = seed_athlete(&resources, "lab-report").await;
    let (user, tenant): (Uuid, TenantId) = (athlete.user_id, athlete.tenant);
    let repos = &resources.common.repos;

    let view = upload_lab_document(&resources, tenant, user, Bytes::from_static(REPORT))
        .await
        .unwrap();
    assert!(view.new, "bug: the first upload was not stored");
    assert_eq!(view.document.status, "read", "bug: {view:?}");
    assert_eq!(
        view.document.engine.as_deref(),
        Some("rules"),
        "bug: the default reader"
    );
    assert_eq!(
        view.records.len(),
        truth_count(),
        "bug: owl's truth: {view:?}"
    );
    let served = repos
        .health_records
        .list_clinical_records(
            &tenant,
            user,
            ClinicalSelection::Served { observations: None },
            window(),
        )
        .await
        .unwrap();
    assert!(
        served.is_empty(),
        "bug: a pending record was served: {served:?}"
    );

    let confirmed = &view.records[0].id;
    let rejected = &view.records[1].id;
    for (id, decision) in [
        (confirmed, ReviewDecision::Confirmed),
        (rejected, ReviewDecision::Rejected),
    ] {
        assert!(
            repos
                .health_records
                .decide_clinical_record(&tenant, user, id, decision, Timestamp::now())
                .await
                .unwrap(),
            "bug: {decision:?} was not recorded"
        );
    }
    assert!(
        !repos
            .health_records
            .decide_clinical_record(
                &tenant,
                user,
                confirmed,
                ReviewDecision::Rejected,
                Timestamp::now()
            )
            .await
            .unwrap(),
        "bug: a decided record was decided again"
    );
    let served = repos
        .health_records
        .list_clinical_records(
            &tenant,
            user,
            ClinicalSelection::Served {
                observations: Some(true),
            },
            window(),
        )
        .await
        .unwrap();
    assert_eq!(
        served.len(),
        1,
        "bug: only the confirmed record is served: {served:?}"
    );
    assert_eq!(&served[0].id, confirmed, "bug: the wrong record was served");

    let again = upload_lab_document(&resources, tenant, user, Bytes::from_static(REPORT))
        .await
        .unwrap();
    assert!(!again.new, "bug: the same file was stored twice");
    let rejected_again = again
        .records
        .iter()
        .find(|record| &record.id == rejected)
        .unwrap();
    assert_eq!(
        rejected_again.review_decision.as_deref(),
        Some("rejected"),
        "bug: a re-upload undid a rejection"
    );

    assert!(
        delete_lab_document(&resources, &tenant, user, &view.document.id)
            .await
            .unwrap(),
        "bug: the delete"
    );
    let pending = repos
        .health_records
        .list_clinical_records(&tenant, user, ClinicalSelection::Pending, window())
        .await
        .unwrap();
    assert!(
        pending.is_empty(),
        "bug: the records outlived their document"
    );
    assert!(
        repos
            .health_records
            .list_lab_documents(&tenant, user)
            .await
            .unwrap()
            .is_empty(),
        "bug: the document row outlived its delete"
    );
    let file = Sha256::of(REPORT).to_hex();
    assert!(
        resources
            .common
            .originals
            .get_named_file(&tenant, user, &file)
            .await
            .unwrap()
            .is_none(),
        "bug: the PDF outlived its delete"
    );
}

#[tokio::test]
async fn a_scan_is_kept_unread_and_a_non_pdf_is_refused() {
    let resources = context(LabReaderModel::rules()).await;
    let athlete = seed_athlete(&resources, "lab-scan").await;

    let scan = upload_lab_document(&resources, athlete.tenant, athlete.user_id, textless_pdf())
        .await
        .unwrap();
    assert_eq!(
        scan.document.status, "no_text",
        "bug: a scan read as text: {scan:?}"
    );
    assert!(scan.records.is_empty(), "bug: a scan yielded records");

    let refused = upload_lab_document(
        &resources,
        athlete.tenant,
        athlete.user_id,
        Bytes::from_static(b"PK\x03\x04 not a pdf"),
    )
    .await;
    assert!(refused.is_err(), "bug: a non-PDF body was stored");
}

#[tokio::test]
async fn the_model_reads_only_under_sensitive_t2_and_a_failing_one_leaves_the_rules() {
    let answers = TestLlmProvider::with_response(r#"{"results": []}"#.to_owned());
    let resources = context(LabReaderModel::over(Arc::new(answers))).await;
    let athlete = seed_athlete(&resources, "lab-model").await;
    let (user, tenant) = (athlete.user_id, athlete.tenant);

    let unconsented = upload_lab_document(&resources, tenant, user, Bytes::from_static(REPORT))
        .await
        .unwrap();
    assert_eq!(
        unconsented.document.engine.as_deref(),
        Some("rules"),
        "bug: a model read without consent"
    );
    assert!(
        delete_lab_document(&resources, &tenant, user, &unconsented.document.id)
            .await
            .unwrap()
    );

    grant_consent(
        &resources.common.repos,
        tenant.as_uuid(),
        user,
        SENSITIVE_T2,
        NoticeKind::AiConsent,
        "Health records",
    )
    .await
    .unwrap();
    let consented = upload_lab_document(&resources, tenant, user, Bytes::from_static(REPORT))
        .await
        .unwrap();
    assert_eq!(
        consented.document.engine.as_deref(),
        Some("model:test-model-v1"),
        "bug: the model under consent: {consented:?}"
    );
    assert!(
        consented.records.is_empty(),
        "bug: the model answered no result"
    );

    let broken = TestLlmProvider::with_response("not the JSON the prompt asks for".to_owned());
    let resources = context(LabReaderModel::over(Arc::new(broken))).await;
    let athlete = seed_athlete(&resources, "lab-model-broken").await;
    grant_consent(
        &resources.common.repos,
        athlete.tenant.as_uuid(),
        athlete.user_id,
        SENSITIVE_T2,
        NoticeKind::AiConsent,
        "Health records",
    )
    .await
    .unwrap();
    let fallback = upload_lab_document(
        &resources,
        athlete.tenant,
        athlete.user_id,
        Bytes::from_static(REPORT),
    )
    .await
    .unwrap();
    assert_eq!(
        fallback.document.engine.as_deref(),
        Some("rules"),
        "bug: a malformed answer was used"
    );
    assert_eq!(
        fallback.document.model_failure.as_deref(),
        Some("malformed"),
        "bug: why the model was not used"
    );
    assert_eq!(
        fallback.records.len(),
        truth_count(),
        "bug: the rules did not read after the failure"
    );
}

/// A one-page PDF whose font's `/ToUnicode` map is a 1.2 KB stream, Flate
/// twice, that inflates to 640 MiB of zeros. Nothing the server's pre-check
/// reads (the page tree, page content, forms) is hostile, so the bomb goes
/// off inside pdf-extract, when it loads the font.
const DECOMPRESSION_BOMB: &[u8] = include_bytes!("../fixtures/owl-labs/tounicode-bomb.pdf");

#[test]
fn the_pdf_child_ends_itself_at_its_memory_cap() {
    let mut child = Command::new(pdf_child())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(DECOMPRESSION_BOMB)
        .unwrap();
    let output = child.wait_with_output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success() && output.stdout.is_empty(),
        "bug: the bomb was answered: {:?}",
        output.status
    );
    assert!(
        stderr.contains("memory allocation of"),
        "bug: the child ended without its allocation cap refusing: {stderr}"
    );
}

#[tokio::test]
async fn a_decompression_bomb_is_refused_and_the_server_reads_on() {
    let resources = context(LabReaderModel::rules()).await;
    let athlete = seed_athlete(&resources, "lab-bomb").await;
    let refused = upload_lab_document(
        &resources,
        athlete.tenant,
        athlete.user_id,
        Bytes::from_static(DECOMPRESSION_BOMB),
    )
    .await;
    let error = refused.expect_err("bug: a decompression bomb was read");
    assert_eq!(
        (error.code, error.message.as_str()),
        (ErrorCode::InvalidInput, FIXED_REFUSAL),
        "bug: the athlete was told more than the fixed sentence"
    );
    let held = resources
        .common
        .repos
        .health_records
        .list_lab_documents(&athlete.tenant, athlete.user_id)
        .await
        .unwrap();
    assert!(held.is_empty(), "bug: a refused file was stored");
    let read = upload_lab_document(
        &resources,
        athlete.tenant,
        athlete.user_id,
        Bytes::from_static(REPORT),
    )
    .await
    .unwrap();
    assert_eq!(
        read.records.len(),
        truth_count(),
        "bug: the next report was not read after the refusal"
    );
}

#[tokio::test]
async fn a_truncated_pdf_is_refused_with_the_fixed_sentence() {
    let resources = context(LabReaderModel::rules()).await;
    let athlete = seed_athlete(&resources, "lab-truncated").await;
    let refused = upload_lab_document(
        &resources,
        athlete.tenant,
        athlete.user_id,
        Bytes::from_static(b"%PDF-1.7\n1 0 obj <<"),
    )
    .await;
    let error = refused.expect_err("bug: a truncated PDF was read");
    assert_eq!(
        (error.code, error.message.as_str()),
        (ErrorCode::InvalidInput, FIXED_REFUSAL),
        "bug: the refusal names more than the fixed sentence"
    );
}

#[tokio::test]
async fn purging_the_lab_document_source_deletes_the_documents_and_their_pdfs() {
    let resources = context(LabReaderModel::rules()).await;
    let athlete = seed_athlete(&resources, "lab-purge").await;
    let (tenant, user) = (athlete.tenant, athlete.user_id);
    let stored = upload_lab_document(&resources, tenant, user, Bytes::from_static(REPORT))
        .await
        .unwrap();
    let file = Sha256::of(REPORT).to_hex();
    assert!(!stored.records.is_empty(), "bug: the report was not read");
    let purge = resources
        .common
        .provider_purge
        .purge_user_provider_data(
            user,
            &tenant,
            LAB_DOCUMENT_PROVIDER,
            PurgeReason::AthleteDisconnect,
        )
        .await
        .unwrap();
    assert!(purge.total() > 0, "bug: the purge removed nothing");
    let repos = &resources.common.repos;
    assert!(
        repos
            .health_records
            .list_lab_documents(&tenant, user)
            .await
            .unwrap()
            .is_empty(),
        "bug: a lab document outlived its source's purge"
    );
    assert!(
        repos
            .health_records
            .list_lab_document_records(&tenant, user, &stored.document.id)
            .await
            .unwrap()
            .is_empty(),
        "bug: a record read from a purged document survived"
    );
    assert_eq!(
        resources
            .common
            .originals
            .get_named_file(&tenant, user, &file)
            .await
            .unwrap(),
        None,
        "bug: the purged document's PDF is still kept"
    );
}

#[tokio::test]
async fn the_admins_lab_document_purge_deletes_every_document_and_pdf() {
    let resources = context(LabReaderModel::rules()).await;
    let athlete = seed_athlete(&resources, "lab-terminate").await;
    let (tenant, user) = (athlete.tenant, athlete.user_id);
    upload_lab_document(&resources, tenant, user, Bytes::from_static(REPORT))
        .await
        .unwrap();
    let file = Sha256::of(REPORT).to_hex();
    resources
        .common
        .provider_purge
        .purge_provider_data(LAB_DOCUMENT_PROVIDER)
        .await
        .unwrap();
    assert!(
        resources
            .common
            .repos
            .health_records
            .list_lab_documents(&tenant, user)
            .await
            .unwrap()
            .is_empty(),
        "bug: a lab document outlived the provider's purge"
    );
    assert_eq!(
        resources
            .common
            .originals
            .get_named_file(&tenant, user, &file)
            .await
            .unwrap(),
        None,
        "bug: the provider purge left a PDF"
    );
}

/// A stand-in for the PDF child that records, in `dir`, how many children are
/// alive as it starts, holds a second, then runs the real child.
fn counting_child(dir: &Path) -> PathBuf {
    let script = dir.join("counting-pdf-text");
    let body = format!(
        "#!/bin/sh\n\
         touch \"{dir}/alive-$$\"\n\
         ls \"{dir}\" | grep -c '^alive-' >> \"{dir}/counts\"\n\
         sleep 1\n\
         \"{child}\"\n\
         status=$?\n\
         rm -f \"{dir}/alive-$$\"\n\
         exit $status\n",
        dir = dir.display(),
        child = pdf_child().display(),
    );
    fs::write(&script, body).unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    script
}

#[tokio::test]
async fn two_simultaneous_uploads_are_both_read_one_child_at_a_time() {
    let dir = tempfile::tempdir().unwrap();
    let resources = context_with(LabReaderModel::rules(), counting_child(dir.path())).await;
    let first = seed_athlete(&resources, "lab-concurrent-a").await;
    let second = seed_athlete(&resources, "lab-concurrent-b").await;
    let reads: [_; 2] = tokio::join!(
        upload_lab_document(
            &resources,
            first.tenant,
            first.user_id,
            Bytes::from_static(REPORT)
        ),
        upload_lab_document(
            &resources,
            second.tenant,
            second.user_id,
            Bytes::from_static(REPORT)
        ),
    )
    .into();
    for read in reads {
        let read = read.expect("bug: a simultaneous upload was refused");
        assert_eq!(
            read.records.len(),
            truth_count(),
            "bug: a simultaneous upload was not read in full"
        );
    }
    let counts = fs::read_to_string(dir.path().join("counts")).unwrap();
    let alive: Vec<&str> = counts.lines().collect();
    assert_eq!(alive.len(), 2, "bug: not one child per upload: {alive:?}");
    assert!(
        alive.iter().all(|count| count.trim() == "1"),
        "bug: two PDF children were alive at once: {alive:?}"
    );
}
