// ABOUTME: SQLite-backed ResumableTurnRepository, emitted from the shared implementation in repositories/resumable_turns.rs
// ABOUTME: Single-writer SQLite has no row locks, so the sweep's claim subquery carries no lock clause
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;

use super::Database;
use crate::repositories::resumable_turns::{
    claim_turns_sql, i64_column, impl_resumable_turn_repository, turn_columns, turn_from_row,
    ResumableTurnClaim, ResumableTurnRepository, ResumableTurnRow, TurnClaim, TurnLease,
    BUMP_TURN_ENQUEUE_SQL, CLAIM_TURN_SQL, FINISH_TURN_SQL, LIST_STALE_TURNS_SQL,
    REAP_EXHAUSTED_TURNS_SQL, RECORD_TURN_SQL, RELEASE_TURN_SQL, RENEW_TURN_LEASE_SQL,
    SET_TURN_PLACEHOLDER_SQL, TURN_ATTEMPTS_SQL,
};

impl_resumable_turn_repository!(Database, "");
