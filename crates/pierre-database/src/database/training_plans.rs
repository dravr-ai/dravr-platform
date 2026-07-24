// ABOUTME: SQLite-backed TrainingPlanRepository — outline + week supersession in transactions
// ABOUTME: Tenant-scoped reads; mirrors backends/postgres/training_plans.rs
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use pierre_core::errors::{AppError, AppResult};
use pierre_memory::training_plans::{PlanWeek, TrainingPlan};
use sqlx::sqlite::SqliteRow;
use sqlx::Row;

use crate::database::Database;
use crate::repositories::training_plans::{
    built_plan_week, built_training_plan, plan_insert_values, plan_week_from_row,
    training_plan_from_row, week_insert_values, BuiltPlan, BuiltWeek, PlanWeekRow,
    SavePlanBundleParams, SavePlanWeekParams, SaveTrainingPlanParams, SavedPlanBundle,
    TrainingPlanRepository, TrainingPlanRow,
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

const SUPERSEDE_ACTIVE_PLAN_SQL: &str = "UPDATE training_plans SET status = 'superseded', \
     updated_at = ?1 WHERE tenant_id = ?2 AND user_id = ?3 AND coach_slug = ?4 \
     AND status = 'active' RETURNING id";

const INSERT_PLAN_SQL: &str = "INSERT INTO training_plans (id, tenant_id, user_id, coach_slug, \
     goal_fact_id, goal_race_json, races_json, strategy, blocks_json, status, supersedes_id, \
     source_conversation_id, created_at, updated_at) \
     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'active', ?10, ?11, ?12, ?12)";

const VERIFY_PLAN_OWNED_SQL: &str =
    "SELECT id FROM training_plans WHERE id = ?1 AND tenant_id = ?2 AND user_id = ?3";

const SUPERSEDE_ACTIVE_WEEK_SQL: &str = "UPDATE training_plan_weeks SET status = 'superseded', \
     updated_at = ?1 WHERE tenant_id = ?2 AND user_id = ?3 AND plan_id = ?4 AND week_start = ?5 \
     AND status = 'active' RETURNING id";

const INSERT_WEEK_SQL: &str = "INSERT INTO training_plan_weeks (id, tenant_id, user_id, plan_id, \
     week_start, focus, days_json, status, supersedes_id, adjustment_reason, created_at, \
     updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'active', ?8, ?9, ?10, ?10)";

/// Supersede the current active outline and insert the new one, on an
/// in-transaction connection. The caller owns the transaction envelope.
async fn supersede_and_insert_plan(
    conn: &mut sqlx::SqliteConnection,
    params: &SaveTrainingPlanParams<'_>,
) -> AppResult<TrainingPlan> {
    let v = plan_insert_values(params)?;
    let superseded: Option<String> = sqlx::query_scalar(SUPERSEDE_ACTIVE_PLAN_SQL)
        .bind(v.now)
        .bind(params.tenant_id)
        .bind(params.user_id)
        .bind(&v.coach_slug)
        .fetch_optional(&mut *conn)
        .await
        .map_err(|e| AppError::database(format!("supersede active plan: {e}")))?;
    sqlx::query(INSERT_PLAN_SQL)
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
        .execute(&mut *conn)
        .await
        .map_err(|e| AppError::database(format!("insert training plan: {e}")))?;
    built_training_plan(BuiltPlan {
        id: v.id,
        tenant_id: params.tenant_id,
        user_id: params.user_id,
        coach_slug: params.coach_slug,
        goal_fact_id: params.goal_fact_id,
        goal_race: params.goal_race,
        races: params.races,
        strategy: params.strategy,
        blocks: params.blocks,
        superseded,
        source_conversation_id: params.source_conversation_id,
        now: v.now,
    })
}

/// Verify the plan exists for this tenant + user — a week must never attach to
/// another athlete's (or tenant's) plan.
async fn verify_plan_owned(
    conn: &mut sqlx::SqliteConnection,
    tenant_id: &str,
    user_id: &str,
    plan_id: &str,
) -> AppResult<()> {
    let owned: Option<String> = sqlx::query_scalar(VERIFY_PLAN_OWNED_SQL)
        .bind(plan_id)
        .bind(tenant_id)
        .bind(user_id)
        .fetch_optional(&mut *conn)
        .await
        .map_err(|e| AppError::database(format!("verify plan ownership: {e}")))?;
    if owned.is_none() {
        return Err(AppError::invalid_input(format!(
            "no training plan {plan_id} for this user"
        )));
    }
    Ok(())
}

/// Supersede the active row for this `week_start` and insert the new week, on
/// an in-transaction connection. Does not verify plan ownership — the caller
/// must have created or verified the plan in the same transaction.
async fn supersede_and_insert_week(
    conn: &mut sqlx::SqliteConnection,
    params: &SavePlanWeekParams<'_>,
) -> AppResult<PlanWeek> {
    let v = week_insert_values(params)?;
    let superseded: Option<String> = sqlx::query_scalar(SUPERSEDE_ACTIVE_WEEK_SQL)
        .bind(v.now)
        .bind(params.tenant_id)
        .bind(params.user_id)
        .bind(params.plan_id)
        .bind(params.week_start)
        .fetch_optional(&mut *conn)
        .await
        .map_err(|e| AppError::database(format!("supersede active week: {e}")))?;
    sqlx::query(INSERT_WEEK_SQL)
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
        .execute(&mut *conn)
        .await
        .map_err(|e| AppError::database(format!("insert plan week: {e}")))?;
    built_plan_week(BuiltWeek {
        id: v.id,
        tenant_id: params.tenant_id,
        user_id: params.user_id,
        plan_id: params.plan_id,
        week_start: params.week_start,
        focus: params.focus,
        days: params.days,
        superseded,
        adjustment_reason: params.adjustment_reason,
        now: v.now,
    })
}

/// Read the athlete's active outline on an in-transaction connection (specific
/// coach first, coach-agnostic `''` as fallback).
async fn resolve_active_plan(
    conn: &mut sqlx::SqliteConnection,
    tenant_id: &str,
    user_id: &str,
    coach_slug: Option<&str>,
) -> AppResult<Option<TrainingPlan>> {
    let sql = format!(
        "SELECT {PLAN_COLUMNS} FROM training_plans \
         WHERE tenant_id = ?1 AND user_id = ?2 AND coach_slug IN (?3, '') \
         AND status = 'active' ORDER BY coach_slug DESC LIMIT 1"
    );
    let row = sqlx::query(&sql)
        .bind(tenant_id)
        .bind(user_id)
        .bind(coach_slug.unwrap_or_default())
        .fetch_optional(&mut *conn)
        .await
        .map_err(|e| AppError::database(format!("get active plan: {e}")))?;
    row.map(|r| plan_row(&r).and_then(training_plan_from_row))
        .transpose()
}

#[async_trait]
impl TrainingPlanRepository for Database {
    async fn save_training_plan(
        &self,
        params: &SaveTrainingPlanParams<'_>,
    ) -> AppResult<TrainingPlan> {
        // Supersede-then-insert in one transaction so the one-active partial
        // unique index never sees two active outlines and a crash between the
        // two writes cannot strand the athlete planless.
        let mut tx = self
            .pool()
            .begin()
            .await
            .map_err(|e| AppError::database(format!("begin plan tx: {e}")))?;
        let plan = supersede_and_insert_plan(&mut tx, params).await?;
        tx.commit()
            .await
            .map_err(|e| AppError::database(format!("commit plan tx: {e}")))?;
        Ok(plan)
    }

    async fn save_plan_week(&self, params: &SavePlanWeekParams<'_>) -> AppResult<PlanWeek> {
        let mut tx = self
            .pool()
            .begin()
            .await
            .map_err(|e| AppError::database(format!("begin week tx: {e}")))?;
        verify_plan_owned(&mut tx, params.tenant_id, params.user_id, params.plan_id).await?;
        let week = supersede_and_insert_week(&mut tx, params).await?;
        tx.commit()
            .await
            .map_err(|e| AppError::database(format!("commit week tx: {e}")))?;
        Ok(week)
    }

    async fn save_plan_bundle(
        &self,
        params: &SavePlanBundleParams<'_>,
    ) -> AppResult<SavedPlanBundle> {
        let mut tx = self
            .pool()
            .begin()
            .await
            .map_err(|e| AppError::database(format!("begin bundle tx: {e}")))?;

        // Resolve the plan the weeks attach to — a fresh outline (superseding
        // the current active one) or the existing active plan — inside the
        // transaction so the outline supersession and every week either all
        // commit or all roll back.
        let (plan, superseded_plan_id) = if let Some(o) = &params.outline {
            let stp = SaveTrainingPlanParams {
                tenant_id: params.tenant_id,
                user_id: params.user_id,
                coach_slug: params.coach_slug,
                goal_fact_id: params.goal_fact_id,
                goal_race: o.goal_race,
                races: o.races,
                strategy: o.strategy,
                blocks: o.blocks,
                source_conversation_id: o.source_conversation_id,
            };
            let plan = supersede_and_insert_plan(&mut tx, &stp).await?;
            let superseded = plan.supersedes_id.clone();
            (plan, superseded)
        } else {
            let plan =
                resolve_active_plan(&mut tx, params.tenant_id, params.user_id, params.coach_slug)
                    .await?
                    .ok_or_else(|| {
                        AppError::invalid_input(
                            "no active plan to attach weeks to — save an outline first",
                        )
                    })?;
            (plan, None)
        };

        let mut weeks = Vec::with_capacity(params.weeks.len());
        for w in params.weeks {
            let swp = SavePlanWeekParams {
                tenant_id: params.tenant_id,
                user_id: params.user_id,
                plan_id: &plan.id,
                week_start: w.week_start,
                focus: w.focus,
                days: w.days,
                adjustment_reason: w.adjustment_reason,
            };
            weeks.push(supersede_and_insert_week(&mut tx, &swp).await?);
        }

        tx.commit()
            .await
            .map_err(|e| AppError::database(format!("commit bundle tx: {e}")))?;
        Ok(SavedPlanBundle {
            plan,
            weeks,
            superseded_plan_id,
        })
    }

    async fn get_active_plan(
        &self,
        tenant_id: &str,
        user_id: &str,
        coach_slug: Option<&str>,
    ) -> AppResult<Option<TrainingPlan>> {
        // Specific coach first, coach-agnostic ('') as fallback — shares the
        // in-transaction resolver so the SELECT lives in one place.
        let mut conn =
            self.pool().acquire().await.map_err(|e| {
                AppError::database(format!("acquire conn for get active plan: {e}"))
            })?;
        resolve_active_plan(&mut conn, tenant_id, user_id, coach_slug).await
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
