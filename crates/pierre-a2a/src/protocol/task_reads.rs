// ABOUTME: Serves a stored A2A task to its owner only as the one read rule lets them read what it holds now
// ABOUTME: GetTask, ListTasks, SubscribeToTask and CancelTask withhold a result or reply that read a member they no longer read

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A task keeps what its call produced: a tool call's result as its
//! artifact, a coaching text turn's reply as its artifact, its status
//! message and its history's agent messages. A coach's call that read a
//! member stores that member's data there, so a later read of the task is
//! one more read of the member: it is decided by the task rule the data
//! export applies ([`task_reading`], carnet#1511 calls 2 and 7), never by the
//! client's ownership alone. A task withheld from its owner is served with
//! its id, state and context, and the owner's own messages; everything the
//! agent produced is left out. A rule that cannot be decided withholds.

use pierre_services::task_reading::{self, StoredTask, TaskReading};
use pierre_services::transcript_reading;
use pierre_tool_runtime::read_subject::may_read_others;
use tracing::warn;
use uuid::Uuid;

use super::{A2AResources, A2AServer};
use crate::protocol_types::{Role, Task};
use pierre_core::errors::AppResult;
use pierre_core::models::a2a::A2ATask;

impl A2AServer {
    /// The wire task for `record` as `reader` may read it now: whole, or with
    /// what the agent produced left out (module docs).
    pub(super) async fn readable_wire_task(
        resources: &A2AResources,
        reader: Uuid,
        record: &A2ATask,
        history_length: Option<u32>,
        include_artifacts: bool,
    ) -> Task {
        let mut task = Self::wire_task(record, history_length, include_artifacts);
        let withheld = Self::task_withheld(resources, reader, record)
            .await
            .unwrap_or_else(|e| {
                warn!(error = %e, task_id = %record.id, "whether the owner may read an A2A task could not be decided; withheld");
                true
            });
        if withheld {
            task.artifacts.clear();
            task.status.message = None;
            task.history.retain(|message| message.role == Role::User);
        }
        task
    }

    /// Whether `reader` is withheld what `record` holds, by the one task rule.
    async fn task_withheld(
        resources: &A2AResources,
        reader: Uuid,
        record: &A2ATask,
    ) -> AppResult<bool> {
        let repos = resources.ctx.repos();
        let stamp = repos.a2a.get_task_members(&record.id).await?;
        let task = StoredTask {
            about_members: stamp.as_deref(),
            updated_at: Some(record.updated_at),
            input: Some(&record.input_data),
            context_id: record.context_id.as_deref(),
        };
        let registry = resources.tool_runtime.tool_registry();
        let own = |tool: &str| !may_read_others(registry, tool);
        match task_reading::task_reading(repos, reader, &task, own).await? {
            TaskReading::Kept => Ok(false),
            TaskReading::Withheld => Ok(true),
            TaskReading::ByThread { context_id, ended } => {
                let reader_id = reader.to_string();
                for tenant in repos.tenants.list_membership_tenant_ids(reader).await? {
                    let Some(conversation) = repos
                        .chat
                        .get_conversation(&context_id, &reader_id, tenant)
                        .await?
                    else {
                        continue;
                    };
                    return Ok(transcript_reading::withheld_rows(
                        repos,
                        &conversation,
                        reader,
                        tenant,
                    )
                    .await?
                    .withheld_through(ended));
                }
                // The thread is no conversation the reader reads: what the
                // reply drew on cannot be told.
                Ok(true)
            }
        }
    }
}
