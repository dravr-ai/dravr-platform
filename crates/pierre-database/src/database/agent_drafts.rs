// ABOUTME: SQLite-backed AgentDraftsRepository, emitted from the shared implementation
// ABOUTME: Guarded single-winner UPDATE for the single-use claim; expiry filtered at resolution time
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};

use crate::database::Database;
use crate::repositories::agent_drafts::{
    action_from_row, impl_agent_drafts_repository, AgentDraftsRepository, ClaimOutcome,
    PendingAgentDraft, CLAIM_PENDING_ACTION_SQL, EXPIRE_PENDING_ACTION_SQL,
    INSERT_PENDING_ACTION_SQL, READ_CLAIMED_ACTION_SQL, SWEEP_PENDING_ACTIONS_SQL,
};

impl_agent_drafts_repository!(Database);
