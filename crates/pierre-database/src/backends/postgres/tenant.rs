// ABOUTME: PostgreSQL-backed TenantRepository, emitted from the shared implementation in repositories/tenants.rs
// ABOUTME: uuid columns are native and list columns are text[] here, so the shared statements take both codecs
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::Utc;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{OAuthApp, Tenant, TenantId, TenantOAuthCredentials};
use sqlx::Row;
use tracing::info;
use uuid::Uuid;

use crate::backends::postgres::PostgresDatabase;
use crate::backends::shared::encryption::HasEncryption;
use crate::repositories::list_columns::NativeList;
use crate::repositories::tenants::{
    impl_tenant_repository, oauth_app_from_row, tenant_from_row, tenant_oauth_aad,
    tenant_oauth_credentials_from_row, TenantRepository, ADD_TENANT_OWNER_SQL,
    CREATE_OAUTH_APP_SQL, CREATE_TENANT_SQL, GET_ALL_TENANTS_SQL, GET_OAUTH_APP_BY_CLIENT_ID_SQL,
    GET_SELECTED_AGENT_SQL, GET_TENANT_BY_ID_SQL, GET_TENANT_BY_SLUG_SQL,
    GET_TENANT_OAUTH_CREDENTIALS_SQL, GET_TENANT_OAUTH_PROVIDERS_SQL, GET_USER_TENANT_ROLE_SQL,
    LIST_MEMBERSHIP_TENANT_IDS_SQL, LIST_TENANTS_FOR_USER_SQL, SET_SELECTED_AGENT_SQL,
    SET_TENANT_PLAN_SQL, STORE_TENANT_OAUTH_CREDENTIALS_SQL,
};
use crate::repositories::uuid_columns::NativeUuid;

impl_tenant_repository!(PostgresDatabase, NativeUuid, NativeList);
