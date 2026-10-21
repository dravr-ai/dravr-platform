// ABOUTME: The AgentsRepository shell on Database (SQLite) — the shared body over the shared statements
// ABOUTME: Supplies the SqliteRow row type and the text uuid codec; no SQL of its own
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashMap;

use chrono::Utc;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::agents::{
    Agent, AgentAssignment, AgentCategory, AgentFieldOverlay, AgentHandle, AgentListItem,
    AgentPrerequisites, AgentUserState, AgentVersion, AgentVisibility, CreateAgentRequest,
    CreateSystemAgentRequest, ListAgentsFilter, UpdateAgentRequest,
};
use pierre_core::models::TenantId;
use pierre_core::models::{split_visuals, AgentRuntimeContext};
use pierre_core::tokens::estimate_prompt_tokens;
use sqlx::sqlite::SqliteRow;
use sqlx::Row;
use uuid::Uuid;

use crate::database::agents::{compute_content_hash, compute_request_hash};
use crate::database::agents::{row_to_agent, row_to_agent_list_item, row_to_agent_version};
use crate::database::Database;
use crate::repositories::agent_rows::{column, instant, token_count_bind};
use crate::repositories::agents::{
    translation_overlays_sql, AgentsRepository, AGENT_ASSIGNED_IN_TENANT_SQL, AGENT_BY_ID_SQL,
    AGENT_DATA_REQUIREMENTS_SQL, AGENT_IN_TENANT_EXISTS_SQL, AGENT_IS_SYSTEM_SQL,
    AGENT_OF_TENANT_SQL, AGENT_OWNED_SQL, AGENT_REACHABLE_SQL, AGENT_STARTUP_QUERY_SQL,
    ASSIGN_AGENT_SQL, COUNT_AGENTS_SQL, COUNT_FORK_AS_INSTALL_SQL, DELETE_AGENT_SQL,
    DELETE_SYSTEM_AGENT_SQL, DESELECT_AGENT_SQL, ENSURE_ASSIGNMENT_SQL, FAVORITE_STATUS_SQL,
    FIND_INSTALLED_BY_HANDLE_SQL, FORK_AGENT_SQL, FORK_TRANSLATIONS_SQL, GET_ACTIVE_AGENT_SQL,
    GET_AGENT_IN_TENANT_SQL, GET_AGENT_SQL, GET_SYSTEM_AGENT_SQL, GET_VERSION_SQL, HIDE_AGENT_SQL,
    INSERT_AGENT_SQL, INSERT_SYSTEM_AGENT_SQL, INSERT_VERSION_SQL, LIST_AGENTS_SQL,
    LIST_ASSIGNMENTS_FOR_TENANT_SQL, LIST_ASSIGNMENTS_SQL, LIST_CATALOGUE_SYSTEM_AGENTS_SQL,
    LIST_HIDDEN_AGENTS_SQL, LIST_SYSTEM_AGENTS_SQL, LIST_VERSIONS_SQL, MAX_VERSION_SQL,
    MOVE_MEMBERSHIP_SQL, OWNED_AGENT_FORKED_FROM_SQL, OWNED_COPY_OF_SQL, RECORD_USAGE_SQL,
    RESELECT_FORK_SQL, REVERT_AGENT_SQL, RUNTIME_CONTEXT_SQL, SEARCH_AGENTS_SQL, SELECT_AGENT_SQL,
    SET_FAVORITE_SQL, SHOW_AGENT_SQL, UNASSIGN_AGENT_SQL, UPDATE_AGENT_SQL,
    UPDATE_SYSTEM_AGENT_SQL, USER_PREFERENCES_SQL,
};
use crate::repositories::agents_backend::impl_agents_repository;
use crate::repositories::agents_lifecycle_backend::impl_agent_lifecycle;
use crate::repositories::store_listings::{
    AGENT_GROUPS_LEFT_SQL, DECREMENT_INSTALLS_SQL, HAND_ON_AGENT_CONVERSATIONS_SQL,
    INCREMENT_INSTALLS_SQL, REPOINT_AGENT_GROUPS_SQL,
};
use crate::repositories::uuid_columns::TextUuid;

impl_agents_repository!(Database, SqliteRow, TextUuid);
impl_agent_lifecycle!(Database, TextUuid);
