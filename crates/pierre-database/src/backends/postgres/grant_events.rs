// ABOUTME: PostgreSQL-backed OAuthGrantEventRepository, emitted from the shared body in repositories/grant_events.rs
// ABOUTME: A consent grant's receipts: the record_log worker's outbox and the athlete's grant history
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use dravr_tronc::dates::Timestamp;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::OAuthGrantEvent;

use crate::backends::postgres::PostgresDatabase;
use crate::repositories::grant_events::{
    grant_event_from_row, impl_grant_event_repository, OAuthGrantEventRepository,
    LIST_UNLOGGED_GRANT_EVENTS_SQL, LIST_UNSIGNED_GRANT_EVENTS_SQL,
    LIST_USER_GRANT_EVENTS_OF_GRANT_SQL, LIST_USER_GRANT_EVENTS_SQL, MARK_GRANT_EVENT_LOGGED_SQL,
    MAX_GRANT_RECEIPT_ATTEMPTS, RECORD_GRANT_EVENT_FAILURE_SQL, SET_GRANT_EVENT_JWS_SQL,
};

impl_grant_event_repository!(PostgresDatabase);
