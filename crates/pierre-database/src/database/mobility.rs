// ABOUTME: SQLite-backed MobilityRepository, emitted from the shared implementation in repositories/mobility.rs
// ABOUTME: Catalogue searches match with LIKE, which SQLite folds for ASCII letters only; Postgres passes ILIKE
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::mobility::{
    ActivityMuscleMapping, ListStretchingFilter, ListYogaFilter, StretchingExercise, YogaPose,
};

use super::Database;
use crate::repositories::mobility::{
    contains_pattern, impl_mobility_repository, json_array_pattern, limit_or, list_stretching_sql,
    list_yoga_sql, muscle_mapping_from_row, poses_for_recovery_sql, search_stretching_sql,
    search_yoga_sql, stretches_for_activity_sql, stretching_columns, stretching_from_row,
    yoga_columns, yoga_pose_from_row, MobilityRepository, GET_MUSCLE_MAPPING_SQL,
    GET_STRETCHING_EXERCISE_SQL, GET_YOGA_POSE_SQL, LIST_MUSCLE_MAPPINGS_SQL,
};

impl_mobility_repository!(Database, "LIKE");
