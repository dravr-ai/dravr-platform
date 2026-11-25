// ABOUTME: PostgreSQL-backed UserOnboardingRepository — durable per-user onboarding step completion state
// ABOUTME: Shell over the shared body in repositories/user_onboarding.rs; sqlx resolves the driver from the pool
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::{AppError, AppResult};

use crate::backends::postgres::PostgresDatabase;
use crate::repositories::user_onboarding::{
    impl_user_onboarding_repository, stalled_from_row, step_record_from_row,
    MEMORY_RESET_STATEMENTS, ONBOARDING_RESET_STATEMENTS, SELECT_ONBOARDING_STEPS_SQL,
    SELECT_STALLED_AT_CONNECT_SQL, UPSERT_ONBOARDING_STEP_SQL,
};
use crate::repositories::{
    OnboardingReset, OnboardingResetScope, OnboardingStepRecord, StalledAtConnect,
    UserOnboardingRepository,
};

impl_user_onboarding_repository!(PostgresDatabase);
