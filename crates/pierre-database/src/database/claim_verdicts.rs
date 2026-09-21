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
    ClaimCategory, ClaimStatus, ClaimVerdict, DispositionReason, VerdictLayer,
};
use sqlx::Row;
use uuid::Uuid;

use super::Database;
use crate::repositories::claim_verdicts::{
    agent_health, apply_status_count, category_health, daily_health, fold_health_breakdown,
    health_breakdown_rows, health_totals, impl_claim_verdict_repository, layer_health,
    reason_count_from_row, verdict_daily_counts_sql, verdict_from_row,
    verdict_health_breakdown_sql, DispositionFilter, GET_VERDICT_SQL, HEALTH_BY_AGENT_SQL,
    HEALTH_BY_CATEGORY_SQL, HEALTH_BY_LAYER_SQL, HEALTH_BY_REASON_SQL, INSERT_CLAIM_VERDICT_SQL,
    LIST_VERDICTS_FILTERED_SQL, LIST_VERDICTS_FOR_CONVERSATION_SQL, LIST_VERDICTS_FOR_MESSAGE_SQL,
    SET_VERDICT_DISPOSITION_SQL,
};
use crate::repositories::{
    ClaimVerdictRepository, InsertClaimVerdictParams, SetVerdictDispositionParams,
    VerdictCalibrationStats, VerdictDailyBucket, VerdictHealthStats, VerdictListFilter,
    VerdictStatusBreakdown,
};

/// This backend's resolved daily-counts statement.
const VERDICT_DAILY_COUNTS_SQL: &str = verdict_daily_counts_sql!("substr(created_at, 1, 10)");

/// This backend's resolved daily health-breakdown statement.
const VERDICT_HEALTH_DAILY_SQL: &str = verdict_health_breakdown_sql!("substr(created_at, 1, 10)");

impl_claim_verdict_repository!(Database, VERDICT_DAILY_COUNTS_SQL, VERDICT_HEALTH_DAILY_SQL);
