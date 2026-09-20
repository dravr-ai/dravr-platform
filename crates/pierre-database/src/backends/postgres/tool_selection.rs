// ABOUTME: PostgreSQL-backed ToolSelectionRepository, emitted from the shared implementation in repositories/tool_selection.rs
// ABOUTME: the override row's uuid columns are native here, so the shared statements bind them as themselves
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::Utc;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{
    TenantId, TenantPlan, TenantToolOverride, ToolCatalogEntry, ToolCategory,
};
use std::collections::HashMap;
use uuid::Uuid;

use crate::backends::postgres::PostgresDatabase;
use crate::repositories::tool_selection::{
    impl_tool_selection_repository, tenant_tool_override_from_row, tool_catalog_entry_from_row,
    tools_by_min_plan_sql, ToolSelectionRepository, DELETE_TENANT_TOOL_OVERRIDE_SQL,
    DELETE_TOOL_CATALOG_ENTRY_SQL, GET_TENANT_TOOL_OVERRIDES_SQL, GET_TENANT_TOOL_OVERRIDE_SQL,
    GET_TOOLS_BY_CATEGORY_SQL, GET_TOOL_CATALOG_ENTRY_SQL, GET_TOOL_CATALOG_SQL,
    UPSERT_TENANT_TOOL_OVERRIDE_SQL, UPSERT_TOOL_CATALOG_ENTRY_SQL,
};
use crate::repositories::uuid_columns::NativeUuid;
use crate::repositories::TenantRepository;

impl_tool_selection_repository!(PostgresDatabase, NativeUuid);
