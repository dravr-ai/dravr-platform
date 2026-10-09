// ABOUTME: The AgentsRepository shell on PostgresDatabase — the shared body over the shared statements
// ABOUTME: Supplies the PgRow row type and the native uuid codec; no SQL of its own
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::agents::{
    Agent, AgentAssignment, AgentCategory, AgentFieldOverlay, AgentHandle, AgentListItem,
    AgentPrerequisites, AgentVersion, AgentVisibility, CreateAgentRequest,
    CreateSystemAgentRequest, ListAgentsFilter, UpdateAgentRequest,
};
use pierre_core::models::TenantId;
use pierre_core::models::{split_visuals, AgentRuntimeContext};
use pierre_core::tokens::estimate_prompt_tokens;
use sqlx::postgres::PgRow;
use sqlx::Row;
use uuid::Uuid;

use super::PostgresDatabase;
use crate::database::agents::{compute_content_hash, compute_request_hash};
use crate::repositories::agent_rows::postgres::{
    row_to_agent, row_to_agent_list_item, row_to_agent_version,
};
use crate::repositories::agent_rows::{column, instant, token_count_bind};
use crate::repositories::agents::{
    translation_overlays_sql, AgentsRepository, AGENT_ASSIGNED_IN_TENANT_SQL, AGENT_BY_ID_SQL,
    AGENT_DATA_REQUIREMENTS_SQL, AGENT_IN_TENANT_EXISTS_SQL, AGENT_IS_SYSTEM_SQL,
    AGENT_OF_TENANT_SQL, AGENT_REACHABLE_SQL, AGENT_STARTUP_QUERY_SQL, ASSIGN_AGENT_SQL,
    COUNT_AGENTS_SQL, DELETE_AGENT_SQL, DELETE_SYSTEM_AGENT_SQL, DESELECT_AGENT_SQL,
    ENSURE_ASSIGNMENT_SQL, FAVORITE_STATUS_SQL, FIND_INSTALLED_BY_HANDLE_SQL, GET_ACTIVE_AGENT_SQL,
    GET_AGENT_IN_TENANT_SQL, GET_AGENT_SQL, GET_SYSTEM_AGENT_SQL, GET_VERSION_SQL, HIDE_AGENT_SQL,
    INSERT_AGENT_SQL, INSERT_SYSTEM_AGENT_SQL, INSERT_VERSION_SQL, LIST_AGENTS_SQL,
    LIST_ASSIGNMENTS_FOR_TENANT_SQL, LIST_ASSIGNMENTS_SQL, LIST_HIDDEN_AGENTS_SQL,
    LIST_SYSTEM_AGENTS_SQL, LIST_VERSIONS_SQL, MAX_VERSION_SQL, RECORD_USAGE_SQL, REVERT_AGENT_SQL,
    RUNTIME_CONTEXT_SQL, SEARCH_AGENTS_SQL, SELECT_AGENT_SQL, SET_FAVORITE_SQL, SHOW_AGENT_SQL,
    UNASSIGN_AGENT_SQL, UPDATE_AGENT_SQL, UPDATE_SYSTEM_AGENT_SQL, USER_PREFERENCES_SQL,
};
use crate::repositories::agents_backend::impl_agents_repository;
use crate::repositories::uuid_columns::NativeUuid;

impl_agents_repository!(PostgresDatabase, PgRow, NativeUuid);
