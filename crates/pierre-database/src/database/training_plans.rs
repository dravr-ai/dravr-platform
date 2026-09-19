// ABOUTME: SQLite-backed TrainingPlanRepository, emitted from the shared implementation in repositories/training_plans.rs
// ABOUTME: Outline + week supersession in one transaction per save; every statement is written once, in the shared module
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::{AppError, AppResult};
use pierre_memory::training_plans::{GoalRace, PlanWeek, TrainingPlan};
use sqlx::Row;
use uuid::Uuid;

use crate::database::Database;
use crate::repositories::training_plans::{
    built_plan_week, built_training_plan, impl_training_plan_repository, phase_index_column,
    plan_insert_values, plan_row, plan_week_from_row, training_plan_from_row, week_insert_values,
    week_row, BuiltPlan, BuiltWeek, PlanOwner, PlanWeekInput, SavePlanBundleParams,
    SaveTrainingPlanParams, SavedPlanBundle, TrainingPlanRepository, ACTIVE_PLAN_SQL,
    AGNOSTIC_PLAN_SLUG, INSERT_PLAN_SQL, INSERT_WEEK_SQL, LIST_ACTIVE_PLAN_WEEKS_SQL,
    LIST_ALL_PLAN_WEEKS_SQL, SUPERSEDE_ACTIVE_PLAN_SQL, SUPERSEDE_ACTIVE_WEEK_SQL,
    SUPERSEDE_CARRIED_WEEKS_SQL,
};

impl_training_plan_repository!(Database, sqlx::Sqlite);
