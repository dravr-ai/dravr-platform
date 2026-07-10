// ABOUTME: In-process per-task event bus backing A2A streaming (SSE) subscriptions
// ABOUTME: Publishes StreamResponse frames to subscribers; channel removal closes the stream
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Task event bus for A2A 1.0 streaming.
//!
//! `SendStreamingMessage` / `SubscribeToTask` subscribers receive
//! [`StreamResponse`] frames published by the task executor. Dropping a
//! task's sender (via [`TaskEventBus::close`]) ends every subscriber's
//! stream — the A2A 1.0 signal that the task reached a terminal state
//! (replacing the pre-1.0 `final` flag).
//!
//! The bus is process-local: subscribers on other replicas do not receive
//! events (the same scope the MCP SSE hub has today).

use std::collections::HashMap;
use std::sync::{LazyLock, PoisonError, RwLock};

use tokio::sync::broadcast;
use tracing::debug;

use crate::protocol_types::StreamResponse;

/// Buffered events per task channel; slow subscribers past this lag miss
/// intermediate frames (they always observe the terminal state via closure).
const CHANNEL_CAPACITY: usize = 64;

/// Per-task broadcast hub for A2A streaming events.
pub struct TaskEventBus {
    /// task id → live broadcast sender. Entry removal closes all receivers.
    channels: RwLock<HashMap<String, broadcast::Sender<StreamResponse>>>,
}

/// Process-wide task event bus shared by the JSON-RPC and HTTP+JSON bindings.
pub static TASK_EVENTS: LazyLock<TaskEventBus> = LazyLock::new(TaskEventBus::new);

impl TaskEventBus {
    fn new() -> Self {
        Self {
            channels: RwLock::new(HashMap::new()),
        }
    }

    /// Subscribe to a task's event stream, creating the channel if needed.
    pub fn subscribe(&self, task_id: &str) -> broadcast::Receiver<StreamResponse> {
        let mut channels = self
            .channels
            .write()
            .unwrap_or_else(PoisonError::into_inner);
        channels
            .entry(task_id.to_owned())
            .or_insert_with(|| broadcast::channel(CHANNEL_CAPACITY).0)
            .subscribe()
    }

    /// Publish an event to a task's subscribers. Events published while no
    /// channel exists (nobody ever subscribed) are dropped silently.
    pub fn publish(&self, task_id: &str, event: &StreamResponse) {
        let channels = self.channels.read().unwrap_or_else(PoisonError::into_inner);
        if let Some(sender) = channels.get(task_id) {
            // A send error only means every receiver disconnected; the
            // channel is cleaned up by `close` when the task terminates.
            if sender.send(event.clone()).is_err() {
                debug!(task_id = %task_id, "A2A task event dropped: no live subscribers");
            }
        }
    }

    /// Close a task's channel. Every subscriber's stream ends, signalling
    /// the terminal state per A2A 1.0 stream semantics.
    pub fn close(&self, task_id: &str) {
        let mut channels = self
            .channels
            .write()
            .unwrap_or_else(PoisonError::into_inner);
        channels.remove(task_id);
    }
}
