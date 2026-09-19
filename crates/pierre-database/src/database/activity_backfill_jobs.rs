// ABOUTME: SQLite-backed ActivityBackfillJobRepository, emitted from the shared implementation in repositories/activity_backfill_jobs.rs
// ABOUTME: One row per (user, provider); stale rows are claimed in one UPDATE … RETURNING over a LIMITed subquery; SQLite needs no lock clause

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::{AppError, AppResult};

use crate::database::Database;
use crate::repositories::activity_backfill_jobs::{
    backfill_job_from_row, claim_backfill_jobs_sql, impl_activity_backfill_job_repository,
    ActivityBackfillJobRepository, ActivityBackfillJobRow, BackfillJobClaim,
    FINISH_BACKFILL_JOB_SQL, REAP_EXHAUSTED_BACKFILL_JOBS_SQL, RECORD_BACKFILL_JOB_SQL,
    RENEW_BACKFILL_JOB_LEASE_SQL,
};

impl_activity_backfill_job_repository!(Database, "");
