// ABOUTME: PostgreSQL-backed CoachAttentionDismissalRepository, emitted from the shared body in repositories/coach_attention_dismissals.rs
// ABOUTME: Postgres stores user ids as native uuid columns, so the shell passes the native uuid codec
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use pierre_core::errors::AppResult;
use pierre_core::models::TenantId;
use sqlx::Row;
use uuid::Uuid;

use super::PostgresDatabase;
use crate::repositories::coach_attention_dismissals::{
    dismissal_error, impl_coach_attention_dismissal_repository, AttentionDismissal,
    AttentionDismissalKey, CoachAttentionDismissalRepository, DISMISS_ATTENTION_SQL,
    LIST_ATTENTION_DISMISSALS_SQL, RESTORE_ATTENTION_SQL,
};
use crate::repositories::uuid_columns::NativeUuid;

impl_coach_attention_dismissal_repository!(PostgresDatabase, NativeUuid);
