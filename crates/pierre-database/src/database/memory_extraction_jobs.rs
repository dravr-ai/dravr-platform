// ABOUTME: SQLite-backed MemoryExtractionJobRepository, emitted from the shared implementation in repositories/memory_extraction_jobs.rs
// ABOUTME: Stale rows are claimed in one UPDATE … RETURNING over a LIMITed subquery, oldest first; SQLite needs no lock clause

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::admin::scope::AdminScope;
use pierre_core::errors::{AppError, AppResult};

use crate::database::Database;
use crate::repositories::memory_extraction_jobs::{
    claim_extraction_jobs_sql, extraction_job_from_row, extraction_run_stats_from_rows,
    impl_memory_extraction_job_repository, run_windows, ExtractionJobClaim, ExtractionRunRecord,
    ExtractionRunStats, JobLease, MemoryExtractionJobRepository, MemoryExtractionJobRow,
    EXTRACTION_BACKLOG_SQL, EXTRACTION_RUN_STATS_SQL, FINISH_EXTRACTION_JOB_SQL,
    PRUNE_EXTRACTION_RUNS_SQL, REAP_EXHAUSTED_EXTRACTION_JOBS_SQL, RECORD_EXTRACTION_JOB_SQL,
    RECORD_EXTRACTION_RUN_SQL, RENEW_EXTRACTION_JOB_LEASE_SQL,
};

impl_memory_extraction_job_repository!(Database, "");
