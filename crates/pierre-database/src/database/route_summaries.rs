// ABOUTME: SQLite implementation of RouteSummaryRepository (Endurance Phase 3)
// ABOUTME: Caches parsed-GPX terrain + climbs JSON keyed by (tenant_id, user_id, activity_id)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use sqlx::Row;
use uuid::Uuid;

use crate::database::Database;
use crate::repositories::RouteSummaryRepository;

#[async_trait]
impl RouteSummaryRepository for Database {
    async fn upsert_route_summary(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
        activity_id: &str,
        gpx_hash: &str,
        terrain_summary_json: &str,
        climbs_json: &str,
    ) -> AppResult<()> {
        sqlx::query(
            r"
            INSERT INTO route_summaries (
                tenant_id, user_id, activity_id, gpx_hash,
                terrain_summary_json, climbs_json, computed_at
            )
            VALUES (?, ?, ?, ?, ?, ?, datetime('now'))
            ON CONFLICT(tenant_id, user_id, activity_id) DO UPDATE SET
                gpx_hash = excluded.gpx_hash,
                terrain_summary_json = excluded.terrain_summary_json,
                climbs_json = excluded.climbs_json,
                computed_at = datetime('now')
            ",
        )
        .bind(tenant_id.to_string())
        .bind(user_id.to_string())
        .bind(activity_id)
        .bind(gpx_hash)
        .bind(terrain_summary_json)
        .bind(climbs_json)
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
            WHERE tenant_id = ? AND user_id = ? AND activity_id = ?
            LIMIT 1
            ",
        )
        .bind(tenant_id.to_string())
        .bind(user_id.to_string())
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
        let terrain: String = row
            .try_get("terrain_summary_json")
            .map_err(|e| AppError::database(format!("read terrain_summary_json: {e}")))?;
        let climbs: String = row
            .try_get("climbs_json")
            .map_err(|e| AppError::database(format!("read climbs_json: {e}")))?;
        Ok(Some((terrain, climbs)))
    }
}
