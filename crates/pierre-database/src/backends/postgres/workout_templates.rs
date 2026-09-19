// ABOUTME: PostgreSQL-backed WorkoutTemplateRepository, emitted from the shared implementation in repositories/workout_templates.rs
// ABOUTME: Ids bind as native uuid, updated_at as TIMESTAMPTZ and the *_json columns as jsonb, through the same binds SQLite uses
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{TenantId, UserId, WorkoutTemplate};
use uuid::Uuid;

use super::PostgresDatabase;
use crate::repositories::workout_templates::{
    impl_workout_template_repository, json_column, json_text, template_from_row,
    WorkoutTemplateRepository, GET_USER_WORKOUT_TEMPLATE_SQL, LIST_USER_WORKOUT_TEMPLATES_SQL,
    UPSERT_WORKOUT_TEMPLATE_SQL,
};
use crate::uuid_column::UuidColumn;

impl_workout_template_repository!(PostgresDatabase);
