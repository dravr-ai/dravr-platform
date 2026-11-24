// ABOUTME: HealthRecordRepository trait plus the one shared body both backends emit: self-reports, clinical records, lab documents
// ABOUTME: The T2–T3 records an import or a lab document yields, keyed by their barrage id; every statement scoped to tenant and user

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Health records (owl phase 5, slice 2; carnet#1544).
//!
//! Every import keeps the T2–T3 records dravr-barrage reads for the athlete
//! (JF 2026-11-20, "Store, gate reads"); who else reads them is decided by
//! the tier consent at read time, never here. Three tables:
//!
//! - `self_reports`: cycle-tracking records, T3;
//! - `clinical_records`: FHIR R4 resources, each with its own tier, its
//!   free-text-stripped copy and its review state. A record read from a lab
//!   document waits for the athlete (`review_state = 'pending'`); their answer
//!   is `review_decision`, which a re-import never overwrites, so a rejected
//!   record stays rejected without a tombstone;
//! - `lab_documents`: one row per lab PDF the athlete uploaded, whose bytes
//!   live in the blob store under the upload key.
//!
//! Each record is keyed by its barrage id per provider key (the import's
//! source key, or `lab_document`) and filed under a data source, as every
//! wellness row is, so the provider purges reach them
//! (`provider_data/statements.rs`). JSON columns are TEXT on both engines.
//! `$n` placeholders throughout; ids go through the backend's uuid codec.

use async_trait::async_trait;
use dravr_tronc::dates::Timestamp;
use pierre_core::errors::AppResult;
use pierre_core::models::TenantId;
use uuid::Uuid;

/// The provider key a record read from an uploaded lab document is filed under.
pub const LAB_DOCUMENT_PROVIDER: &str = "lab_document";

/// The `source` marker of the data source lab records are filed under.
pub const LAB_DOCUMENT_SOURCE_MARKER: &str = "upload";

/// The most rows one list returns.
pub const MAX_HEALTH_RECORDS_PAGE: u32 = 200;

/// Whose records a write files, and under which provider key and data source.
#[derive(Debug, Clone, Copy)]
pub struct RecordFiling<'a> {
    /// The athlete's tenant.
    pub tenant_id: &'a TenantId,
    /// The athlete.
    pub user_id: Uuid,
    /// The provider key: the import's source key, or [`LAB_DOCUMENT_PROVIDER`].
    pub provider: &'a str,
    /// The data source the rows are filed under.
    pub data_source_id: &'a str,
}

/// One self-report to store, its fields barrage's `SelfReport`'s.
#[derive(Debug, Clone)]
pub struct SelfReportWrite {
    /// Barrage's stable id for the record.
    pub record_id: String,
    /// What it records (`menstrual_flow`, ...).
    pub kind: String,
    /// When it starts.
    pub start_at: Timestamp,
    /// When it ends.
    pub end_at: Timestamp,
    /// A coded value, for a category.
    pub value_code: Option<String>,
    /// A measured value, for a quantity.
    pub value: Option<f64>,
    /// The measured value's unit.
    pub unit: Option<String>,
    /// The athlete marked it as the start of a cycle.
    pub cycle_start: bool,
    /// Protection recorded as used, for sexual activity.
    pub protection_used: Option<bool>,
    /// The unkeyed id an earlier import stored it under, when this one is
    /// keyed: that row is replaced by this one (carnet#1701).
    pub legacy_id: Option<String>,
}

/// One stored self-report.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredSelfReport {
    /// The row's id.
    pub id: String,
    /// The provider key it was filed under.
    pub provider: String,
    /// What it records.
    pub kind: String,
    /// When it starts.
    pub start_at: Timestamp,
    /// When it ends.
    pub end_at: Timestamp,
    /// A coded value.
    pub value_code: Option<String>,
    /// A measured value.
    pub value: Option<f64>,
    /// Its unit.
    pub unit: Option<String>,
    /// The start of a cycle.
    pub cycle_start: bool,
    /// Protection recorded as used.
    pub protection_used: Option<bool>,
}

/// One clinical record to store, its fields barrage's `ClinicalResource`'s,
/// each JSON field already serialized.
#[derive(Debug, Clone)]
pub struct ClinicalRecordWrite {
    /// Barrage's stable id for the record.
    pub record_id: String,
    /// The FHIR `resourceType`.
    pub resource_type: String,
    /// Its tier, as barrage serializes it (`T2`, `T3`).
    pub tier: String,
    /// Why a code raised it to T3, when one did.
    pub sensitivity_class: Option<String>,
    /// The FHIR version its file states.
    pub fhir_version: String,
    /// The resource as read (JSON).
    pub resource: String,
    /// The resource without free text (JSON): what a model may be served.
    pub stripped: String,
    /// Its R4 Provenance (JSON).
    pub provenance: String,
    /// Its flattened shape, for an Observation (JSON).
    pub flat: Option<String>,
    /// How it came from its file (`verbatim`, `parsed`, `extracted`).
    pub derivation: String,
    /// `none`, or `pending` while it waits for the athlete.
    pub review_state: String,
    /// The checks an extracted record failed (JSON array).
    pub checks: String,
    /// Where in its document it was read (`text:page=P;line=L`).
    pub locator: Option<String>,
    /// The engine that read it from its document.
    pub read_by: Option<String>,
    /// When it was effective, for date filters.
    pub effective_at: Option<Timestamp>,
    /// The lab document it was read from.
    pub lab_document_id: Option<String>,
    /// The unkeyed id an earlier import stored it under, when this one is
    /// keyed: that row is replaced by this one (carnet#1701).
    pub legacy_id: Option<String>,
}

/// One stored clinical record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredClinicalRecord {
    /// The row's id.
    pub id: String,
    /// The provider key it was filed under.
    pub provider: String,
    /// The FHIR `resourceType`.
    pub resource_type: String,
    /// Its tier (`T2`, `T3`).
    pub tier: String,
    /// Why a code raised it to T3.
    pub sensitivity_class: Option<String>,
    /// The FHIR version.
    pub fhir_version: String,
    /// The resource as read (JSON).
    pub resource: String,
    /// The resource without free text (JSON).
    pub stripped: String,
    /// Its R4 Provenance (JSON).
    pub provenance: String,
    /// Its flattened shape (JSON).
    pub flat: Option<String>,
    /// How it came from its file.
    pub derivation: String,
    /// `none` or `pending`.
    pub review_state: String,
    /// The athlete's answer: `confirmed`, `rejected`, or none yet.
    pub review_decision: Option<String>,
    /// The checks an extracted record failed (JSON array).
    pub checks: String,
    /// Where in its document it was read.
    pub locator: Option<String>,
    /// The engine that read it.
    pub read_by: Option<String>,
    /// When it was effective.
    pub effective_at: Option<Timestamp>,
    /// The lab document it was read from.
    pub lab_document_id: Option<String>,
    /// When the athlete answered.
    pub decided_at: Option<Timestamp>,
}

/// Which clinical records a list returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClinicalSelection {
    /// The records that may be served: read as they are, or confirmed by the
    /// athlete. Observations only when `Some(true)`, everything else when
    /// `Some(false)`, both when `None`.
    Served {
        /// Observations, or every other resource type, or both.
        observations: Option<bool>,
    },
    /// The records waiting for the athlete, undecided.
    Pending,
}

/// A date window and a page size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordWindow {
    /// From this instant, inclusive.
    pub since: Option<Timestamp>,
    /// Until this instant, exclusive.
    pub until: Option<Timestamp>,
    /// At most this many rows, capped at [`MAX_HEALTH_RECORDS_PAGE`].
    pub limit: u32,
}

impl RecordWindow {
    /// The page size as bound, within `1..=MAX_HEALTH_RECORDS_PAGE`.
    #[must_use]
    pub fn bound_limit(&self) -> i64 {
        i64::from(self.limit.clamp(1, MAX_HEALTH_RECORDS_PAGE))
    }
}

/// The athlete's answer to a pending record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewDecision {
    /// The record is right: it may be served.
    Confirmed,
    /// The record is wrong: it is served to no one, and stays rejected.
    Rejected,
}

impl ReviewDecision {
    /// The stored value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Confirmed => "confirmed",
            Self::Rejected => "rejected",
        }
    }
}

/// What a lab document's reading yielded, to store with it.
#[derive(Debug, Clone)]
pub struct LabDocumentWrite {
    /// The SHA-256 of the file's bytes.
    pub file_sha256: String,
    /// Its size.
    pub size_bytes: u64,
    /// Its pages.
    pub pages: u32,
    /// `read`, or `no_text` when it holds no text layer.
    pub status: String,
    /// The engine that read it (`rules`, `model:<name>`).
    pub engine: Option<String>,
    /// Lines the injection screen distrusted.
    pub flagged_lines: u32,
    /// Result rows seen but not coded.
    pub skipped: u32,
    /// Model results dropped by verification.
    pub dropped: u32,
    /// Why a supplied model's reading was not used.
    pub model_failure: Option<String>,
    /// When it was received.
    pub received_at: Timestamp,
}

/// Where a lab document's PDF is kept: whose upload, under which hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabDocumentFile {
    /// The tenant it was uploaded in.
    pub tenant_id: TenantId,
    /// The athlete who uploaded it.
    pub user_id: Uuid,
    /// The SHA-256 its bytes are kept under.
    pub file_sha256: String,
}

/// One stored lab document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredLabDocument {
    /// The row's id.
    pub id: String,
    /// The SHA-256 of the file's bytes.
    pub file_sha256: String,
    /// Its size.
    pub size_bytes: u64,
    /// Its pages.
    pub pages: u32,
    /// `read` or `no_text`.
    pub status: String,
    /// The engine that read it.
    pub engine: Option<String>,
    /// Lines the injection screen distrusted.
    pub flagged_lines: u32,
    /// Result rows seen but not coded.
    pub skipped: u32,
    /// Model results dropped by verification.
    pub dropped: u32,
    /// Why a supplied model's reading was not used.
    pub model_failure: Option<String>,
    /// When it was received.
    pub received_at: Timestamp,
}

/// The athlete's health records.
#[async_trait]
pub trait HealthRecordRepository: Send + Sync {
    /// Store `reports` under `filing`, each replacing the row its record id
    /// already holds, and the row its legacy id holds when it carries one: a
    /// re-import that keyed a record's id moves it, never duplicates it.
    /// Returns how many rows were written.
    ///
    /// # Errors
    /// Returns a database error when a write fails.
    async fn upsert_self_reports(
        &self,
        filing: RecordFiling<'_>,
        reports: &[SelfReportWrite],
    ) -> AppResult<u64>;

    /// Store `records` under `filing`, each replacing the content of the row
    /// its record id already holds, never its review decision, and the row
    /// its legacy id holds when it carries one. Returns how many rows were
    /// written.
    ///
    /// # Errors
    /// Returns a database error when a write fails.
    async fn upsert_clinical_records(
        &self,
        filing: RecordFiling<'_>,
        records: &[ClinicalRecordWrite],
    ) -> AppResult<u64>;

    /// Delete the athlete's self-reports filed under `provider` whose record
    /// ids are in `record_ids`: the records their source no longer holds.
    /// Returns how many rows were deleted; an id no row holds deletes nothing.
    ///
    /// # Errors
    /// Returns a database error when a delete fails.
    async fn delete_self_reports(
        &self,
        tenant_id: &TenantId,
        user_id: Uuid,
        provider: &str,
        record_ids: &[String],
    ) -> AppResult<u64>;

    /// The athlete's self-reports starting in `window`, newest first.
    ///
    /// # Errors
    /// Returns a database error when the read fails.
    async fn list_self_reports(
        &self,
        tenant_id: &TenantId,
        user_id: Uuid,
        window: RecordWindow,
    ) -> AppResult<Vec<StoredSelfReport>>;

    /// The athlete's clinical records `selection` names, effective in
    /// `window` (a record with no date is listed whatever the window),
    /// newest first.
    ///
    /// # Errors
    /// Returns a database error when the read fails.
    async fn list_clinical_records(
        &self,
        tenant_id: &TenantId,
        user_id: Uuid,
        selection: ClinicalSelection,
        window: RecordWindow,
    ) -> AppResult<Vec<StoredClinicalRecord>>;

    /// Record the athlete's `decision` on their pending, undecided record
    /// `id`. Returns `false` when they hold no such record.
    ///
    /// # Errors
    /// Returns a database error when the write fails.
    async fn decide_clinical_record(
        &self,
        tenant_id: &TenantId,
        user_id: Uuid,
        id: &str,
        decision: ReviewDecision,
        at: Timestamp,
    ) -> AppResult<bool>;

    /// Store a lab document's row. Returns its id, or `None` when the athlete
    /// already holds the same file, which is left as it is.
    ///
    /// # Errors
    /// Returns a database error when the write fails.
    async fn insert_lab_document(
        &self,
        tenant_id: &TenantId,
        user_id: Uuid,
        document: &LabDocumentWrite,
    ) -> AppResult<Option<String>>;

    /// The athlete's lab document holding the file `file_sha256`.
    ///
    /// # Errors
    /// Returns a database error when the read fails.
    async fn find_lab_document_by_file(
        &self,
        tenant_id: &TenantId,
        user_id: Uuid,
        file_sha256: &str,
    ) -> AppResult<Option<StoredLabDocument>>;

    /// The athlete's lab document `id`.
    ///
    /// # Errors
    /// Returns a database error when the read fails.
    async fn get_lab_document(
        &self,
        tenant_id: &TenantId,
        user_id: Uuid,
        id: &str,
    ) -> AppResult<Option<StoredLabDocument>>;

    /// The athlete's lab documents, newest first.
    ///
    /// # Errors
    /// Returns a database error when the read fails.
    async fn list_lab_documents(
        &self,
        tenant_id: &TenantId,
        user_id: Uuid,
    ) -> AppResult<Vec<StoredLabDocument>>;

    /// The records read from the athlete's lab document `id`, whatever their
    /// review state.
    ///
    /// # Errors
    /// Returns a database error when the read fails.
    async fn list_lab_document_records(
        &self,
        tenant_id: &TenantId,
        user_id: Uuid,
        id: &str,
    ) -> AppResult<Vec<StoredClinicalRecord>>;

    /// Delete the athlete's lab document `id` and every record read from it.
    /// Returns `false` when they hold no such document.
    ///
    /// # Errors
    /// Returns a database error when the delete fails.
    async fn delete_lab_document(
        &self,
        tenant_id: &TenantId,
        user_id: Uuid,
        id: &str,
    ) -> AppResult<bool>;

    /// Every lab document's file, in every tenant: what a purge of the
    /// [`LAB_DOCUMENT_PROVIDER`] provider deletes from the blob store before
    /// its rows go.
    ///
    /// # Errors
    /// Returns a database error when the read fails.
    async fn list_every_lab_document_file(&self) -> AppResult<Vec<LabDocumentFile>>;
}

/// Store one self-report, replacing the row its record id holds.
pub(crate) const UPSERT_SELF_REPORT_SQL: &str = r"
    INSERT INTO self_reports (
        id, tenant_id, user_id, provider, data_source_id, record_id, kind, start_at, end_at,
        value_code, value, unit, cycle_start, protection_used, created_at, updated_at
    )
    VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $15)
    ON CONFLICT (tenant_id, user_id, provider, record_id) DO UPDATE SET
        data_source_id = excluded.data_source_id,
        kind = excluded.kind,
        start_at = excluded.start_at,
        end_at = excluded.end_at,
        value_code = excluded.value_code,
        value = excluded.value,
        unit = excluded.unit,
        cycle_start = excluded.cycle_start,
        protection_used = excluded.protection_used,
        updated_at = excluded.updated_at";

/// Store one clinical record, replacing the content of the row its record id
/// holds; its review decision is never touched.
pub(crate) const UPSERT_CLINICAL_RECORD_SQL: &str = r"
    INSERT INTO clinical_records (
        id, tenant_id, user_id, provider, data_source_id, record_id, resource_type, tier,
        sensitivity_class, fhir_version, resource, stripped, provenance, flat, derivation,
        review_state, checks, locator, read_by, effective_at, lab_document_id,
        created_at, updated_at
    )
    VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19,
            $20, $21, $22, $22)
    ON CONFLICT (tenant_id, user_id, provider, record_id) DO UPDATE SET
        data_source_id = excluded.data_source_id,
        resource_type = excluded.resource_type,
        tier = excluded.tier,
        sensitivity_class = excluded.sensitivity_class,
        fhir_version = excluded.fhir_version,
        resource = excluded.resource,
        stripped = excluded.stripped,
        provenance = excluded.provenance,
        flat = excluded.flat,
        derivation = excluded.derivation,
        review_state = excluded.review_state,
        checks = excluded.checks,
        locator = excluded.locator,
        read_by = excluded.read_by,
        effective_at = excluded.effective_at,
        lab_document_id = excluded.lab_document_id,
        updated_at = excluded.updated_at";

/// One of the athlete's self-reports under a provider key, by its record id.
pub(crate) const DELETE_SELF_REPORT_SQL: &str = r"
    DELETE FROM self_reports
    WHERE tenant_id = $1 AND user_id = $2 AND provider = $3 AND record_id = $4";

/// One of the athlete's clinical records under a provider key, by its record
/// id: the row a keyed record's legacy id held.
pub(crate) const DELETE_CLINICAL_RECORD_SQL: &str = r"
    DELETE FROM clinical_records
    WHERE tenant_id = $1 AND user_id = $2 AND provider = $3 AND record_id = $4";

/// The legacy id a written record replaces, unless it is the record's own.
pub(crate) fn replaced_legacy<'a>(record_id: &str, legacy_id: Option<&'a str>) -> Option<&'a str> {
    legacy_id.filter(|legacy| *legacy != record_id)
}

/// The athlete's self-reports starting in a window, newest first.
pub(crate) const LIST_SELF_REPORTS_SQL: &str = r"
    SELECT id, provider, kind, start_at, end_at, value_code, value, unit, cycle_start,
           protection_used
    FROM self_reports
    WHERE tenant_id = $1 AND user_id = $2
      AND ($3 IS NULL OR start_at >= $3)
      AND ($4 IS NULL OR start_at < $4)
    ORDER BY start_at DESC, id
    LIMIT $5";

/// Every column a clinical record is read with.
macro_rules! clinical_columns {
    () => {
        "id, provider, resource_type, tier, sensitivity_class, fhir_version, resource, stripped, \
         provenance, flat, derivation, review_state, review_decision, checks, locator, read_by, \
         effective_at, lab_document_id, decided_at"
    };
}

/// The athlete's servable clinical records effective in a window: read as
/// they are, or confirmed. `$5` selects Observations (TRUE), every other
/// type (FALSE), or both (NULL).
pub(crate) const LIST_SERVED_CLINICAL_SQL: &str = concat!(
    "SELECT ",
    clinical_columns!(),
    " FROM clinical_records \
     WHERE tenant_id = $1 AND user_id = $2 \
       AND ((review_state = 'none' AND review_decision IS NULL) OR review_decision = 'confirmed') \
       AND ($3 IS NULL OR effective_at IS NULL OR effective_at >= $3) \
       AND ($4 IS NULL OR effective_at IS NULL OR effective_at < $4) \
       AND ($5 IS NULL OR (resource_type = 'Observation') = $5) \
     ORDER BY effective_at DESC, id \
     LIMIT $6"
);

/// The athlete's records waiting for them, undecided, newest first.
pub(crate) const LIST_PENDING_CLINICAL_SQL: &str = concat!(
    "SELECT ",
    clinical_columns!(),
    " FROM clinical_records \
     WHERE tenant_id = $1 AND user_id = $2 \
       AND review_state = 'pending' AND review_decision IS NULL \
       AND ($3 IS NULL OR effective_at IS NULL OR effective_at >= $3) \
       AND ($4 IS NULL OR effective_at IS NULL OR effective_at < $4) \
     ORDER BY effective_at DESC, id \
     LIMIT $5"
);

/// The records read from one of the athlete's lab documents.
pub(crate) const LIST_LAB_DOCUMENT_RECORDS_SQL: &str = concat!(
    "SELECT ",
    clinical_columns!(),
    " FROM clinical_records \
     WHERE tenant_id = $1 AND user_id = $2 AND lab_document_id = $3 \
     ORDER BY locator, id"
);

/// The athlete's answer to a pending, undecided record of theirs.
pub(crate) const DECIDE_CLINICAL_RECORD_SQL: &str = r"
    UPDATE clinical_records
    SET review_decision = $4, decided_at = $5, updated_at = $5
    WHERE tenant_id = $1 AND user_id = $2 AND id = $3
      AND review_state = 'pending' AND review_decision IS NULL";

/// Store a lab document's row once per file.
pub(crate) const INSERT_LAB_DOCUMENT_SQL: &str = r"
    INSERT INTO lab_documents (
        id, tenant_id, user_id, file_sha256, size_bytes, pages, status, engine, flagged_lines,
        skipped, dropped, model_failure, received_at
    )
    VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
    ON CONFLICT (tenant_id, user_id, file_sha256) DO NOTHING";

/// Every column a lab document is read with.
macro_rules! lab_document_columns {
    () => {
        "id, file_sha256, size_bytes, pages, status, engine, flagged_lines, skipped, dropped, \
         model_failure, received_at"
    };
}

/// One of the athlete's lab documents, by its file.
pub(crate) const FIND_LAB_DOCUMENT_BY_FILE_SQL: &str = concat!(
    "SELECT ",
    lab_document_columns!(),
    " FROM lab_documents WHERE tenant_id = $1 AND user_id = $2 AND file_sha256 = $3"
);

/// One of the athlete's lab documents, by id.
pub(crate) const GET_LAB_DOCUMENT_SQL: &str = concat!(
    "SELECT ",
    lab_document_columns!(),
    " FROM lab_documents WHERE tenant_id = $1 AND user_id = $2 AND id = $3"
);

/// The athlete's lab documents, newest first.
pub(crate) const LIST_LAB_DOCUMENTS_SQL: &str = concat!(
    "SELECT ",
    lab_document_columns!(),
    " FROM lab_documents WHERE tenant_id = $1 AND user_id = $2 \
     ORDER BY received_at DESC, id"
);

/// The records read from one of the athlete's lab documents, deleted with it.
pub(crate) const DELETE_LAB_DOCUMENT_RECORDS_SQL: &str = r"
    DELETE FROM clinical_records
    WHERE tenant_id = $1 AND user_id = $2 AND lab_document_id = $3";

/// One of the athlete's lab documents.
pub(crate) const DELETE_LAB_DOCUMENT_SQL: &str = r"
    DELETE FROM lab_documents WHERE tenant_id = $1 AND user_id = $2 AND id = $3";

/// Every lab document's file, its owner ids as text on both engines.
///
/// ISOLATION EXEMPTION: no `tenant_id` or `user_id` filter, on purpose. It
/// serves only the admin's whole-provider purge of `lab_document`
/// (`PROVIDER_PURGE_SQL`), reachable from the super-admin route that audits
/// the call. No tenant- or user-facing path may run it.
pub(crate) const LIST_EVERY_LAB_DOCUMENT_FILE_SQL: &str = r"
    SELECT CAST(tenant_id AS TEXT) AS tenant_id, CAST(user_id AS TEXT) AS user_id, file_sha256
    FROM lab_documents ORDER BY tenant_id, user_id, id";

/// A count read back from a row, as the domain holds it.
pub(crate) fn stored_count(value: i64) -> u32 {
    u32::try_from(value.max(0)).unwrap_or(u32::MAX)
}

/// Emit the whole [`HealthRecordRepository`] implementation for one backend
/// type. `$ids` is that backend's uuid codec
/// ([`crate::repositories::uuid_columns`]) and `$row` its row type. The body
/// names its consts and types unqualified, so the invoking shell `use`s them.
macro_rules! impl_health_record_repository {
    ($ty:ty, $ids:ident, $row:ty) => {
        /// A column error naming the table.
        fn health_column(table: &'static str) -> impl Fn(sqlx::Error) -> AppError {
            move |e| AppError::database(format!("Invalid {table} column: {e}"))
        }

        /// One `self_reports` row.
        fn self_report_from_row(row: &$row) -> AppResult<StoredSelfReport> {
            let column = health_column("self_reports");
            Ok(StoredSelfReport {
                id: row.try_get("id").map_err(&column)?,
                provider: row.try_get("provider").map_err(&column)?,
                kind: row.try_get("kind").map_err(&column)?,
                start_at: row.try_get("start_at").map_err(&column)?,
                end_at: row.try_get("end_at").map_err(&column)?,
                value_code: row.try_get("value_code").map_err(&column)?,
                value: row.try_get("value").map_err(&column)?,
                unit: row.try_get("unit").map_err(&column)?,
                cycle_start: row.try_get("cycle_start").map_err(&column)?,
                protection_used: row.try_get("protection_used").map_err(&column)?,
            })
        }

        /// One `clinical_records` row.
        fn clinical_record_from_row(row: &$row) -> AppResult<StoredClinicalRecord> {
            let column = health_column("clinical_records");
            Ok(StoredClinicalRecord {
                id: row.try_get("id").map_err(&column)?,
                provider: row.try_get("provider").map_err(&column)?,
                resource_type: row.try_get("resource_type").map_err(&column)?,
                tier: row.try_get("tier").map_err(&column)?,
                sensitivity_class: row.try_get("sensitivity_class").map_err(&column)?,
                fhir_version: row.try_get("fhir_version").map_err(&column)?,
                resource: row.try_get("resource").map_err(&column)?,
                stripped: row.try_get("stripped").map_err(&column)?,
                provenance: row.try_get("provenance").map_err(&column)?,
                flat: row.try_get("flat").map_err(&column)?,
                derivation: row.try_get("derivation").map_err(&column)?,
                review_state: row.try_get("review_state").map_err(&column)?,
                review_decision: row.try_get("review_decision").map_err(&column)?,
                checks: row.try_get("checks").map_err(&column)?,
                locator: row.try_get("locator").map_err(&column)?,
                read_by: row.try_get("read_by").map_err(&column)?,
                effective_at: row.try_get("effective_at").map_err(&column)?,
                lab_document_id: row.try_get("lab_document_id").map_err(&column)?,
                decided_at: row.try_get("decided_at").map_err(&column)?,
            })
        }

        /// One `lab_documents` row.
        fn lab_document_from_row(row: &$row) -> AppResult<StoredLabDocument> {
            let column = health_column("lab_documents");
            let size: i64 = row.try_get("size_bytes").map_err(&column)?;
            let pages: i64 = row.try_get("pages").map_err(&column)?;
            let flagged: i64 = row.try_get("flagged_lines").map_err(&column)?;
            let skipped: i64 = row.try_get("skipped").map_err(&column)?;
            let dropped: i64 = row.try_get("dropped").map_err(&column)?;
            Ok(StoredLabDocument {
                id: row.try_get("id").map_err(&column)?,
                file_sha256: row.try_get("file_sha256").map_err(&column)?,
                size_bytes: u64::try_from(size.max(0)).unwrap_or(0),
                pages: stored_count(pages),
                status: row.try_get("status").map_err(&column)?,
                engine: row.try_get("engine").map_err(&column)?,
                flagged_lines: stored_count(flagged),
                skipped: stored_count(skipped),
                dropped: stored_count(dropped),
                model_failure: row.try_get("model_failure").map_err(&column)?,
                received_at: row.try_get("received_at").map_err(&column)?,
            })
        }

        #[async_trait::async_trait]
        impl HealthRecordRepository for $ty {
            async fn upsert_self_reports(
                &self,
                filing: RecordFiling<'_>,
                reports: &[SelfReportWrite],
            ) -> AppResult<u64> {
                let now = Timestamp::now();
                let failed =
                    |e: sqlx::Error| AppError::database(format!("upsert_self_reports: {e}"));
                let mut written = 0;
                for report in reports {
                    let mut tx = self.pool().begin().await.map_err(failed)?;
                    let done = sqlx::query(UPSERT_SELF_REPORT_SQL)
                        .bind(Uuid::new_v4().to_string())
                        .bind($ids::bind(filing.tenant_id.as_uuid()))
                        .bind($ids::bind(filing.user_id))
                        .bind(filing.provider)
                        .bind(filing.data_source_id)
                        .bind(&report.record_id)
                        .bind(&report.kind)
                        .bind(report.start_at)
                        .bind(report.end_at)
                        .bind(report.value_code.as_deref())
                        .bind(report.value)
                        .bind(report.unit.as_deref())
                        .bind(report.cycle_start)
                        .bind(report.protection_used)
                        .bind(now)
                        .execute(&mut *tx)
                        .await
                        .map_err(failed)?;
                    if let Some(legacy) =
                        replaced_legacy(&report.record_id, report.legacy_id.as_deref())
                    {
                        sqlx::query(DELETE_SELF_REPORT_SQL)
                            .bind($ids::bind(filing.tenant_id.as_uuid()))
                            .bind($ids::bind(filing.user_id))
                            .bind(filing.provider)
                            .bind(legacy)
                            .execute(&mut *tx)
                            .await
                            .map_err(failed)?;
                    }
                    tx.commit().await.map_err(failed)?;
                    written += done.rows_affected();
                }
                Ok(written)
            }

            async fn upsert_clinical_records(
                &self,
                filing: RecordFiling<'_>,
                records: &[ClinicalRecordWrite],
            ) -> AppResult<u64> {
                let now = Timestamp::now();
                let failed =
                    |e: sqlx::Error| AppError::database(format!("upsert_clinical_records: {e}"));
                let mut written = 0;
                for record in records {
                    let mut tx = self.pool().begin().await.map_err(failed)?;
                    let done = sqlx::query(UPSERT_CLINICAL_RECORD_SQL)
                        .bind(Uuid::new_v4().to_string())
                        .bind($ids::bind(filing.tenant_id.as_uuid()))
                        .bind($ids::bind(filing.user_id))
                        .bind(filing.provider)
                        .bind(filing.data_source_id)
                        .bind(&record.record_id)
                        .bind(&record.resource_type)
                        .bind(&record.tier)
                        .bind(record.sensitivity_class.as_deref())
                        .bind(&record.fhir_version)
                        .bind(&record.resource)
                        .bind(&record.stripped)
                        .bind(&record.provenance)
                        .bind(record.flat.as_deref())
                        .bind(&record.derivation)
                        .bind(&record.review_state)
                        .bind(&record.checks)
                        .bind(record.locator.as_deref())
                        .bind(record.read_by.as_deref())
                        .bind(record.effective_at)
                        .bind(record.lab_document_id.as_deref())
                        .bind(now)
                        .execute(&mut *tx)
                        .await
                        .map_err(failed)?;
                    if let Some(legacy) =
                        replaced_legacy(&record.record_id, record.legacy_id.as_deref())
                    {
                        sqlx::query(DELETE_CLINICAL_RECORD_SQL)
                            .bind($ids::bind(filing.tenant_id.as_uuid()))
                            .bind($ids::bind(filing.user_id))
                            .bind(filing.provider)
                            .bind(legacy)
                            .execute(&mut *tx)
                            .await
                            .map_err(failed)?;
                    }
                    tx.commit().await.map_err(failed)?;
                    written += done.rows_affected();
                }
                Ok(written)
            }

            async fn delete_self_reports(
                &self,
                tenant_id: &TenantId,
                user_id: Uuid,
                provider: &str,
                record_ids: &[String],
            ) -> AppResult<u64> {
                if record_ids.is_empty() {
                    return Ok(0);
                }
                let failed =
                    |e: sqlx::Error| AppError::database(format!("delete_self_reports: {e}"));
                let mut tx = self.pool().begin().await.map_err(failed)?;
                let mut deleted = 0;
                for record_id in record_ids {
                    let done = sqlx::query(DELETE_SELF_REPORT_SQL)
                        .bind($ids::bind(tenant_id.as_uuid()))
                        .bind($ids::bind(user_id))
                        .bind(provider)
                        .bind(record_id)
                        .execute(&mut *tx)
                        .await
                        .map_err(failed)?;
                    deleted += done.rows_affected();
                }
                tx.commit().await.map_err(failed)?;
                Ok(deleted)
            }

            async fn list_self_reports(
                &self,
                tenant_id: &TenantId,
                user_id: Uuid,
                window: RecordWindow,
            ) -> AppResult<Vec<StoredSelfReport>> {
                let rows = sqlx::query(LIST_SELF_REPORTS_SQL)
                    .bind($ids::bind(tenant_id.as_uuid()))
                    .bind($ids::bind(user_id))
                    .bind(window.since)
                    .bind(window.until)
                    .bind(window.bound_limit())
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("list_self_reports: {e}")))?;
                rows.iter().map(self_report_from_row).collect()
            }

            async fn list_clinical_records(
                &self,
                tenant_id: &TenantId,
                user_id: Uuid,
                selection: ClinicalSelection,
                window: RecordWindow,
            ) -> AppResult<Vec<StoredClinicalRecord>> {
                let query = match selection {
                    ClinicalSelection::Served { observations } => {
                        sqlx::query(LIST_SERVED_CLINICAL_SQL)
                            .bind($ids::bind(tenant_id.as_uuid()))
                            .bind($ids::bind(user_id))
                            .bind(window.since)
                            .bind(window.until)
                            .bind(observations)
                            .bind(window.bound_limit())
                    }
                    ClinicalSelection::Pending => sqlx::query(LIST_PENDING_CLINICAL_SQL)
                        .bind($ids::bind(tenant_id.as_uuid()))
                        .bind($ids::bind(user_id))
                        .bind(window.since)
                        .bind(window.until)
                        .bind(window.bound_limit()),
                };
                let rows = query
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("list_clinical_records: {e}")))?;
                rows.iter().map(clinical_record_from_row).collect()
            }

            async fn decide_clinical_record(
                &self,
                tenant_id: &TenantId,
                user_id: Uuid,
                id: &str,
                decision: ReviewDecision,
                at: Timestamp,
            ) -> AppResult<bool> {
                let done = sqlx::query(DECIDE_CLINICAL_RECORD_SQL)
                    .bind($ids::bind(tenant_id.as_uuid()))
                    .bind($ids::bind(user_id))
                    .bind(id)
                    .bind(decision.as_str())
                    .bind(at)
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("decide_clinical_record: {e}")))?;
                Ok(done.rows_affected() > 0)
            }

            async fn insert_lab_document(
                &self,
                tenant_id: &TenantId,
                user_id: Uuid,
                document: &LabDocumentWrite,
            ) -> AppResult<Option<String>> {
                let id = Uuid::new_v4().to_string();
                let done = sqlx::query(INSERT_LAB_DOCUMENT_SQL)
                    .bind(&id)
                    .bind($ids::bind(tenant_id.as_uuid()))
                    .bind($ids::bind(user_id))
                    .bind(&document.file_sha256)
                    .bind(i64::try_from(document.size_bytes).unwrap_or(i64::MAX))
                    .bind(i64::from(document.pages))
                    .bind(&document.status)
                    .bind(document.engine.as_deref())
                    .bind(i64::from(document.flagged_lines))
                    .bind(i64::from(document.skipped))
                    .bind(i64::from(document.dropped))
                    .bind(document.model_failure.as_deref())
                    .bind(document.received_at)
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("insert_lab_document: {e}")))?;
                Ok((done.rows_affected() > 0).then_some(id))
            }

            async fn find_lab_document_by_file(
                &self,
                tenant_id: &TenantId,
                user_id: Uuid,
                file_sha256: &str,
            ) -> AppResult<Option<StoredLabDocument>> {
                let row = sqlx::query(FIND_LAB_DOCUMENT_BY_FILE_SQL)
                    .bind($ids::bind(tenant_id.as_uuid()))
                    .bind($ids::bind(user_id))
                    .bind(file_sha256)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("find_lab_document_by_file: {e}")))?;
                row.as_ref().map(lab_document_from_row).transpose()
            }

            async fn get_lab_document(
                &self,
                tenant_id: &TenantId,
                user_id: Uuid,
                id: &str,
            ) -> AppResult<Option<StoredLabDocument>> {
                let row = sqlx::query(GET_LAB_DOCUMENT_SQL)
                    .bind($ids::bind(tenant_id.as_uuid()))
                    .bind($ids::bind(user_id))
                    .bind(id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("get_lab_document: {e}")))?;
                row.as_ref().map(lab_document_from_row).transpose()
            }

            async fn list_lab_documents(
                &self,
                tenant_id: &TenantId,
                user_id: Uuid,
            ) -> AppResult<Vec<StoredLabDocument>> {
                let rows = sqlx::query(LIST_LAB_DOCUMENTS_SQL)
                    .bind($ids::bind(tenant_id.as_uuid()))
                    .bind($ids::bind(user_id))
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("list_lab_documents: {e}")))?;
                rows.iter().map(lab_document_from_row).collect()
            }

            async fn list_lab_document_records(
                &self,
                tenant_id: &TenantId,
                user_id: Uuid,
                id: &str,
            ) -> AppResult<Vec<StoredClinicalRecord>> {
                let rows = sqlx::query(LIST_LAB_DOCUMENT_RECORDS_SQL)
                    .bind($ids::bind(tenant_id.as_uuid()))
                    .bind($ids::bind(user_id))
                    .bind(id)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("list_lab_document_records: {e}")))?;
                rows.iter().map(clinical_record_from_row).collect()
            }

            async fn delete_lab_document(
                &self,
                tenant_id: &TenantId,
                user_id: Uuid,
                id: &str,
            ) -> AppResult<bool> {
                let mut tx = self
                    .pool()
                    .begin()
                    .await
                    .map_err(|e| AppError::database(format!("delete_lab_document: {e}")))?;
                sqlx::query(DELETE_LAB_DOCUMENT_RECORDS_SQL)
                    .bind($ids::bind(tenant_id.as_uuid()))
                    .bind($ids::bind(user_id))
                    .bind(id)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| AppError::database(format!("delete_lab_document records: {e}")))?;
                let done = sqlx::query(DELETE_LAB_DOCUMENT_SQL)
                    .bind($ids::bind(tenant_id.as_uuid()))
                    .bind($ids::bind(user_id))
                    .bind(id)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| AppError::database(format!("delete_lab_document: {e}")))?;
                tx.commit()
                    .await
                    .map_err(|e| AppError::database(format!("delete_lab_document commit: {e}")))?;
                Ok(done.rows_affected() > 0)
            }

            async fn list_every_lab_document_file(&self) -> AppResult<Vec<LabDocumentFile>> {
                let rows = sqlx::query(LIST_EVERY_LAB_DOCUMENT_FILE_SQL)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("list_every_lab_document_file: {e}"))
                    })?;
                let column = health_column("lab_documents");
                let id = |row: &$row, name: &str| -> AppResult<Uuid> {
                    let text: String = row.try_get(name).map_err(&column)?;
                    Uuid::parse_str(&text).map_err(|e| {
                        AppError::database(format!("Invalid lab_documents {name}: {e}"))
                    })
                };
                rows.iter()
                    .map(|row| {
                        Ok(LabDocumentFile {
                            tenant_id: TenantId::from_uuid(id(row, "tenant_id")?),
                            user_id: id(row, "user_id")?,
                            file_sha256: row.try_get("file_sha256").map_err(&column)?,
                        })
                    })
                    .collect()
            }
        }
    };
}

pub(crate) use impl_health_record_repository;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_is_kept_within_one_and_the_cap() {
        let window = |limit| RecordWindow {
            since: None,
            until: None,
            limit,
        };
        assert_eq!(window(0).bound_limit(), 1, "bug: an empty page was bound");
        assert_eq!(window(50).bound_limit(), 50, "bug: a page in range moved");
        assert_eq!(
            window(5_000).bound_limit(),
            i64::from(MAX_HEALTH_RECORDS_PAGE),
            "bug: a page above the cap was bound"
        );
    }

    #[test]
    fn a_decision_is_stored_as_the_migration_checks_it() {
        // The migration is the data under test: its CHECK constraint names the
        // decisions the column admits, and this pins it to the enum's spelling.
        let migration = include_str!("../../../../migrations/20261121120000_health_records.sql");
        for decision in [ReviewDecision::Confirmed, ReviewDecision::Rejected] {
            assert!(
                migration.contains(&format!("'{}'", decision.as_str())),
                "bug: the migration does not admit {}",
                decision.as_str()
            );
        }
    }
}
