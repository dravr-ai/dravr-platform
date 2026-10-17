// ABOUTME: SQLite-backed TrainingPlanRepository, emitted from the shared implementation in repositories/training_plans.rs
// ABOUTME: Outline + week supersession in one transaction per save; every statement is written once, in the shared module
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::Utc;
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::transport::TransportPolicy;
use pierre_memory::training_plans::{GoalRace, PlanWeek, TrainingPlan};
use sqlx::Row;
use uuid::Uuid;

use crate::database::Database;
use crate::repositories::training_plans::{
    built_plan_week, built_training_plan, draft_insert_error, draft_outline_params,
    impl_training_plan_draft_helpers, impl_training_plan_repository, phase_index_column,
    plan_draft_from_row, plan_insert_values, plan_row, plan_week_from_row, training_plan_from_row,
    week_insert_values, week_row, BuiltPlan, BuiltWeek, DraftConflicts, DraftOwner, DraftPreview,
    DraftPublish, PlanAuthor, PlanDraft, PlanOutlineInput, PlanWeekInput, PublishDraftParams,
    PublishedDraft, SaveDraftParams, SavePlanBundleParams, SaveTrainingPlanParams,
    SavedDraftBundle, SavedPlanBundle, TrainingPlanRepository, TrainingPlanRow, WeekInsertValues,
    WeekTarget, ACTIVE_PLAN_SQL, CLAIM_ACTIVE_PLAN_SQL, CLAIM_DRAFT_SQL, DISCARD_DRAFT_SQL,
    DRAFT_PLAN_SQL, DRAFT_WEEK_CONFLICTS_SQL, INSERT_DRAFT_FROM_LIVE_SQL, INSERT_DRAFT_SQL,
    INSERT_PLAN_SQL, INSERT_WEEK_SQL, LIST_ACTIVE_PLAN_WEEKS_SQL, LIST_ALL_PLAN_WEEKS_SQL,
    LIST_DRAFT_WEEKS_SQL, LIVE_WEEK_ID_SQL, PUBLISHED_DRAFT_SQL, RELAY_DRAFT_OUTLINE_SQL,
    SUPERSEDE_ACTIVE_PLAN_SQL, SUPERSEDE_ACTIVE_WEEK_SQL, SUPERSEDE_CARRIED_WEEKS_SQL,
    SUPERSEDE_OWN_ACTIVE_PLAN_SQL,
};

impl_training_plan_draft_helpers!(sqlx::Sqlite);
impl_training_plan_repository!(Database, sqlx::Sqlite);
