// ABOUTME: SQLite-backed A2ATaskReaperRepository, emitted from the shared implementation in repositories/a2a_task_reaper.rs
// ABOUTME: Its clock is datetime('now'); create_task writes RFC 3339 and transitions write datetime('now'), so datetime() normalises both

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::{AppError, AppResult};
use serde_json::Value;

use crate::database::Database;
use crate::repositories::a2a_task_reaper::{
    fail_stale_tasks_sql, impl_a2a_task_reaper_repository, task_id_from_row,
    A2ATaskReaperRepository,
};

impl_a2a_task_reaper_repository!(
    Database,
    fail_stale_tasks_sql!(
        "datetime('now')",
        "$1",
        "datetime(updated_at) < datetime('now', '-' || $2 || ' seconds')"
    )
);
