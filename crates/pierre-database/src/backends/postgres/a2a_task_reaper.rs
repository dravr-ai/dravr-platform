// ABOUTME: PostgreSQL-backed A2ATaskReaperRepository, emitted from the shared implementation in repositories/a2a_task_reaper.rs
// ABOUTME: Its clock is NOW(), the status message is cast to jsonb, and staleness is an interval arithmetic on updated_at

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::{AppError, AppResult};
use serde_json::Value;

use crate::backends::postgres::PostgresDatabase;
use crate::repositories::a2a_task_reaper::{
    fail_stale_tasks_sql, impl_a2a_task_reaper_repository, task_id_from_row,
    A2ATaskReaperRepository,
};

impl_a2a_task_reaper_repository!(
    PostgresDatabase,
    fail_stale_tasks_sql!(
        "NOW()",
        "$1::jsonb",
        "updated_at < NOW() - ($2 * INTERVAL '1 second')"
    )
);
