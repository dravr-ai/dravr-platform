// ABOUTME: Per-call cooperative cancel flag for tool executions running behind an MCP task handle
// ABOUTME: Task-local so it crosses the tronc McpTool trait boundary without a signature change
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Cooperative cancellation for task-handled tool calls.
//!
//! The MCP Tasks extension's `tasks/cancel` is cooperative: the engine
//! transitions the task row to `cancelled` and the host's worker is expected
//! to notice and stop. The worker here is a spawned `tools/call` dispatch
//! whose tool signature (`McpTool::execute`) is tronc's and cannot grow a
//! cancellation parameter — so the flag travels as a task-local scoped
//! around the whole spawned future by the dispatcher, and any tool body with
//! a natural stopping point (a per-entry write loop) polls it between units
//! of work. A call running inline (no task handle) has no scope and the
//! probe reads as not-cancelled.

use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

tokio::task_local! {
    /// The cancel flag for the current task-handled dispatch, when one exists.
    static MCP_TASK_CANCEL_FLAG: Arc<AtomicBool>;
}

/// Run `fut` with `flag` installed as the current dispatch's cancel flag.
///
/// The dispatcher wraps the spawned tool call in this before racing the
/// fast-path budget; the settle follower sets the flag when it observes the
/// engine's `cancelled` transition in the task store.
pub fn scoped_with_cancel_flag<F: Future>(
    flag: Arc<AtomicBool>,
    fut: F,
) -> impl Future<Output = F::Output> {
    MCP_TASK_CANCEL_FLAG.scope(flag, fut)
}

/// The current dispatch's cancel flag, or `None` when this execution does not
/// run behind a task handle.
#[must_use]
pub fn current_task_cancel_flag() -> Option<Arc<AtomicBool>> {
    MCP_TASK_CANCEL_FLAG.try_with(Clone::clone).ok()
}

/// Whether cancellation was requested for the current dispatch. `false` when
/// no flag is scoped (an inline, non-task execution).
#[must_use]
pub fn task_cancel_requested() -> bool {
    MCP_TASK_CANCEL_FLAG
        .try_with(|flag| flag.load(Ordering::Relaxed))
        .unwrap_or(false)
}
