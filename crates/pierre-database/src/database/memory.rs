// ABOUTME: SQLite-backed HarnessMemoryRepository, emitted from the shared implementation in repositories/memory.rs
// ABOUTME: Compaction blocks, user facts, agent notes, followups and sessions; timestamps are RFC3339 TEXT here
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{Pillar, TenantId};
use pierre_memory::{
    AgentFollowup, AgentNote, AgentSession, CompactionBlock, FactKind, FactSource, FollowupStatus,
    PredicateCode, SessionStatus, UserFact, UserFactMetrics,
};
use uuid::Uuid;

use super::Database;
use crate::repositories::memory::{
    agent_followup_from_row, agent_note_from_row, agent_session_from_row,
    compaction_block_from_row, impl_harness_memory_repository, user_fact_from_row,
    user_fact_metrics_from_rows, ARCHIVE_AGENT_SESSION_SQL, CANCEL_FOLLOWUP_SQL,
    COUNT_USER_FACTS_BY_KIND_SQL, COUNT_USER_FACTS_SQL, DELETE_USER_FACT_SQL,
    EXPIRE_ONBOARDING_FACTS_SQL, GET_ACTIVE_AGENT_SESSION_SQL, GET_USER_FACT_BY_ID_SQL,
    GET_USER_FACT_SQL, INSERT_AGENT_FOLLOWUP_SQL, INSERT_AGENT_NOTE_SQL, INSERT_AGENT_SESSION_SQL,
    INSERT_COMPACTION_BLOCK_SQL, INSERT_USER_FACT_SQL, LIST_AGENT_NOTES_FOR_TENANT_SQL,
    LIST_AGENT_NOTES_SQL, LIST_COMPACTION_BLOCKS_SQL, LIST_DUE_FOLLOWUPS_SQL,
    LIST_PENDING_FOLLOWUPS_FOR_TENANT_SQL, LIST_PENDING_FOLLOWUPS_SQL,
    LIST_USER_FACTS_BY_AGENT_AND_KIND_SQL, LIST_USER_FACTS_BY_AGENT_SQL,
    LIST_USER_FACTS_BY_KIND_SQL, LIST_USER_FACTS_BY_SOURCE_SQL, LIST_USER_FACTS_SQL,
    MARK_FOLLOWUP_DELIVERED_SQL, MERGE_USER_FACT_SQL, SET_AGENT_NOTE_SUPPRESSED_SQL,
    TOUCH_AGENT_SESSION_SQL,
};
use crate::repositories::{
    HarnessMemoryRepository, InsertAgentFollowupParams, InsertAgentNoteParams,
    InsertCompactionBlockParams, MergeUserFactParams, UpsertUserFactParams,
};

impl_harness_memory_repository!(Database);
