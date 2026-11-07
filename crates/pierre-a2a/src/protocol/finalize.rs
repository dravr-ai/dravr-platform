// ABOUTME: A2A task terminal transitions: the agent's reply, the persisted wire state, the final stream events
// ABOUTME: Shared by every intent a message can carry, so a tool result and a coach's reply end a task alike

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use pierre_runtime_context::A2ACtx;
use serde_json::{to_value, Value};
use tracing::error;
use uuid::Uuid;

use super::{A2AServer, SendFailure};
use crate::events::TASK_EVENTS;
use crate::protocol_types::{
    Artifact, Message, Part, StreamResponse, TaskArtifactUpdateEvent, TaskState,
};

impl A2AServer {
    /// Build the agent reply for a terminal transition, stamped with the
    /// task/context ids and appended to the history.
    fn build_reply(
        task_id: &str,
        context_id: &str,
        text: String,
        history: &mut Vec<Message>,
    ) -> Message {
        Self::build_reply_with(task_id, context_id, text, Vec::new(), history)
    }

    /// [`Self::build_reply`] with `extra_parts` after the text.
    fn build_reply_with(
        task_id: &str,
        context_id: &str,
        text: String,
        extra_parts: Vec<Part>,
        history: &mut Vec<Message>,
    ) -> Message {
        let mut parts = vec![Part::text(text)];
        parts.extend(extra_parts);
        let mut reply = Message::agent(Uuid::new_v4().to_string(), parts);
        reply.task_id = Some(task_id.to_owned());
        reply.context_id = Some(context_id.to_owned());
        history.push(reply.clone()); // Safe: Message ownership for history
        reply
    }

    /// Terminal `failed` transition: explanatory agent reply, persisted
    /// history and status, and a status-update stream event.
    pub(super) async fn finalize_failed(
        ctx: &Arc<dyn A2ACtx>,
        task_id: &str,
        context_id: &str,
        mut history: Vec<Message>,
        failure: SendFailure,
    ) {
        let SendFailure { reason, data } = failure;
        let extra_parts = data.iter().cloned().map(Part::data).collect();
        let reply = Self::build_reply_with(task_id, context_id, reason, extra_parts, &mut history);
        Self::persist_history(ctx, task_id, &history).await;

        let status_message = to_value(&reply).ok();
        if let Err(e) = ctx
            .repos()
            .a2a
            .update_task_status(
                task_id,
                &TaskState::Failed,
                data.as_ref(),
                status_message.as_ref(),
            )
            .await
        {
            error!("Failed to persist A2A task failure: {e}");
        }
        Self::publish_status(task_id, context_id, TaskState::Failed, Some(reply));
    }

    /// Terminal `completed` transition: agent reply, optional artifact with
    /// its stream event, persisted wire state and status, and the final
    /// status-update stream event.
    pub(super) async fn finalize_completed(
        ctx: &Arc<dyn A2ACtx>,
        task_id: &str,
        context_id: &str,
        mut history: Vec<Message>,
        artifact: Option<Artifact>,
        reply_text: String,
        result: Option<Value>,
    ) {
        let reply = Self::build_reply(task_id, context_id, reply_text, &mut history);

        let artifacts_json = artifact
            .as_ref()
            .and_then(|artifact| to_value(vec![artifact]).ok());
        let history_json = to_value(&history).ok();
        if let Err(e) = ctx
            .repos()
            .a2a
            .update_task_wire_state(task_id, history_json.as_ref(), artifacts_json.as_ref())
            .await
        {
            error!("Failed to persist A2A task wire state: {e}");
        }

        if let Some(artifact) = artifact {
            TASK_EVENTS.publish(
                task_id,
                &StreamResponse::ArtifactUpdate(TaskArtifactUpdateEvent {
                    task_id: task_id.to_owned(),
                    context_id: context_id.to_owned(),
                    artifact,
                    append: false,
                    last_chunk: true,
                    metadata: None,
                }),
            );
        }

        let status_message = to_value(&reply).ok();
        if let Err(e) = ctx
            .repos()
            .a2a
            .update_task_status(
                task_id,
                &TaskState::Completed,
                result.as_ref(),
                status_message.as_ref(),
            )
            .await
        {
            error!("Failed to persist A2A task completion: {e}");
        }
        Self::publish_status(task_id, context_id, TaskState::Completed, Some(reply));
    }

    /// Persist the task's message history, logging (not propagating) failures.
    async fn persist_history(ctx: &Arc<dyn A2ACtx>, task_id: &str, history: &[Message]) {
        if let Ok(history_json) = to_value(history) {
            if let Err(e) = ctx
                .repos()
                .a2a
                .update_task_wire_state(task_id, Some(&history_json), None)
                .await
            {
                error!("Failed to persist A2A task history: {e}");
            }
        }
    }
}
