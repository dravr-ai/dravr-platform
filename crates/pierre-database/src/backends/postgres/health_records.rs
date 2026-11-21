// ABOUTME: PostgreSQL-backed HealthRecordRepository, emitted from the shared body in repositories/
// ABOUTME: Native uuid ids through the NativeUuid codec; the statements are the ones the other backend runs

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use sqlx::postgres::PgRow;
use sqlx::Row;
use uuid::Uuid;

use crate::backends::postgres::PostgresDatabase;
use crate::repositories::health_records::{
    impl_health_record_repository, stored_count, ClinicalRecordWrite, ClinicalSelection,
    HealthRecordRepository, LabDocumentFile, LabDocumentWrite, RecordFiling, RecordWindow, ReviewDecision,
    SelfReportWrite, StoredClinicalRecord, StoredLabDocument, StoredSelfReport,
    DECIDE_CLINICAL_RECORD_SQL, DELETE_LAB_DOCUMENT_RECORDS_SQL, DELETE_LAB_DOCUMENT_SQL,
    FIND_LAB_DOCUMENT_BY_FILE_SQL, GET_LAB_DOCUMENT_SQL, INSERT_LAB_DOCUMENT_SQL,
    LIST_EVERY_LAB_DOCUMENT_FILE_SQL, LIST_LAB_DOCUMENTS_SQL, LIST_LAB_DOCUMENT_RECORDS_SQL, LIST_PENDING_CLINICAL_SQL,
    LIST_SELF_REPORTS_SQL, LIST_SERVED_CLINICAL_SQL, UPSERT_CLINICAL_RECORD_SQL,
    UPSERT_SELF_REPORT_SQL,
};
use crate::repositories::uuid_columns::NativeUuid;

impl_health_record_repository!(PostgresDatabase, NativeUuid, PgRow);
