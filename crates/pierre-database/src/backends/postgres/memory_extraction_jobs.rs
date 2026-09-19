// ABOUTME: PostgreSQL-backed MemoryExtractionJobRepository, emitted from the shared implementation in repositories/memory_extraction_jobs.rs
// ABOUTME: The claim subquery takes FOR UPDATE SKIP LOCKED so a second sweep at the same instant takes other rows

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::{AppError, AppResult};

use crate::backends::postgres::PostgresDatabase;
use crate::repositories::memory_extraction_jobs::{
    claim_extraction_jobs_sql, extraction_job_from_row, impl_memory_extraction_job_repository,
    ExtractionJobClaim, MemoryExtractionJobRepository, MemoryExtractionJobRow,
    FINISH_EXTRACTION_JOB_SQL, REAP_EXHAUSTED_EXTRACTION_JOBS_SQL, RECORD_EXTRACTION_JOB_SQL,
};

impl_memory_extraction_job_repository!(PostgresDatabase, "FOR UPDATE SKIP LOCKED ");
