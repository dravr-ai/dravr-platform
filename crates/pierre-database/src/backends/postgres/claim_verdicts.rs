// ABOUTME: PostgreSQL implementation of the ClaimVerdictRepository trait
// ABOUTME: Shell over the shared body; Postgres formats the calendar day out of a timestamptz
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Claim verdict persistence for the `PostgreSQL` backend.
//!
//! The statements, the row decode and the trait body live in
//! [`crate::repositories::claim_verdicts`]. `created_at` is `TIMESTAMPTZ`
//! here, so a calendar day comes from formatting it in UTC rather than from
//! slicing text.

use std::collections::BTreeMap;

use chrono::{DateTime, Duration, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use pierre_memory::claims::{
    ClaimCategory, ClaimStatus, ClaimVerdict, EvidenceStrength, VerdictLayer,
};
use sqlx::postgres::PgRow;
use sqlx::Row;
use uuid::Uuid;

use super::PostgresDatabase;
use crate::repositories::claim_verdicts::{
    apply_status_count, impl_claim_verdict_repository, verdict_daily_counts_sql,
    INSERT_CLAIM_VERDICT_SQL, LIST_RECENT_VERDICTS_SQL, LIST_VERDICTS_FOR_CONVERSATION_SQL,
};
use crate::repositories::{
    ClaimVerdictRepository, InsertClaimVerdictParams, VerdictCalibrationStats, VerdictDailyBucket,
    VerdictStatusBreakdown,
};

/// This backend's resolved daily-counts statement.
const VERDICT_DAILY_COUNTS_SQL: &str =
    verdict_daily_counts_sql!("to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD')");

impl_claim_verdict_repository!(PostgresDatabase, PgRow, VERDICT_DAILY_COUNTS_SQL);
