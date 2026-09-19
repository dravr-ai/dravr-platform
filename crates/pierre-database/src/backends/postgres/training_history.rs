// ABOUTME: PostgreSQL-backed TrainingHistoryRepository, emitted from the shared implementation in repositories/training_history.rs
// ABOUTME: user_id is a native uuid column here, so the shared statements bind the uuid unwrapped

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::NaiveDate;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{DailyTrainingState, TenantId};
use uuid::Uuid;

use crate::backends::postgres::PostgresDatabase;
use crate::repositories::training_history::{
    impl_training_history_repository, training_state_from_row, DELETE_TRAINING_HISTORY_RANGE_SQL,
    GET_TRAINING_HISTORY_SQL, LATEST_TRAINING_HISTORY_SQL, UPSERT_TRAINING_HISTORY_SQL,
};
use crate::repositories::uuid_columns::NativeUuid;
use crate::repositories::TrainingHistoryRepository;

impl_training_history_repository!(PostgresDatabase, NativeUuid::bind);
