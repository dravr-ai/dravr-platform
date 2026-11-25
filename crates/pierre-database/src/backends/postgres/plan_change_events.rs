// ABOUTME: PostgreSQL-backed PlanChangeEventRepository, emitted from the shared body in repositories/plan_change_events.rs
// ABOUTME: Postgres stores user ids as native uuid columns, so the shell passes the native uuid codec
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::AppResult;
use sqlx::postgres::PgRow;
use sqlx::Row;
use uuid::Uuid;

use super::PostgresDatabase;
use crate::repositories::plan_change_events::{
    decode_dates, encode_dates, impl_plan_change_event_repository, plan_change_error,
    NewPlanChangeEvent, PlanChangeEvent, PlanChangeEventRepository, PlanChangeKind,
    PlanChangeQuery, PlanChangeSource, LIST_PLAN_CHANGES_BEFORE_SQL, LIST_PLAN_CHANGES_SQL,
    RECORD_PLAN_CHANGE_SQL,
};
use crate::repositories::uuid_columns::NativeUuid;

impl_plan_change_event_repository!(PostgresDatabase, PgRow, NativeUuid);
