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
use uuid::Uuid;

use crate::database::Database;
use crate::repositories::training_plans::{
    built_plan_week, built_training_plan, plan_insert_values, plan_week_from_row,
    training_plan_from_row, week_insert_values, BuiltPlan, BuiltWeek, PlanWeekInput, PlanWeekRow,
    SavePlanBundleParams, SaveTrainingPlanParams, SavedPlanBundle, TrainingPlanRepository,
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

const SUPERSEDE_ACTIVE_PLAN_SQL: &str = "UPDATE training_plans SET status = 'superseded', \
     updated_at = ?1 WHERE tenant_id = ?2 AND user_id = ?3 AND coach_slug = ?4 \
     AND status = 'active' RETURNING id";

const INSERT_PLAN_SQL: &str = "INSERT INTO training_plans (id, tenant_id, user_id, coach_slug, \
     goal_fact_id, goal_race_json, races_json, strategy, blocks_json, status, supersedes_id, \
     source_conversation_id, created_at, updated_at) \
     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'active', ?10, ?11, ?12, ?12)";

const SUPERSEDE_ACTIVE_WEEK_SQL: &str = "UPDATE training_plan_weeks SET status = 'superseded', \
     updated_at = ?1 WHERE tenant_id = ?2 AND user_id = ?3 AND plan_id = ?4 AND week_start = ?5 \
     AND status = 'active' RETURNING id";

const INSERT_WEEK_SQL: &str = "INSERT INTO training_plan_weeks (id, tenant_id, user_id, plan_id, \
     week_start, focus, days_json, status, supersedes_id, adjustment_reason, created_at, \
     updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'active', ?8, ?9, ?10, ?10)";

/// Re-parent a superseded outline's surviving weeks onto the outline that
/// replaced it: every active week is flipped to `'superseded'` and re-inserted
/// against the new plan id with `supersedes_id` pointing back at it. Weeks are
/// keyed by `plan_id` and every read surface lists the *active* plan's weeks,
/// so a week left behind is a schedule the athlete can no longer see. Row
/// content is carried verbatim — `days_json` is never re-parsed — so only
/// identity changes and the migration's "new row with `supersedes_id` set, old
/// row `status='superseded'`" model holds across the plan boundary too.
async fn carry_forward_active_weeks(
    conn: &mut sqlx::SqliteConnection,
    tenant_id: &str,
    user_id: &str,
    old_plan_id: &str,
    new_plan_id: &str,
    now: i64,
) -> AppResult<()> {
    let sql = format!(
        "UPDATE training_plan_weeks SET status = 'superseded', updated_at = ?1 \
         WHERE tenant_id = ?2 AND user_id = ?3 AND plan_id = ?4 AND status = 'active' \
         RETURNING {WEEK_COLUMNS}"
    );
    let rows = sqlx::query(&sql)
        .bind(now)
        .bind(tenant_id)
        .bind(user_id)
        .bind(old_plan_id)
        .fetch_all(&mut *conn)
        .await
        .map_err(|e| AppError::database(format!("supersede carried weeks: {e}")))?;
    for row in &rows {
        let old = week_row(row)?;
        sqlx::query(INSERT_WEEK_SQL)
            .bind(Uuid::new_v4().to_string())
            .bind(tenant_id)
            .bind(user_id)
            .bind(new_plan_id)
            .bind(&old.week_start)
            .bind(&old.focus)
            .bind(&old.days_json)
            .bind(&old.id)
            .bind(&old.adjustment_reason)
            .bind(now)
            .execute(&mut *conn)
            .await
            .map_err(|e| AppError::database(format!("carry forward plan week: {e}")))?;
    }
    Ok(())
}

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
    // The new outline carries a new id, so the replaced outline's weeks move
    // with it in this same transaction — otherwise the athlete's schedule
    // stays on a plan no read surface fetches.
    if let Some(old_plan_id) = superseded.as_deref() {
        carry_forward_active_weeks(
            &mut *conn,
            params.tenant_id,
            params.user_id,
            old_plan_id,
            &v.id,
            v.now,
        )
        .await?;
    }
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

/// Supersede the active row for this `week_start` and insert the new week, on
/// an in-transaction connection. The caller resolves `plan_id` from a plan it
/// created or read for this tenant + user in the same transaction, so the week
/// can never attach to another athlete's (or tenant's) plan.
async fn supersede_and_insert_week(
    conn: &mut sqlx::SqliteConnection,
    tenant_id: &str,
    user_id: &str,
    plan_id: &str,
    week: &PlanWeekInput<'_>,
) -> AppResult<PlanWeek> {
    let v = week_insert_values(week.days)?;
    let superseded: Option<String> = sqlx::query_scalar(SUPERSEDE_ACTIVE_WEEK_SQL)
        .bind(v.now)
        .bind(tenant_id)
        .bind(user_id)
        .bind(plan_id)
        .bind(week.week_start)
        .fetch_optional(&mut *conn)
        .await
        .map_err(|e| AppError::database(format!("supersede active week: {e}")))?;
    sqlx::query(INSERT_WEEK_SQL)
        .bind(&v.id)
        .bind(tenant_id)
        .bind(user_id)
        .bind(plan_id)
        .bind(week.week_start)
        .bind(week.focus)
        .bind(&v.days_json)
        .bind(superseded.as_deref())
        .bind(week.adjustment_reason)
        .bind(v.now)
        .execute(&mut *conn)
        .await
        .map_err(|e| AppError::database(format!("insert plan week: {e}")))?;
    built_plan_week(BuiltWeek {
        id: v.id,
        tenant_id,
        user_id,
        plan_id,
        week_start: week.week_start,
        focus: week.focus,
        days: week.days,
        superseded,
        adjustment_reason: week.adjustment_reason,
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
            weeks.push(
                supersede_and_insert_week(&mut tx, params.tenant_id, params.user_id, &plan.id, w)
                    .await?,
            );
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
