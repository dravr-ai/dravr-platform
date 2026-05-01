// ABOUTME: PostgreSQL implementation of RouteSummaryRepository (Endurance Phase 3)
// ABOUTME: Mirrors crates/pierre-database/src/database/route_summaries.rs for the Postgres tier
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use serde_json::Value;
use sqlx::Row;
use uuid::Uuid;

use super::PostgresDatabase;
use crate::repositories::RouteSummaryRepository;

#[async_trait]
impl RouteSummaryRepository for PostgresDatabase {
    async fn upsert_route_summary(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
        activity_id: &str,
        gpx_hash: &str,
        terrain_summary_json: &str,
        climbs_json: &str,
    ) -> AppResult<()> {
        let terrain_value: Value = serde_json::from_str(terrain_summary_json)
            .map_err(|e| AppError::database(format!("parse terrain_summary_json: {e}")))?;
        let climbs_value: Value = serde_json::from_str(climbs_json)
            .map_err(|e| AppError::database(format!("parse climbs_json: {e}")))?;
        sqlx::query(
            r"
            INSERT INTO route_summaries (
                tenant_id, user_id, activity_id, gpx_hash,
                terrain_summary_json, climbs_json, computed_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, NOW())
            ON CONFLICT (tenant_id, user_id, activity_id) DO UPDATE SET
                gpx_hash = EXCLUDED.gpx_hash,
                terrain_summary_json = EXCLUDED.terrain_summary_json,
                climbs_json = EXCLUDED.climbs_json,
                computed_at = NOW()
            ",
        )
        .bind(tenant_id.as_uuid())
        .bind(user_id)
        .bind(activity_id)
        .bind(gpx_hash)
        .bind(terrain_value)
        .bind(climbs_value)
        .execute(self.pool())
        .await
        .map_err(|e| AppError::database(format!("upsert_route_summary: {e}")))?;
        Ok(())
    }

    async fn get_route_summary(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
        activity_id: &str,
        expected_hash: &str,
    ) -> AppResult<Option<(String, String)>> {
        let row = sqlx::query(
            r"
            SELECT gpx_hash, terrain_summary_json, climbs_json
            FROM route_summaries
            WHERE tenant_id = $1 AND user_id = $2 AND activity_id = $3
            LIMIT 1
            ",
        )
        .bind(tenant_id.as_uuid())
        .bind(user_id)
        .bind(activity_id)
        .fetch_optional(self.pool())
        .await
        .map_err(|e| AppError::database(format!("get_route_summary: {e}")))?;

        let Some(row) = row else { return Ok(None) };
        let stored_hash: String = row
            .try_get("gpx_hash")
            .map_err(|e| AppError::database(format!("read gpx_hash: {e}")))?;
        if stored_hash != expected_hash {
            return Ok(None);
        }
        let terrain: Value = row
            .try_get("terrain_summary_json")
            .map_err(|e| AppError::database(format!("read terrain_summary_json: {e}")))?;
        let climbs: Value = row
            .try_get("climbs_json")
            .map_err(|e| AppError::database(format!("read climbs_json: {e}")))?;
        let terrain_str = serde_json::to_string(&terrain)
            .map_err(|e| AppError::database(format!("serialize terrain: {e}")))?;
        let climbs_str = serde_json::to_string(&climbs)
            .map_err(|e| AppError::database(format!("serialize climbs: {e}")))?;
        Ok(Some((terrain_str, climbs_str)))
    }
}
