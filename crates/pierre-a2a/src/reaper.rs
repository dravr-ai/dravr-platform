// ABOUTME: Periodic reaper for A2A tasks left non-terminal by an instance that died mid-run
// ABOUTME: Fails the stale rows, closes their stream channels, and delivers the terminal push

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Fails the tasks an instance died holding.
//!
//! A `SendMessage` with `returnImmediately`, or a `SendStreamingMessage`
//! whose client has dropped, runs its tool detached from any request. When
//! the instance is killed mid-run the task row stays `working`: nothing
//! else ever writes it, `GetTask` reports it as running forever, a later
//! `SubscribeToTask` waits on a process-local channel nobody will close,
//! and the push webhooks never fire (carnet#462).
//!
//! [`reap_stale_tasks`] runs on a period (the server spawns it on the
//! shared worker ledger, every [`REAP_INTERVAL`]). Any task still `submitted` or
//! `working` whose status has not moved for [`STALE_AFTER`] is failed with a
//! message that says what happened, its stream channel on this instance is
//! closed so a waiting subscriber gets the terminal event, and the push
//! configs are notified exactly as a finished run notifies them. The
//! threshold is far past any registered tool's runtime — a tool call is a
//! database read or a provider fetch, and the deep-history backfill spawns
//! its own job and returns — so a task that old is one whose runner is gone.

use std::sync::Arc;
use std::time::Duration;

use pierre_core::errors::AppResult;
use pierre_runtime_context::A2ACtx;
use serde_json::to_value;
use tracing::{info, warn};
use uuid::Uuid;

use crate::events::TASK_EVENTS;
use crate::protocol::A2AServer;
use crate::protocol_types::{Message, Part, TaskState};

/// How long a task may sit non-terminal before the reaper fails it.
pub const STALE_AFTER: Duration = Duration::from_mins(10);

/// How often the reaper looks.
pub const REAP_INTERVAL: Duration = Duration::from_mins(5);

/// What the failed task's status message says.
pub const ORPHANED_MESSAGE: &str = "The instance running this task stopped before it \
                                    finished; the task has been failed and may be resubmitted.";

/// The agent reply a reaped task carries, addressed to the task when its ids
/// are known (the stream event) and bare when they are not (the bulk stamp).
fn orphaned_reply(task_id: Option<&str>, context_id: Option<&str>) -> Message {
    let mut reply = Message::agent(
        Uuid::new_v4().to_string(),
        vec![Part::text(ORPHANED_MESSAGE.to_owned())],
    );
    reply.task_id = task_id.map(str::to_owned);
    reply.context_id = context_id.map(str::to_owned);
    reply
}

/// Fail every task that has been non-terminal longer than `stale_after`,
/// close its stream channel, and deliver its push notifications. Returns
/// how many tasks were reaped.
///
/// # Errors
///
/// Returns the repository error when the stale rows cannot be updated or
/// the status message cannot be serialised; a task whose follow-up (stream
/// event, push) fails is logged and still counted.
pub async fn reap_stale_tasks(ctx: &Arc<dyn A2ACtx>, stale_after: Duration) -> AppResult<usize> {
    let stale_secs = i64::try_from(stale_after.as_secs()).unwrap_or(i64::MAX);
    let status_message = to_value(orphaned_reply(None, None))?;
    let reaped = ctx
        .repos()
        .a2a_task_reaper
        .fail_stale_tasks(stale_secs, &status_message)
        .await?;

    for task_id in &reaped {
        let context_id = match ctx.repos().a2a.get_task(task_id).await {
            Ok(Some(task)) => task.context_id.unwrap_or_default(),
            Ok(None) => String::new(),
            Err(e) => {
                warn!(task_id, error = %e, "reaped A2A task could not be re-read for its stream event");
                String::new()
            }
        };
        A2AServer::publish_status(
            task_id,
            &context_id,
            TaskState::Failed,
            Some(orphaned_reply(Some(task_id), Some(&context_id))),
        );
        TASK_EVENTS.close(task_id);
        A2AServer::notify_webhooks(ctx, task_id).await;
    }

    if !reaped.is_empty() {
        info!(
            reaped = reaped.len(),
            stale_after_secs = stale_after.as_secs(),
            "failed A2A tasks orphaned by a dead instance"
        );
    }
    Ok(reaped.len())
}
