// ABOUTME: SQLite implementation of the ClaimVerdictRepository trait
// ABOUTME: Shell over the shared body; SQLite cuts the calendar day out of the RFC3339 text
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Claim verdict persistence for the `SQLite` backend.
//!
//! The statements, the row decode and the trait body live in
//! [`crate::repositories::claim_verdicts`]. `created_at` is `TEXT` holding
//! RFC3339 here, so a calendar day is the first ten characters of it — cheaper
//! than `strftime` and free of its parsing quirks.

use std::collections::BTreeMap;

use chrono::{DateTime, Duration, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use pierre_memory::claims::{
    ClaimCategory, ClaimStatus, ClaimVerdict, EvidenceStrength, VerdictLayer,
};
use sqlx::sqlite::SqliteRow;
use sqlx::Row;
use uuid::Uuid;

use super::Database;
use crate::repositories::claim_verdicts::{
    apply_status_count, impl_claim_verdict_repository, verdict_daily_counts_sql,
    INSERT_CLAIM_VERDICT_SQL, LIST_RECENT_VERDICTS_SQL, LIST_VERDICTS_FOR_CONVERSATION_SQL,
};
use crate::repositories::{
    ClaimVerdictRepository, InsertClaimVerdictParams, VerdictCalibrationStats, VerdictDailyBucket,
    VerdictStatusBreakdown,
};

/// This backend's resolved daily-counts statement.
const VERDICT_DAILY_COUNTS_SQL: &str = verdict_daily_counts_sql!("substr(created_at, 1, 10)");

impl_claim_verdict_repository!(Database, SqliteRow, VERDICT_DAILY_COUNTS_SQL);
