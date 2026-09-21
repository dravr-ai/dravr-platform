// ABOUTME: PostgreSQL-backed UsageCounterRepository, emitted from the shared implementation in repositories/usage_counters.rs
// ABOUTME: Every key column is TEXT and updated_at is TIMESTAMPTZ here, so the shared statements bind as they are
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::Utc;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::UsageCounterRecord;

use crate::backends::postgres::PostgresDatabase;
use crate::repositories::usage_counters::{
    counter_from_row, impl_usage_counter_repository, DELETE_OLD_COUNTERS_SQL, GET_COUNTER_SQL,
    INCREMENT_COUNTER_SQL,
};
use crate::repositories::UsageCounterRepository;

impl_usage_counter_repository!(PostgresDatabase);
