// ABOUTME: PostgreSQL-backed McpTaskRepository, emitted from the shared implementation
// ABOUTME: Persists MCP Tasks extension handles with owner-scoped lookups and TTL expiry
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::{AppError, AppResult};

use super::PostgresDatabase;
use crate::repositories::mcp_tasks::{
    impl_mcp_task_repository, task_from_row, McpTaskRepository, McpTaskRow, GET_TASK_SQL,
    SWEEP_EXPIRED_TASKS_SQL, UPSERT_TASK_SQL,
};

impl_mcp_task_repository!(PostgresDatabase);
