// ABOUTME: A2A task query methods — GetTask, ListTasks (paginated, owner-scoped) and CancelTask
// ABOUTME: Handlers on A2AServer dispatched by the parent protocol module's JSON-RPC router

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::params::CancelParams;
use super::principal::AuthPrincipal;
use super::{A2AResources, A2AServer};
use crate::events::TASK_EVENTS;
use crate::protocol_types::{
    A2ASpecError, GetTaskRequest, ListTasksRequest, ListTasksResponse, Task, TaskState,
};
use crate::{A2ARequest, A2AResponse};
use pierre_core::models::a2a::A2ATask;
use serde_json::{from_value, to_value, Value};
use std::cmp::Reverse;
use tracing::error;

/// Default `ListTasks` page size (spec §9: default 50, range 1..=100).
const LIST_TASKS_DEFAULT_PAGE_SIZE: u32 = 50;
/// Maximum `ListTasks` page size.
const LIST_TASKS_MAX_PAGE_SIZE: u32 = 100;

/// Validated `ListTasks` query: the parsed request plus derived pagination
/// state (clamped page size, `pageToken` offset, parsed timestamp filter).
struct ListTasksQuery {
    params: ListTasksRequest,
    page_size: u32,
    offset: u32,
    updated_after: Option<chrono::DateTime<chrono::Utc>>,
}

impl A2AServer {
    pub(super) async fn handle_get_task(
        &self,
        request: A2ARequest,
        principal: AuthPrincipal,
    ) -> A2AResponse {
        let Some(resources) = &self.resources else {
            return Self::server_not_configured_error(request.id);
        };

        let params: GetTaskRequest = match request
            .params
            .clone() // Safe: JSON value ownership for deserialization
            .map(from_value)
            .transpose()
        {
            Ok(Some(params)) => params,
            Ok(None) => return Self::invalid_params("Missing params: id", request.id),
            Err(e) => return Self::invalid_params(format!("Invalid params: {e}"), request.id),
        };

        match Self::load_owned_task(resources, &params.id, &principal, request.id.as_ref()).await {
            Ok(record) => {
                let task = Self::wire_task(&record, params.history_length, true);
                Self::task_response(&task, request.id)
            }
            Err(err) => *err,
        }
    }

    /// Parse and validate `ListTasks` params into a [`ListTasksQuery`].
    fn parse_list_tasks_params(request: &A2ARequest) -> Result<ListTasksQuery, Box<A2AResponse>> {
        let params: ListTasksRequest = match request
            .params
            .clone() // Safe: JSON value ownership for deserialization
            .map(from_value)
            .transpose()
        {
            Ok(params) => params.unwrap_or_default(),
            Err(e) => {
                return Err(Box::new(Self::invalid_params(
                    format!("Invalid params: {e}"),
                    request.id.clone(),
                )))
            }
        };

        let page_size = params
            .page_size
            .unwrap_or(LIST_TASKS_DEFAULT_PAGE_SIZE)
            .clamp(1, LIST_TASKS_MAX_PAGE_SIZE);

        let offset = match params.page_token.as_deref() {
            None | Some("") => 0_u32,
            Some(token) => match token.strip_prefix('o').and_then(|n| n.parse::<u32>().ok()) {
                Some(offset) => offset,
                None => {
                    return Err(Box::new(Self::invalid_params(
                        "Invalid pageToken",
                        request.id.clone(),
                    )))
                }
            },
        };

        let updated_after = match params.status_timestamp_after.as_deref() {
            None => None,
            Some(ts) => match chrono::DateTime::parse_from_rfc3339(ts) {
                Ok(instant) => Some(instant.with_timezone(&chrono::Utc)),
                Err(_) => {
                    return Err(Box::new(Self::invalid_params(
                        "Invalid statusTimestampAfter: expected ISO-8601",
                        request.id.clone(),
                    )))
                }
            },
        };

        Ok(ListTasksQuery {
            params,
            page_size,
            offset,
            updated_after,
        })
    }

    /// Fetch the caller's task records — a client-credentials principal is
    /// scoped to its own client; a user principal spans every owned client
    /// (usually one) — merged and sorted by status timestamp descending.
    async fn fetch_owned_task_records(
        resources: &A2AResources,
        principal: &AuthPrincipal,
        params: &ListTasksRequest,
        updated_after: Option<chrono::DateTime<chrono::Utc>>,
        fetch_limit: u32,
        request_id: Option<&Value>,
    ) -> Result<Vec<A2ATask>, Box<A2AResponse>> {
        let owned_client_ids = if let Some(acting_client) = &principal.client_id {
            vec![acting_client.clone()] // Safe: String ownership for scope list
        } else {
            Self::get_owned_client_ids(&principal.user_id, resources)
                .await
                .map_err(|e| {
                    error!("Failed to resolve client ownership: {e}");
                    Box::new(Self::a2a_error(
                        -32000,
                        "Failed to resolve client ownership",
                        request_id.cloned(),
                    ))
                })?
        };

        let mut records: Vec<A2ATask> = Vec::new();
        for client_id in &owned_client_ids {
            let tasks = resources
                .ctx
                .repos()
                .a2a
                .list_tasks(
                    Some(client_id),
                    params.status.as_ref(),
                    params.context_id.as_deref(),
                    updated_after,
                    Some(fetch_limit),
                    None,
                )
                .await
                .map_err(|e| {
                    error!("A2A task list database error: {e}");
                    Box::new(Self::a2a_error(
                        -32000,
                        "Database error",
                        request_id.cloned(),
                    ))
                })?;
            records.extend(tasks);
        }
        records.sort_by_key(|record| Reverse(record.updated_at));
        Ok(records)
    }

    pub(super) async fn handle_list_tasks(
        &self,
        request: A2ARequest,
        principal: AuthPrincipal,
    ) -> A2AResponse {
        let Some(resources) = &self.resources else {
            return Self::server_not_configured_error(request.id);
        };

        let query = match Self::parse_list_tasks_params(&request) {
            Ok(query) => query,
            Err(err) => return *err,
        };
        let ListTasksQuery {
            params,
            page_size,
            offset,
            updated_after,
        } = query;

        // Fetch one page-plus-one rows so the continuation token is exact.
        let fetch_limit = offset.saturating_add(page_size).saturating_add(1);
        let records = match Self::fetch_owned_task_records(
            resources,
            &principal,
            &params,
            updated_after,
            fetch_limit,
            request.id.as_ref(),
        )
        .await
        {
            Ok(records) => records,
            Err(err) => return *err,
        };

        let skip_count = usize::try_from(offset).unwrap_or(usize::MAX);
        let take_count = usize::try_from(page_size).unwrap_or(usize::MAX);
        let has_more = records.len() > skip_count.saturating_add(take_count);

        let tasks: Vec<Task> = records
            .iter()
            .skip(skip_count)
            .take(take_count)
            .map(|record| Self::wire_task(record, params.history_length, params.include_artifacts))
            .collect();

        let next_page_token = if has_more {
            format!("o{}", offset.saturating_add(page_size))
        } else {
            String::new()
        };

        let response = ListTasksResponse {
            tasks,
            next_page_token,
        };
        match to_value(&response) {
            Ok(value) => Self::ok(value, request.id),
            Err(e) => {
                error!("Failed to serialize A2A task list: {e}");
                Self::a2a_error(-32603, "Failed to serialize task list", request.id)
            }
        }
    }

    pub(super) async fn handle_cancel_task(
        &self,
        request: A2ARequest,
        principal: AuthPrincipal,
    ) -> A2AResponse {
        let Some(resources) = &self.resources else {
            return Self::server_not_configured_error(request.id);
        };

        let params: CancelParams = match request
            .params
            .clone() // Safe: JSON value ownership for deserialization
            .map(from_value)
            .transpose()
        {
            Ok(Some(params)) => params,
            Ok(None) => return Self::invalid_params("Missing params: id", request.id),
            Err(e) => return Self::invalid_params(format!("Invalid params: {e}"), request.id),
        };

        let record =
            match Self::load_owned_task(resources, &params.id, &principal, request.id.as_ref())
                .await
            {
                Ok(record) => record,
                Err(err) => return *err,
            };

        if record.status.is_terminal() {
            return Self::spec_error(A2ASpecError::TaskNotCancelable, request.id);
        }

        if let Err(e) = resources
            .ctx
            .repos()
            .a2a
            .update_task_status(&record.id, &TaskState::Canceled, None, None)
            .await
        {
            error!("Failed to cancel A2A task: {e}");
            return Self::a2a_error(-32000, "Failed to cancel task", request.id);
        }

        let context_id = record.context_id.clone().unwrap_or_default();
        Self::publish_status(&record.id, &context_id, TaskState::Canceled, None);
        TASK_EVENTS.close(&record.id);
        Self::notify_webhooks(&resources.ctx, &record.id).await;

        match Self::load_owned_task(resources, &record.id, &principal, request.id.as_ref()).await {
            Ok(updated) => {
                let task = Self::wire_task(&updated, None, true);
                Self::task_response(&task, request.id)
            }
            Err(err) => *err,
        }
    }
}
