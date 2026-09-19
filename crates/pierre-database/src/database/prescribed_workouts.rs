// ABOUTME: SQLite-backed PrescribedWorkoutRepository, emitted from the shared implementation in repositories/prescribed_workouts.rs
// ABOUTME: Ids bind as hyphenated text, dates and timestamps as ISO text and payload_json as TEXT, through the same binds Postgres uses
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{NaiveDate, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{PrescribedWorkout, TenantId, UserId};
use serde_json::Value;
use uuid::Uuid;

use crate::database::Database;
use crate::repositories::prescribed_workouts::{
    impl_prescribed_workout_repository, prescribed_from_row, PrescribedWorkoutRepository,
    GET_PRESCRIBED_WORKOUT_SQL, LIST_LIVE_CALENDAR_EVENTS_SQL, LIST_PRESCRIBED_WORKOUTS_SQL,
    MAX_LIST_LIMIT, SET_PRESCRIBED_WORKOUT_STATUS_SQL, UPSERT_PRESCRIBED_WORKOUT_SQL,
};
use crate::uuid_column::UuidColumn;

impl_prescribed_workout_repository!(Database);
