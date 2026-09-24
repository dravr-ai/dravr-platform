// ABOUTME: SQLite-backed DelegatedConnectionRepository, emitted from the shared implementation in repositories/delegated_connections.rs
// ABOUTME: uuid columns are hyphenated TEXT here, so the shared statements bind and read ids through the text codec
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{DelegatedConnection, DelegationEndReason, DelegationStatus, TenantId};
use sqlx::sqlite::SqliteRow;
use uuid::Uuid;

use crate::database::Database;
use crate::repositories::delegated_connections::{
    already_linked, column, impl_delegated_connection_repository, require_fresh_proposal,
    unknown_value, CONFIRM_SQL, END_CONFIRMED_FOR_MEMBER_SQL, END_FOR_COACH_SQL,
    END_FOR_GROUP_MEMBER_SQL, END_FOR_GROUP_SQL, END_ONE_SQL, FIND_ACTIVE_FOR_MEMBER_SQL,
    GET_FOR_PARTICIPANT_SQL, LIST_BACKED_FOR_MEMBER_SQL, LIST_LIVE_FOR_COACH_IN_GROUP_SQL,
    LIST_LIVE_FOR_COACH_SQL, LIST_LIVE_FOR_MEMBER_IN_GROUP_SQL, LIST_LIVE_FOR_MEMBER_SQL,
    PROPOSE_SQL,
};
use crate::repositories::uuid_columns::TextUuid;
use crate::repositories::DelegatedConnectionRepository;

impl_delegated_connection_repository!(Database, SqliteRow, TextUuid);
