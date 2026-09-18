// ABOUTME: SQLite-backed UserOnboardingRepository — durable per-user onboarding step completion state
// ABOUTME: Shell over the shared body in repositories/user_onboarding.rs; sqlx resolves the driver from the pool
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::{AppError, AppResult};

use crate::database::Database;
use crate::repositories::user_onboarding::{
    impl_user_onboarding_repository, step_record_from_row, SELECT_ONBOARDING_STEPS_SQL,
    UPSERT_ONBOARDING_STEP_SQL,
};
use crate::repositories::{OnboardingStepRecord, UserOnboardingRepository};

impl_user_onboarding_repository!(Database);
