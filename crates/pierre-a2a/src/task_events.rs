// ABOUTME: The two ways a task's state leaves the process — the stream event bus and the push webhooks
// ABOUTME: Shared by the protocol's terminal transitions and the reaper that ends tasks a dead instance left

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use pierre_runtime_context::A2ACtx;
use tracing::error;

use crate::events::TASK_EVENTS;
use crate::protocol::A2AServer;
use crate::protocol_types::{
    Message, StreamResponse, TaskState, TaskStatusUpdateEvent, WireTaskStatus,
};
use crate::push;

impl A2AServer {
    /// Publish a status-update stream event.
    pub(crate) fn publish_status(
        task_id: &str,
        context_id: &str,
        state: TaskState,
        message: Option<Message>,
    ) {
        TASK_EVENTS.publish(
            task_id,
            &StreamResponse::StatusUpdate(TaskStatusUpdateEvent {
                task_id: task_id.to_owned(),
                context_id: context_id.to_owned(),
                status: WireTaskStatus {
                    state,
                    message,
                    timestamp: Some(Self::now_timestamp()),
                },
                metadata: None,
            }),
        );
    }

    /// Deliver the task's current state to its registered webhooks.
    pub(crate) async fn notify_webhooks(ctx: &Arc<dyn A2ACtx>, task_id: &str) {
        let repos = ctx.repos();
        let configs = match repos.a2a.list_push_configs(task_id).await {
            Ok(configs) if !configs.is_empty() => configs,
            Ok(_) => return,
            Err(e) => {
                error!("Failed to load A2A push configs: {e}");
                return;
            }
        };

        let Ok(Some(record)) = repos.a2a.get_task(task_id).await else {
            return;
        };
        let task = Self::wire_task(&record, None, true);
        let event = StreamResponse::StatusUpdate(TaskStatusUpdateEvent {
            task_id: task.id.clone(), // Safe: String ownership for event
            context_id: task.context_id.clone().unwrap_or_default(), // Safe: String ownership for event
            status: task.status,
            metadata: None,
        });
        push::deliver_task_update(&configs, &event).await;
    }
}
