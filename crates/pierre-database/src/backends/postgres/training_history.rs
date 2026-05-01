// ABOUTME: PostgreSQL implementation of TrainingHistoryRepository (Endurance Phase 2)
// ABOUTME: Mirrors crates/pierre-database/src/database/training_history.rs for the Postgres tier
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use chrono::NaiveDate;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{DailyTrainingState, TenantId};
use sqlx::postgres::PgRow;
use sqlx::Row;
use uuid::Uuid;

use super::PostgresDatabase;
use crate::repositories::TrainingHistoryRepository;

#[async_trait]
impl TrainingHistoryRepository for PostgresDatabase {
    async fn upsert_training_history_day(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
        state: &DailyTrainingState,
    ) -> AppResult<()> {
        sqlx::query(
            r"
            INSERT INTO training_history (
                tenant_id, user_id, date, ctl, atl, tsb,
                acwr, monotony, strain, ramp_rate, daily_load, computed_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, NOW())
            ON CONFLICT (tenant_id, user_id, date) DO UPDATE SET
                ctl = EXCLUDED.ctl,
                atl = EXCLUDED.atl,
                tsb = EXCLUDED.tsb,
                acwr = EXCLUDED.acwr,
                monotony = EXCLUDED.monotony,
                strain = EXCLUDED.strain,
                ramp_rate = EXCLUDED.ramp_rate,
                daily_load = EXCLUDED.daily_load,
                computed_at = NOW()
            ",
        )
        .bind(tenant_id.as_uuid())
        .bind(user_id)
        .bind(state.date)
        .bind(state.ctl)
        .bind(state.atl)
        .bind(state.tsb)
        .bind(state.acwr)
        .bind(state.monotony)
        .bind(state.strain)
        .bind(state.ramp_rate)
        .bind(state.daily_load)
        .execute(self.pool())
        .await
        .map_err(|e| AppError::database(format!("upsert_training_history_day: {e}")))?;
        Ok(())
    }

    async fn upsert_training_history_batch(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
        states: &[DailyTrainingState],
    ) -> AppResult<()> {
        let mut tx = self
            .pool()
            .begin()
            .await
            .map_err(|e| AppError::database(format!("begin tx: {e}")))?;
        for state in states {
            sqlx::query(
                r"
                INSERT INTO training_history (
                    tenant_id, user_id, date, ctl, atl, tsb,
                    acwr, monotony, strain, ramp_rate, daily_load, computed_at
                )
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, NOW())
                ON CONFLICT (tenant_id, user_id, date) DO UPDATE SET
                    ctl = EXCLUDED.ctl,
                    atl = EXCLUDED.atl,
                    tsb = EXCLUDED.tsb,
                    acwr = EXCLUDED.acwr,
                    monotony = EXCLUDED.monotony,
                    strain = EXCLUDED.strain,
                    ramp_rate = EXCLUDED.ramp_rate,
                    daily_load = EXCLUDED.daily_load,
                    computed_at = NOW()
                ",
            )
            .bind(tenant_id.as_uuid())
            .bind(user_id)
            .bind(state.date)
            .bind(state.ctl)
            .bind(state.atl)
            .bind(state.tsb)
            .bind(state.acwr)
            .bind(state.monotony)
            .bind(state.strain)
            .bind(state.ramp_rate)
            .bind(state.daily_load)
            .execute(&mut *tx)
            .await
            .map_err(|e| AppError::database(format!("upsert_training_history_batch: {e}")))?;
        }
        tx.commit()
            .await
            .map_err(|e| AppError::database(format!("commit tx: {e}")))?;
        Ok(())
    }

    async fn get_training_history(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
        from: NaiveDate,
        to: NaiveDate,
    ) -> AppResult<Vec<DailyTrainingState>> {
        let rows = sqlx::query(
            r"
            SELECT date, ctl, atl, tsb, acwr, monotony, strain, ramp_rate, daily_load
            FROM training_history
            WHERE tenant_id = $1 AND user_id = $2
              AND date BETWEEN $3 AND $4
            ORDER BY date ASC
            ",
        )
        .bind(tenant_id.as_uuid())
        .bind(user_id)
        .bind(from)
        .bind(to)
        .fetch_all(self.pool())
        .await
        .map_err(|e| AppError::database(format!("get_training_history: {e}")))?;

        rows.iter().map(row_to_state).collect()
    }

    async fn latest_training_history(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
    ) -> AppResult<Option<DailyTrainingState>> {
        let row = sqlx::query(
            r"
            SELECT date, ctl, atl, tsb, acwr, monotony, strain, ramp_rate, daily_load
            FROM training_history
            WHERE tenant_id = $1 AND user_id = $2
            ORDER BY date DESC
            LIMIT 1
            ",
        )
        .bind(tenant_id.as_uuid())
        .bind(user_id)
        .fetch_optional(self.pool())
        .await
        .map_err(|e| AppError::database(format!("latest_training_history: {e}")))?;

        row.as_ref().map(row_to_state).transpose()
    }
}

fn row_to_state(row: &PgRow) -> AppResult<DailyTrainingState> {
    let date: NaiveDate = row
        .try_get("date")
        .map_err(|e| AppError::database(format!("read date: {e}")))?;
    Ok(DailyTrainingState {
        date,
        ctl: row.try_get("ctl").unwrap_or(0.0),
        atl: row.try_get("atl").unwrap_or(0.0),
        tsb: row.try_get("tsb").unwrap_or(0.0),
        acwr: row
            .try_get::<Option<f64>, _>("acwr")
            .map_err(|e| AppError::database(format!("read acwr: {e}")))?,
        monotony: row
            .try_get::<Option<f64>, _>("monotony")
            .map_err(|e| AppError::database(format!("read monotony: {e}")))?,
        strain: row
            .try_get::<Option<f64>, _>("strain")
            .map_err(|e| AppError::database(format!("read strain: {e}")))?,
        ramp_rate: row
            .try_get::<Option<f64>, _>("ramp_rate")
            .map_err(|e| AppError::database(format!("read ramp_rate: {e}")))?,
        daily_load: row.try_get("daily_load").unwrap_or(0.0),
    })
}
