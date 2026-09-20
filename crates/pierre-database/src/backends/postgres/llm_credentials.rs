// ABOUTME: PostgreSQL-backed LlmCredentialRepository, emitted from the shared implementation in repositories/llm_credentials.rs
// ABOUTME: user_id and created_by are native uuid columns here, so the shared statements bind the uuids as themselves
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::admin::models::AdminConfigOverrideRow;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{LlmCredentialRecord, LlmCredentialSummary, TenantId};
use sqlx::Row;
use uuid::Uuid;

use crate::backends::postgres::PostgresDatabase;
use crate::repositories::llm_credentials::{
    admin_config_category, credential_column_error, credential_timestamp,
    impl_llm_credential_repository, llm_credential_record_from_row,
    llm_credential_summary_from_row, DELETE_TENANT_LLM_CREDENTIALS_SQL,
    DELETE_USER_LLM_CREDENTIALS_SQL, GET_GLOBAL_ADMIN_CONFIG_OVERRIDE_SQL,
    GET_TENANT_ADMIN_CONFIG_OVERRIDE_SQL, GET_TENANT_LLM_CREDENTIALS_SQL,
    GET_USER_LLM_CREDENTIALS_SQL, LIST_ADMIN_CONFIG_OVERRIDES_BY_CATEGORY_SQL,
    LIST_LLM_CREDENTIALS_SQL, STORE_LLM_CREDENTIALS_SQL,
};
use crate::repositories::uuid_columns::NativeUuid;
use crate::repositories::LlmCredentialRepository;

impl_llm_credential_repository!(PostgresDatabase, NativeUuid);
