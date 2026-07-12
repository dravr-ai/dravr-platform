// ABOUTME: SQLite-backed TrainingPlanRepository — outline + week supersession in transactions
// ABOUTME: Tenant-scoped reads; mirrors backends/postgres/training_plans.rs
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use pierre_core::errors::{AppError, AppResult};
use pierre_memory::training_plans::{PlanStatus, PlanWeek, TrainingPlan, WeekStatus};
use sqlx::sqlite::SqliteRow;
use sqlx::Row;

use crate::database::Database;
use crate::repositories::training_plans::{
    plan_insert_values, plan_week_from_row, training_plan_from_row, week_insert_values,
    PlanWeekRow, SavePlanWeekParams, SaveTrainingPlanParams, TrainingPlanRepository,
    TrainingPlanRow,
};

/// Column list shared by every outline read so row mapping stays aligned.
const PLAN_COLUMNS: &str = "id, tenant_id, user_id, coach_slug, goal_fact_id, goal_race_json, \
     races_json, strategy, blocks_json, status, supersedes_id, source_conversation_id, \
     created_at, updated_at";

/// Column list shared by every week read.
const WEEK_COLUMNS: &str = "id, tenant_id, user_id, plan_id, week_start, focus, days_json, \
     status, supersedes_id, adjustment_reason, created_at, updated_at";

fn plan_row(row: &SqliteRow) -> AppResult<TrainingPlanRow> {
    Ok(TrainingPlanRow {
        id: row.try_get("id").map_err(map_col("id"))?,
        tenant_id: row.try_get("tenant_id").map_err(map_col("tenant_id"))?,
        user_id: row.try_get("user_id").map_err(map_col("user_id"))?,
        coach_slug: row.try_get("coach_slug").map_err(map_col("coach_slug"))?,
        goal_fact_id: row
            .try_get("goal_fact_id")
            .map_err(map_col("goal_fact_id"))?,
        goal_race_json: row
            .try_get("goal_race_json")
            .map_err(map_col("goal_race_json"))?,
        races_json: row.try_get("races_json").map_err(map_col("races_json"))?,
        strategy: row.try_get("strategy").map_err(map_col("strategy"))?,
        blocks_json: row.try_get("blocks_json").map_err(map_col("blocks_json"))?,
        status: row.try_get("status").map_err(map_col("status"))?,
        supersedes_id: row
            .try_get("supersedes_id")
            .map_err(map_col("supersedes_id"))?,
        source_conversation_id: row
            .try_get("source_conversation_id")
            .map_err(map_col("source_conversation_id"))?,
        created_at: row.try_get("created_at").map_err(map_col("created_at"))?,
        updated_at: row.try_get("updated_at").map_err(map_col("updated_at"))?,
    })
}

fn week_row(row: &SqliteRow) -> AppResult<PlanWeekRow> {
    Ok(PlanWeekRow {
        id: row.try_get("id").map_err(map_col("id"))?,
        tenant_id: row.try_get("tenant_id").map_err(map_col("tenant_id"))?,
        user_id: row.try_get("user_id").map_err(map_col("user_id"))?,
        plan_id: row.try_get("plan_id").map_err(map_col("plan_id"))?,
        week_start: row.try_get("week_start").map_err(map_col("week_start"))?,
        focus: row.try_get("focus").map_err(map_col("focus"))?,
        days_json: row.try_get("days_json").map_err(map_col("days_json"))?,
        status: row.try_get("status").map_err(map_col("status"))?,
        supersedes_id: row
            .try_get("supersedes_id")
            .map_err(map_col("supersedes_id"))?,
        adjustment_reason: row
            .try_get("adjustment_reason")
            .map_err(map_col("adjustment_reason"))?,
        created_at: row.try_get("created_at").map_err(map_col("created_at"))?,
        updated_at: row.try_get("updated_at").map_err(map_col("updated_at"))?,
    })
}

fn map_col(column: &'static str) -> impl Fn(sqlx::Error) -> AppError {
    move |e| AppError::database(format!("training plan column {column}: {e}"))
}

#[async_trait]
impl TrainingPlanRepository for Database {
    async fn save_training_plan(
        &self,
        params: &SaveTrainingPlanParams<'_>,
    ) -> AppResult<TrainingPlan> {
        let v = plan_insert_values(params)?;
        // Supersede-then-insert in one transaction so the one-active partial
        // unique index never sees two active outlines and a crash between the
        // two writes cannot strand the athlete planless.
        let mut tx = self
            .pool()
            .begin()
            .await
            .map_err(|e| AppError::database(format!("begin plan tx: {e}")))?;
        let superseded: Option<String> = sqlx::query_scalar(
            "UPDATE training_plans SET status = 'superseded', updated_at = ?1 \
             WHERE tenant_id = ?2 AND user_id = ?3 AND coach_slug = ?4 AND status = 'active' \
             RETURNING id",
        )
        .bind(v.now)
        .bind(params.tenant_id)
        .bind(params.user_id)
        .bind(&v.coach_slug)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| AppError::database(format!("supersede active plan: {e}")))?;
        sqlx::query(
            "INSERT INTO training_plans (id, tenant_id, user_id, coach_slug, goal_fact_id, \
             goal_race_json, races_json, strategy, blocks_json, status, supersedes_id, \
             source_conversation_id, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'active', ?10, ?11, ?12, ?12)",
        )
        .bind(&v.id)
        .bind(params.tenant_id)
        .bind(params.user_id)
        .bind(&v.coach_slug)
        .bind(params.goal_fact_id)
        .bind(&v.goal_race_json)
        .bind(&v.races_json)
        .bind(params.strategy)
        .bind(&v.blocks_json)
        .bind(superseded.as_deref())
        .bind(params.source_conversation_id)
        .bind(v.now)
        .execute(&mut *tx)
        .await
        .map_err(|e| AppError::database(format!("insert training plan: {e}")))?;
        tx.commit()
            .await
            .map_err(|e| AppError::database(format!("commit plan tx: {e}")))?;

        Ok(TrainingPlan {
            id: v.id,
            tenant_id: params.tenant_id.to_owned(),
            user_id: params.user_id.to_owned(),
            coach_slug: params.coach_slug.map(str::to_owned),
            goal_fact_id: params.goal_fact_id.map(str::to_owned),
            goal_race: params.goal_race.clone(),
            races: params.races.to_vec(),
            strategy: params.strategy.to_owned(),
            blocks: params.blocks.to_vec(),
            status: PlanStatus::Active,
            supersedes_id: superseded,
            source_conversation_id: params.source_conversation_id.map(str::to_owned),
            created_at: chrono::DateTime::from_timestamp(v.now, 0)
                .ok_or_else(|| AppError::internal("plan timestamp out of range"))?,
            updated_at: chrono::DateTime::from_timestamp(v.now, 0)
                .ok_or_else(|| AppError::internal("plan timestamp out of range"))?,
        })
    }

    async fn save_plan_week(&self, params: &SavePlanWeekParams<'_>) -> AppResult<PlanWeek> {
        let v = week_insert_values(params)?;
        let mut tx = self
            .pool()
            .begin()
            .await
            .map_err(|e| AppError::database(format!("begin week tx: {e}")))?;
        // The plan must exist for this tenant + user — a week must never
        // attach to another athlete's (or tenant's) plan.
        let plan_exists: Option<String> = sqlx::query_scalar(
            "SELECT id FROM training_plans WHERE id = ?1 AND tenant_id = ?2 AND user_id = ?3",
        )
        .bind(params.plan_id)
        .bind(params.tenant_id)
        .bind(params.user_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| AppError::database(format!("verify plan ownership: {e}")))?;
        if plan_exists.is_none() {
            return Err(AppError::invalid_input(format!(
                "no training plan {} for this user",
                params.plan_id
            )));
        }
        let superseded: Option<String> = sqlx::query_scalar(
            "UPDATE training_plan_weeks SET status = 'superseded', updated_at = ?1 \
             WHERE tenant_id = ?2 AND user_id = ?3 AND plan_id = ?4 AND week_start = ?5 \
             AND status = 'active' RETURNING id",
        )
        .bind(v.now)
        .bind(params.tenant_id)
        .bind(params.user_id)
        .bind(params.plan_id)
        .bind(params.week_start)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| AppError::database(format!("supersede active week: {e}")))?;
        sqlx::query(
            "INSERT INTO training_plan_weeks (id, tenant_id, user_id, plan_id, week_start, \
             focus, days_json, status, supersedes_id, adjustment_reason, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'active', ?8, ?9, ?10, ?10)",
        )
        .bind(&v.id)
        .bind(params.tenant_id)
        .bind(params.user_id)
        .bind(params.plan_id)
        .bind(params.week_start)
        .bind(params.focus)
        .bind(&v.days_json)
        .bind(superseded.as_deref())
        .bind(params.adjustment_reason)
        .bind(v.now)
        .execute(&mut *tx)
        .await
        .map_err(|e| AppError::database(format!("insert plan week: {e}")))?;
        tx.commit()
            .await
            .map_err(|e| AppError::database(format!("commit week tx: {e}")))?;

        Ok(PlanWeek {
            id: v.id,
            tenant_id: params.tenant_id.to_owned(),
            user_id: params.user_id.to_owned(),
            plan_id: params.plan_id.to_owned(),
            week_start: params.week_start.to_owned(),
            focus: params.focus.to_owned(),
            days: params.days.to_vec(),
            status: WeekStatus::Active,
            supersedes_id: superseded,
            adjustment_reason: params.adjustment_reason.to_owned(),
            created_at: chrono::DateTime::from_timestamp(v.now, 0)
                .ok_or_else(|| AppError::internal("week timestamp out of range"))?,
            updated_at: chrono::DateTime::from_timestamp(v.now, 0)
                .ok_or_else(|| AppError::internal("week timestamp out of range"))?,
        })
    }

    async fn get_active_plan(
        &self,
        tenant_id: &str,
        user_id: &str,
        coach_slug: Option<&str>,
    ) -> AppResult<Option<TrainingPlan>> {
        // Specific coach first, coach-agnostic ('') as fallback — the DESC
        // sort puts the non-empty slug ahead of ''.
        let sql = format!(
            "SELECT {PLAN_COLUMNS} FROM training_plans \
             WHERE tenant_id = ?1 AND user_id = ?2 AND coach_slug IN (?3, '') \
             AND status = 'active' ORDER BY coach_slug DESC LIMIT 1"
        );
        let row = sqlx::query(&sql)
            .bind(tenant_id)
            .bind(user_id)
            .bind(coach_slug.unwrap_or_default())
            .fetch_optional(self.pool())
            .await
            .map_err(|e| AppError::database(format!("get active plan: {e}")))?;
        row.map(|r| plan_row(&r).and_then(training_plan_from_row))
            .transpose()
    }

    async fn list_plan_weeks(
        &self,
        tenant_id: &str,
        user_id: &str,
        plan_id: &str,
        include_superseded: bool,
    ) -> AppResult<Vec<PlanWeek>> {
        let sql = if include_superseded {
            format!(
                "SELECT {WEEK_COLUMNS} FROM training_plan_weeks \
                 WHERE tenant_id = ?1 AND user_id = ?2 AND plan_id = ?3 \
                 ORDER BY week_start ASC, created_at ASC"
            )
        } else {
            format!(
                "SELECT {WEEK_COLUMNS} FROM training_plan_weeks \
                 WHERE tenant_id = ?1 AND user_id = ?2 AND plan_id = ?3 AND status = 'active' \
                 ORDER BY week_start ASC"
            )
        };
        let rows = sqlx::query(&sql)
            .bind(tenant_id)
            .bind(user_id)
            .bind(plan_id)
            .fetch_all(self.pool())
            .await
            .map_err(|e| AppError::database(format!("list plan weeks: {e}")))?;
        rows.iter()
            .map(|r| week_row(r).and_then(plan_week_from_row))
            .collect()
    }
}
