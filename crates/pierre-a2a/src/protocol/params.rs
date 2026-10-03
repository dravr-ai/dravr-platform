// ABOUTME: Request params of the A2A task methods — subscribe, cancel and the task push-notification configs
// ABOUTME: Deserialized from each JSON-RPC request's `params` by the protocol handlers of the parent module

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::protocol_types::PushNotificationConfigInput;

/// `SubscribeToTask` params (`id` + optional `historyLength` for the snapshot).
#[derive(serde::Deserialize)]
pub(super) struct SubscribeParams {
    /// Task identifier
    pub(super) id: String,
    /// History truncation for the initial snapshot
    #[serde(rename = "historyLength", default)]
    pub(super) history_length: Option<u32>,
}

/// `CancelTask` params.
#[derive(serde::Deserialize)]
pub(super) struct CancelParams {
    /// Task identifier
    pub(super) id: String,
}

/// `CreateTaskPushNotificationConfig` params.
#[derive(serde::Deserialize)]
pub(super) struct CreatePushConfigParams {
    /// Task the config attaches to
    #[serde(rename = "taskId")]
    pub(super) task_id: String,
    /// The configuration to register
    pub(super) config: PushNotificationConfigInput,
}

/// `GetTaskPushNotificationConfig` / `DeleteTaskPushNotificationConfig` params.
#[derive(serde::Deserialize)]
pub(super) struct PushConfigRefParams {
    /// Task the config attaches to
    #[serde(rename = "taskId")]
    pub(super) task_id: String,
    /// The configuration identifier
    #[serde(rename = "configId")]
    pub(super) config_id: String,
}

/// `ListTaskPushNotificationConfigs` params.
#[derive(serde::Deserialize)]
pub(super) struct PushConfigListParams {
    /// Task the configs attach to
    #[serde(rename = "taskId")]
    pub(super) task_id: String,
}
