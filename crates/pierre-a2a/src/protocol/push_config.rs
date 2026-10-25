// ABOUTME: A2A task push-notification config CRUD — Create, Get, List and Delete
// ABOUTME: Handlers on A2AServer dispatched by the parent protocol module's JSON-RPC router

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::params::{CreatePushConfigParams, PushConfigListParams, PushConfigRefParams};
use super::principal::AuthPrincipal;
use super::{A2AResources, A2AServer};
use crate::protocol_types::{
    PushNotificationAuthenticationInfo, PushNotificationConfigInput, TaskPushNotificationConfig,
};
use crate::{push, A2ARequest, A2AResponse};
use pierre_core::models::a2a::A2APushNotificationConfig;
use serde_json::{from_value, json, to_value, Map, Value};
use tracing::error;
use uuid::Uuid;

impl A2AServer {
    /// Validate and persist a push notification configuration on a task.
    pub(super) async fn store_push_config(
        resources: &A2AResources,
        task_id: &str,
        input: PushNotificationConfigInput,
        request_id: Option<&Value>,
    ) -> Result<TaskPushNotificationConfig, Box<A2AResponse>> {
        if let Err(reason) = push::validate_webhook_url(&input.url).await {
            return Err(Box::new(Self::invalid_params(
                format!("Invalid push notification URL: {reason}"),
                request_id.cloned(),
            )));
        }

        let now = chrono::Utc::now();
        let record = A2APushNotificationConfig {
            config_id: input.id.unwrap_or_else(|| Uuid::new_v4().to_string()),
            task_id: task_id.to_owned(),
            url: input.url,
            token: input.token,
            auth_scheme: input.authentication.as_ref().map(|a| a.scheme.clone()), // Safe: String ownership for record
            auth_credentials: input
                .authentication
                .as_ref()
                .and_then(|a| a.credentials.clone()), // Safe: String ownership for record
            created_at: now,
            updated_at: now,
        };

        resources
            .ctx
            .repos()
            .a2a
            .create_push_config(&record)
            .await
            .map_err(|e| {
                error!("Failed to persist A2A push config: {e}");
                Box::new(Self::a2a_error(
                    -32000,
                    "Failed to persist push notification config",
                    request_id.cloned(),
                ))
            })?;

        Ok(Self::wire_push_config(&record))
    }

    /// Convert a stored push config into its wire shape.
    fn wire_push_config(record: &A2APushNotificationConfig) -> TaskPushNotificationConfig {
        TaskPushNotificationConfig {
            id: record.config_id.clone(), // Safe: String ownership for wire object
            task_id: record.task_id.clone(), // Safe: String ownership for wire object
            url: record.url.clone(),      // Safe: String ownership for wire object
            token: record.token.clone(),  // Safe: String ownership for wire object
            authentication: record.auth_scheme.as_ref().map(|scheme| {
                PushNotificationAuthenticationInfo {
                    scheme: scheme.clone(), // Safe: String ownership for wire object
                    credentials: record.auth_credentials.clone(), // Safe: String ownership for wire object
                }
            }),
        }
    }

    pub(super) async fn handle_create_push_config(
        &self,
        request: A2ARequest,
        principal: AuthPrincipal,
    ) -> A2AResponse {
        let Some(resources) = &self.resources else {
            return Self::server_not_configured_error(request.id);
        };

        let params: CreatePushConfigParams = match request
            .params
            .clone() // Safe: JSON value ownership for deserialization
            .map(from_value)
            .transpose()
        {
            Ok(Some(params)) => params,
            Ok(None) => return Self::invalid_params("Missing params: taskId, config", request.id),
            Err(e) => return Self::invalid_params(format!("Invalid params: {e}"), request.id),
        };

        if let Err(err) =
            Self::load_owned_task(resources, &params.task_id, &principal, request.id.as_ref()).await
        {
            return *err;
        }

        match Self::store_push_config(
            resources,
            &params.task_id,
            params.config,
            request.id.as_ref(),
        )
        .await
        {
            Ok(config) => match to_value(&config) {
                Ok(value) => Self::ok(value, request.id),
                Err(e) => {
                    error!("Failed to serialize A2A push config: {e}");
                    Self::a2a_error(-32603, "Failed to serialize push config", request.id)
                }
            },
            Err(err) => *err,
        }
    }

    pub(super) async fn handle_get_push_config(
        &self,
        request: A2ARequest,
        principal: AuthPrincipal,
    ) -> A2AResponse {
        let Some(resources) = &self.resources else {
            return Self::server_not_configured_error(request.id);
        };

        let params: PushConfigRefParams = match request
            .params
            .clone() // Safe: JSON value ownership for deserialization
            .map(from_value)
            .transpose()
        {
            Ok(Some(params)) => params,
            Ok(None) => {
                return Self::invalid_params("Missing params: taskId, configId", request.id)
            }
            Err(e) => return Self::invalid_params(format!("Invalid params: {e}"), request.id),
        };

        if let Err(err) =
            Self::load_owned_task(resources, &params.task_id, &principal, request.id.as_ref()).await
        {
            return *err;
        }

        match resources
            .ctx
            .repos()
            .a2a
            .get_push_config(&params.task_id, &params.config_id)
            .await
        {
            Ok(Some(record)) => match to_value(Self::wire_push_config(&record)) {
                Ok(value) => Self::ok(value, request.id),
                Err(e) => {
                    error!("Failed to serialize A2A push config: {e}");
                    Self::a2a_error(-32603, "Failed to serialize push config", request.id)
                }
            },
            Ok(None) => Self::invalid_params("Unknown push notification config id", request.id),
            Err(e) => {
                error!("Failed to load A2A push config: {e}");
                Self::a2a_error(-32000, "Database error", request.id)
            }
        }
    }

    pub(super) async fn handle_list_push_configs(
        &self,
        request: A2ARequest,
        principal: AuthPrincipal,
    ) -> A2AResponse {
        let Some(resources) = &self.resources else {
            return Self::server_not_configured_error(request.id);
        };

        let params: PushConfigListParams = match request
            .params
            .clone() // Safe: JSON value ownership for deserialization
            .map(from_value)
            .transpose()
        {
            Ok(Some(params)) => params,
            Ok(None) => return Self::invalid_params("Missing params: taskId", request.id),
            Err(e) => return Self::invalid_params(format!("Invalid params: {e}"), request.id),
        };

        if let Err(err) =
            Self::load_owned_task(resources, &params.task_id, &principal, request.id.as_ref()).await
        {
            return *err;
        }

        match resources
            .ctx
            .repos()
            .a2a
            .list_push_configs(&params.task_id)
            .await
        {
            Ok(records) => {
                let configs: Vec<TaskPushNotificationConfig> =
                    records.iter().map(Self::wire_push_config).collect();
                Self::ok(json!({ "configs": configs }), request.id)
            }
            Err(e) => {
                error!("Failed to list A2A push configs: {e}");
                Self::a2a_error(-32000, "Database error", request.id)
            }
        }
    }

    pub(super) async fn handle_delete_push_config(
        &self,
        request: A2ARequest,
        principal: AuthPrincipal,
    ) -> A2AResponse {
        let Some(resources) = &self.resources else {
            return Self::server_not_configured_error(request.id);
        };

        let params: PushConfigRefParams = match request
            .params
            .clone() // Safe: JSON value ownership for deserialization
            .map(from_value)
            .transpose()
        {
            Ok(Some(params)) => params,
            Ok(None) => {
                return Self::invalid_params("Missing params: taskId, configId", request.id)
            }
            Err(e) => return Self::invalid_params(format!("Invalid params: {e}"), request.id),
        };

        if let Err(err) =
            Self::load_owned_task(resources, &params.task_id, &principal, request.id.as_ref()).await
        {
            return *err;
        }

        match resources
            .ctx
            .repos()
            .a2a
            .delete_push_config(&params.task_id, &params.config_id)
            .await
        {
            Ok(true) => Self::ok(Value::Object(Map::default()), request.id),
            Ok(false) => Self::invalid_params("Unknown push notification config id", request.id),
            Err(e) => {
                error!("Failed to delete A2A push config: {e}");
                Self::a2a_error(-32000, "Database error", request.id)
            }
        }
    }
}
