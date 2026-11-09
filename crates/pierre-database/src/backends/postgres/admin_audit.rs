// ABOUTME: PostgreSQL-backed AdminAuditRepository, emitted from the shared implementation
// ABOUTME: The admin audit trail: one row per operator write, read newest first
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use serde_json::Value;
use sqlx::types::Json;
use sqlx::Row;
use uuid::Uuid;

use crate::backends::postgres::PostgresDatabase;
use crate::repositories::admin_audit::{
    action_prefix_pattern, audit_column_error, impl_admin_audit_repository, severity_for,
    AdminAuditRepository, AuditEventFilter, AuditEventRecord, AuditOutcome, NewAuditEvent,
    RecentReadKey, AUDIT_PAGE_MAX, HAS_RECENT_READ_SQL, INSERT_AUDIT_EVENT_SQL,
    LIST_AUDIT_EVENTS_SQL,
};
use crate::repositories::uuid_columns::NativeUuid;

impl_admin_audit_repository!(PostgresDatabase, NativeUuid);
