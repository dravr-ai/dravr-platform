// ABOUTME: SQLite-backed PlanChangeEventRepository, emitted from the shared body in repositories/plan_change_events.rs
// ABOUTME: SQLite stores user ids as hyphenated TEXT, so the shell passes the text uuid codec
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::AppResult;
use sqlx::sqlite::SqliteRow;
use sqlx::Row;
use uuid::Uuid;

use crate::database::Database;
use crate::repositories::plan_change_events::{
    decode_dates, encode_dates, impl_plan_change_event_repository, plan_change_error,
    NewPlanChangeEvent, PlanChangeEvent, PlanChangeEventRepository, PlanChangeKind,
    PlanChangeQuery, PlanChangeSource, LIST_PLAN_CHANGES_BEFORE_SQL, LIST_PLAN_CHANGES_SQL,
    RECORD_PLAN_CHANGE_SQL,
};
use crate::repositories::uuid_columns::TextUuid;

impl_plan_change_event_repository!(Database, SqliteRow, TextUuid);
