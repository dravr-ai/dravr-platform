// ABOUTME: PostgreSQL implementation of the Tier 5.5 ClaimVerdictRepository trait
// ABOUTME: Mirrors the SQLite path for the bullshit detector verdict persistence
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::BTreeMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use pierre_memory::claims::{
    ClaimCategory, ClaimStatus, ClaimVerdict, EvidenceStrength, VerdictLayer,
};
use sqlx::postgres::PgRow;
use sqlx::Row;
use uuid::Uuid;

use super::PostgresDatabase;
use crate::repositories::{
    ClaimVerdictRepository, InsertClaimVerdictParams, VerdictCalibrationStats, VerdictDailyBucket,
    VerdictStatusBreakdown,
};

fn row_to_verdict(row: &PgRow) -> AppResult<ClaimVerdict> {
    let category_str: String = row.get("category");
    let status_str: String = row.get("status");
    let strength_str: String = row.get("evidence_strength");
    let layer_str: String = row.get("layer_fired");
    let created_at: DateTime<Utc> = row.get("created_at");

    let category = ClaimCategory::parse(&category_str)
        .ok_or_else(|| AppError::internal(format!("Invalid claim category: {category_str}")))?;
    let status = ClaimStatus::parse(&status_str)
        .ok_or_else(|| AppError::internal(format!("Invalid claim status: {status_str}")))?;
    let evidence_strength = EvidenceStrength::parse(&strength_str)
        .ok_or_else(|| AppError::internal(format!("Invalid evidence strength: {strength_str}")))?;
    let layer_fired = VerdictLayer::parse(&layer_str)
        .ok_or_else(|| AppError::internal(format!("Invalid verdict layer: {layer_str}")))?;

    Ok(ClaimVerdict {
        id: row.get("id"),
        tenant_id: row.get("tenant_id"),
        user_id: row.get("user_id"),
        coach_id: row.get("coach_id"),
        conversation_id: row.get("conversation_id"),
        message_id: row.get("message_id"),
        claim_text: row.get("claim_text"),
        category,
        status,
        evidence_strength,
        confidence: row.get("confidence"),
        layer_fired,
        explanation: row.get("explanation"),
        evidence_refs: row.get("evidence_refs"),
        created_at,
    })
}

#[async_trait]
impl ClaimVerdictRepository for PostgresDatabase {
    async fn insert_claim_verdict(
        &self,
        params: &InsertClaimVerdictParams<'_>,
    ) -> AppResult<ClaimVerdict> {
        let id = Uuid::new_v4().to_string();
        let now: DateTime<Utc> = Utc::now();

        sqlx::query(
            r"
            INSERT INTO claim_verdicts (
                id, tenant_id, user_id, coach_id, conversation_id, message_id,
                claim_text, category, status, evidence_strength, confidence,
                layer_fired, explanation, evidence_refs, created_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
            ",
        )
        .bind(&id)
        .bind(params.tenant_id.to_string())
        .bind(params.user_id)
        .bind(params.coach_id)
        .bind(params.conversation_id)
        .bind(params.message_id)
        .bind(params.claim_text)
        .bind(params.category.as_str())
        .bind(params.status.as_str())
        .bind(params.evidence_strength.as_str())
        .bind(params.confidence)
        .bind(params.layer_fired.as_str())
        .bind(params.explanation)
        .bind(params.evidence_refs)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to insert claim verdict: {e}")))?;

        Ok(ClaimVerdict {
            id,
            tenant_id: params.tenant_id.to_string(),
            user_id: params.user_id.to_owned(),
            coach_id: params.coach_id.map(ToOwned::to_owned),
            conversation_id: params.conversation_id.map(ToOwned::to_owned),
            message_id: params.message_id.map(ToOwned::to_owned),
            claim_text: params.claim_text.to_owned(),
            category: params.category,
            status: params.status,
            evidence_strength: params.evidence_strength,
            confidence: params.confidence,
            layer_fired: params.layer_fired,
            explanation: params.explanation.map(ToOwned::to_owned),
            evidence_refs: params.evidence_refs.map(ToOwned::to_owned),
            created_at: now,
        })
    }

    async fn list_verdicts_for_conversation(
        &self,
        conversation_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<Vec<ClaimVerdict>> {
        let rows = sqlx::query(
            r"
            SELECT id, tenant_id, user_id, coach_id, conversation_id, message_id,
                   claim_text, category, status, evidence_strength, confidence,
                   layer_fired, explanation, evidence_refs, created_at
            FROM claim_verdicts
            WHERE conversation_id = $1 AND tenant_id = $2
            ORDER BY created_at ASC
            ",
        )
        .bind(conversation_id)
        .bind(tenant_id.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to list claim verdicts: {e}")))?;

        rows.iter().map(row_to_verdict).collect()
    }

    async fn list_recent_verdicts(
        &self,
        tenant_id: TenantId,
        limit: i64,
    ) -> AppResult<Vec<ClaimVerdict>> {
        let rows = sqlx::query(
            r"
            SELECT id, tenant_id, user_id, coach_id, conversation_id, message_id,
                   claim_text, category, status, evidence_strength, confidence,
                   layer_fired, explanation, evidence_refs, created_at
            FROM claim_verdicts
            WHERE tenant_id = $1
            ORDER BY created_at DESC
            LIMIT $2
            ",
        )
        .bind(tenant_id.to_string())
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to list recent verdicts: {e}")))?;

        rows.iter().map(row_to_verdict).collect()
    }

    async fn aggregate_verdict_stats(
        &self,
        tenant_id: TenantId,
        window_days: i64,
    ) -> AppResult<VerdictCalibrationStats> {
        let days = window_days.clamp(1, 365);
        let window_start = Utc::now() - chrono::Duration::days(days);

        let rows = sqlx::query(
            r"
            SELECT to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD') AS day,
                   status AS status,
                   COUNT(*)::bigint AS c
            FROM claim_verdicts
            WHERE tenant_id = $1 AND created_at >= $2
            GROUP BY day, status
            ORDER BY day ASC
            ",
        )
        .bind(tenant_id.to_string())
        .bind(window_start)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to aggregate claim verdicts: {e}")))?;

        let mut totals = VerdictStatusBreakdown::default();
        let mut daily_map: BTreeMap<String, VerdictStatusBreakdown> = BTreeMap::new();

        for row in &rows {
            let day: String = row.get("day");
            let status: String = row.get("status");
            let count: i64 = row.get("c");
            let bucket = daily_map.entry(day).or_default();
            apply_status_count(bucket, &status, count);
            apply_status_count(&mut totals, &status, count);
        }

        let daily = daily_map
            .into_iter()
            .map(|(date, counts)| VerdictDailyBucket { date, counts })
            .collect();

        Ok(VerdictCalibrationStats {
            window_start: window_start.to_rfc3339(),
            window_days: days,
            totals,
            daily,
        })
    }
}

fn apply_status_count(breakdown: &mut VerdictStatusBreakdown, status: &str, count: i64) {
    match status {
        "supported" => breakdown.supported += count,
        "unsupported" => breakdown.unsupported += count,
        "contradicted" => breakdown.contradicted += count,
        "rhetorical" => breakdown.rhetorical += count,
        "unverifiable" => breakdown.unverifiable += count,
        _ => {}
    }
}
