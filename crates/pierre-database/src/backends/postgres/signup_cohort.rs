// ABOUTME: PostgreSQL implementation of the new-user cohort read, emitted from the shared statements
// ABOUTME: Ids are native uuid here; the TEXT tenant and user columns compare through CAST in the SQL

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use sqlx::Row;

use crate::backends::postgres::PostgresDatabase;
use crate::repositories::signup_cohort::{
    impl_signup_cohort_repository, NewUserMilestones, NewUserTurn, SignupCohortRepository,
    NEW_USER_MILESTONES_IN_TENANT_SQL, NEW_USER_MILESTONES_SQL, NEW_USER_TURNS_IN_TENANT_SQL,
    NEW_USER_TURNS_SQL,
};
use crate::repositories::uuid_columns::NativeUuid;

impl_signup_cohort_repository!(PostgresDatabase, NativeUuid);
