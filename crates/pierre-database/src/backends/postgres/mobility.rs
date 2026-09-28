// ABOUTME: PostgreSQL-backed MobilityRepository, emitted from the shared implementation in repositories/mobility.rs
// ABOUTME: Catalogue searches match with ILIKE, Postgres's case-folding LIKE; SQLite passes plain LIKE
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::mobility::{
    ListStretchingFilter, ListYogaFilter, StretchingExercise, YogaPose,
};

use super::PostgresDatabase;
use crate::repositories::mobility::{
    impl_mobility_repository, json_array_pattern, limit_or, list_stretching_sql, list_yoga_sql,
    poses_for_recovery_sql, stretches_for_activity_sql, stretching_columns, stretching_from_row,
    yoga_columns, yoga_pose_from_row, MobilityRepository, GET_STRETCHING_EXERCISE_SQL,
    GET_YOGA_POSE_SQL,
};

impl_mobility_repository!(PostgresDatabase, "ILIKE");
