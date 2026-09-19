// ABOUTME: PostgreSQL-backed PlaybookRepository, emitted from the shared implementation in repositories/playbooks.rs
// ABOUTME: Atomic ON CONFLICT counter upsert returning the surviving id, plus tenant-scoped reads
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::Utc;
use pierre_core::errors::{AppError, AppResult};
use pierre_memory::playbooks::{
    ArchetypePrior, LabelSource, OutcomeLabel, PendingAdvice, Playbook,
};
use sqlx::Row;
use tracing::warn;

use crate::backends::postgres::PostgresDatabase;
use crate::repositories::playbooks::{
    agg_row, impl_playbook_repository, list_archetype_priors_sql, outcome_upsert_values,
    pending_advice_from_row, pending_row, playbook_from_row, playbook_row, prior_from_row,
    rank_by_confidence, ArchetypePriorUpsert, PlaybookAggInput, PlaybookRepository,
    RecordedOutcome, AGGREGATE_PLAYBOOK_ROWS_SQL, ARCHETYPE_PRIOR_FETCH_CEILING,
    DELETE_ARCHETYPE_PRIOR_SQL, DELETE_PLAYBOOK_SQL, DUE_PENDING_ADVICE_SQL,
    INSERT_PENDING_ADVICE_SQL, LIST_ALL_USER_PLAYBOOKS_SQL, LIST_PLAYBOOKS_SQL,
    MARK_ADVICE_EXPIRED_SQL, MARK_ADVICE_LABELED_SQL, PLAYBOOK_FETCH_CEILING,
    PURGE_PLAYBOOK_ADVICE_SQL, UPSERT_ARCHETYPE_PRIOR_SQL, UPSERT_OUTCOME_SQL,
};

impl_playbook_repository!(PostgresDatabase);
