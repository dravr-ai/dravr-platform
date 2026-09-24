// ABOUTME: SQLite-backed ProviderConnectionRepository, emitted from the shared implementation in repositories/provider_connections.rs
// ABOUTME: user_id is TEXT and every timestamp RFC 3339 text here, so the shared statements need no engine argument
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use pierre_core::errors::AppResult;
use pierre_core::models::{
    ConnectionType, ProviderAccountRole, ProviderConnection, ReauthMark, TenantId, UserOAuthToken,
};
use uuid::Uuid;

use crate::database::Database;
use crate::repositories::provider_connections::{
    connection_from_row, impl_provider_connection_repository, reauth_mark,
    CLAIM_REAUTH_NOTIFICATION_SQL, CONNECTION_STATUS_SQL, GET_FOR_USER_IN_TENANT_SQL,
    GET_FOR_USER_SQL, IS_CONNECTED_SQL, MARK_ACTIVE_SQL, MARK_NEEDS_REAUTH_IF_TOKEN_CURRENT_SQL,
    MARK_NEEDS_REAUTH_SQL, REGISTER_CONNECTION_SQL, REMOVE_CONNECTION_SQL,
    REMOVE_DELEGATED_CONNECTION_SQL, RESOLVE_MOST_RECENT_IN_TENANT_SQL, RESOLVE_MOST_RECENT_SQL,
    SET_ACCOUNT_ROLE_SQL, TOUCH_LAST_USED_SQL,
};
use crate::repositories::ProviderConnectionRepository;

impl_provider_connection_repository!(Database);
