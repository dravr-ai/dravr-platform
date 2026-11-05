// ABOUTME: PostgreSQL-backed GuardianPendingActionsRepository, emitted from the shared implementation
// ABOUTME: Guarded single-winner UPDATE for the single-use claim; expiry filtered at resolution time
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};

use crate::backends::postgres::PostgresDatabase;
use crate::repositories::guardian_actions::{
    action_from_row, impl_guardian_pending_actions_repository, ClaimOutcome,
    GuardianPendingActionsRepository, PendingGuardianAction, CLAIM_PENDING_ACTION_SQL,
    EXPIRE_PENDING_ACTION_SQL, INSERT_PENDING_ACTION_SQL, READ_CLAIMED_ACTION_SQL,
    SWEEP_PENDING_ACTIONS_SQL,
};

impl_guardian_pending_actions_repository!(PostgresDatabase);
