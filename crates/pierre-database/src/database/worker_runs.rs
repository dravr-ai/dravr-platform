// ABOUTME: SQLite-backed WorkerRunRepository, emitted from the shared implementation in repositories/worker_runs.rs
// ABOUTME: One row per periodic worker; the claim is one INSERT … ON CONFLICT DO UPDATE whose WHERE carries the due-and-unheld test

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::{AppError, AppResult};

use crate::database::Database;
use crate::repositories::worker_runs::{
    impl_worker_run_repository, worker_run_from_row, WorkerRun, WorkerRunRepository,
    CLAIM_WORKER_RUN_SQL, DEFER_WORKER_RUN_SQL, FINISH_WORKER_RUN_SQL, GET_WORKER_RUN_SQL,
};

impl_worker_run_repository!(Database);
