// ABOUTME: PostgreSQL link-state lifecycle, emitted from the shared body in repositories/messaging_link_states.rs
// ABOUTME: tenant_id and user_id are uuid columns here, so a user id binds through ::uuid and reads back through ::text
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Link states for the Postgres backend.
//!
//! The statements, the guards and the row decode live in
//! [`crate::repositories::messaging_link_states`]; this file expands them
//! over the Postgres pool. The `MessagingRepository` impl in `messaging.rs`
//! delegates to these free functions.

use chrono::Utc;
use pierre_core::errors::messaging::MessagingError;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use serde_json::Value;
use sqlx::{Pool, Postgres};

use crate::repositories::messaging_link_states::{
    complete_link_state_sql, create_link_state_sql, impl_link_state_functions,
    link_state_by_code_sql, link_state_columns, link_state_for_tenant_sql, link_state_from_row,
    live_link_state_sql, parse_expires_at, LinkStateRow, CONSUME_LINK_STATE_SQL,
};
use crate::repositories::CreateLinkStateParams;

impl_link_state_functions!(Postgres, "::uuid", "::text");
