// ABOUTME: PostgreSQL-backed ActivityBackfillJobRepository, emitted from the shared implementation in repositories/activity_backfill_jobs.rs
// ABOUTME: One row per (user, provider, kind); the claim subquery takes FOR UPDATE SKIP LOCKED so a second sweep at the same instant takes other rows

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use sqlx::Row;
use uuid::Uuid;

use crate::backends::postgres::PostgresDatabase;
use crate::repositories::activity_backfill_jobs::{
    backfill_job_from_row, backfill_job_state_of, claim_backfill_jobs_sql,
    impl_activity_backfill_job_repository, incomplete_outcome, job_progress_from_row,
    ActivityBackfillJobRepository, ActivityBackfillJobRow, BackfillJobClaim, BackfillJobKind,
    BackfillJobState, JobProgress, JobProgressStep, BACKFILL_JOB_STATE_SQL, END_BACKFILL_JOB_SQL,
    FINISH_BACKFILL_JOB_SQL, JOB_PROGRESS_SQL, REAP_EXHAUSTED_BACKFILL_JOBS_SQL,
    RECORD_BACKFILL_JOB_SQL, RECORD_JOB_PROGRESS_SQL, RECORD_LIVE_READ_SQL,
    RELEASE_BACKFILL_JOB_CLAIM_SQL, RENEW_BACKFILL_JOB_LEASE_SQL,
};
use crate::repositories::memory_extraction_jobs::JobLease;

impl_activity_backfill_job_repository!(PostgresDatabase, "FOR UPDATE SKIP LOCKED ");
