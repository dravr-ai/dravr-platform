// ABOUTME: PostgreSQL-backed ProviderConnectionRepository, emitted from the shared implementation in repositories/provider_connections.rs
// ABOUTME: user_id is TEXT and every timestamp TIMESTAMPTZ here, so the shared statements need no engine argument
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::Utc;
use pierre_core::errors::AppResult;
use pierre_core::models::{ConnectionType, ProviderConnection, TenantId};
use uuid::Uuid;

use crate::backends::postgres::PostgresDatabase;
use crate::repositories::provider_connections::{
    connection_from_row, impl_provider_connection_repository, CLAIM_REAUTH_NOTIFICATION_SQL,
    GET_FOR_USER_IN_TENANT_SQL, GET_FOR_USER_SQL, IS_CONNECTED_SQL, MARK_ACTIVE_SQL,
    MARK_NEEDS_REAUTH_SQL, REGISTER_CONNECTION_SQL, REMOVE_CONNECTION_SQL,
    RESOLVE_MOST_RECENT_IN_TENANT_SQL, RESOLVE_MOST_RECENT_SQL, TOUCH_LAST_USED_SQL,
};
use crate::repositories::ProviderConnectionRepository;

impl_provider_connection_repository!(PostgresDatabase);
