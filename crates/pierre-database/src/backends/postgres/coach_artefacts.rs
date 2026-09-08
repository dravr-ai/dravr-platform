// ABOUTME: PostgreSQL-backed CoachArtefactRepository — a coach package's flavour, skeleton and workout files
// ABOUTME: Mirrors the SQLite impl with PG-native types (TIMESTAMPTZ read straight into DateTime<Utc>)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use chrono::Utc;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{ArtefactKind, CoachArtefact, PackageArtefact};
use sqlx::postgres::PgRow;
use sqlx::Row;
use uuid::Uuid;

use crate::backends::postgres::PostgresDatabase;
use crate::repositories::CoachArtefactRepository;

#[async_trait]
impl CoachArtefactRepository for PostgresDatabase {
    async fn replace_coach_artefacts(
        &self,
        tenant_id: &str,
        coach_id: &str,
        artefacts: &[PackageArtefact],
    ) -> AppResult<usize> {
        let mut tx = self
            .pool()
            .begin()
            .await
            .map_err(|e| AppError::database(format!("begin coach_artefacts tx: {e}")))?;
        sqlx::query("DELETE FROM coach_artefacts WHERE tenant_id = $1 AND coach_id = $2")
            .bind(tenant_id)
            .bind(coach_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| AppError::database(format!("clear coach_artefacts: {e}")))?;
        let now = Utc::now();
        for artefact in artefacts {
            sqlx::query(
                r"
                INSERT INTO coach_artefacts
                    (id, coach_id, tenant_id, kind, slug, content, sha256, created_at, updated_at)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $8)
                ",
            )
            .bind(Uuid::new_v4().to_string())
            .bind(coach_id)
            .bind(tenant_id)
            .bind(artefact.kind.as_str())
            .bind(&artefact.slug)
            .bind(&artefact.content)
            .bind(&artefact.sha256)
            .bind(now)
            .execute(&mut *tx)
            .await
            .map_err(|e| AppError::database(format!("insert coach_artefact: {e}")))?;
        }
        tx.commit()
            .await
            .map_err(|e| AppError::database(format!("commit coach_artefacts tx: {e}")))?;
        Ok(artefacts.len())
    }

    async fn list_coach_artefacts(
        &self,
        tenant_id: &str,
        coach_id: &str,
    ) -> AppResult<Vec<CoachArtefact>> {
        let rows = sqlx::query(
            r"
            SELECT id, coach_id, tenant_id, kind, slug, content, sha256, created_at, updated_at
            FROM coach_artefacts
            WHERE tenant_id = $1 AND coach_id = $2
            ORDER BY kind, slug
            ",
        )
        .bind(tenant_id)
        .bind(coach_id)
        .fetch_all(self.pool())
        .await
        .map_err(|e| AppError::database(format!("list coach_artefacts: {e}")))?;
        rows.iter().map(row_to_artefact).collect()
    }
}

fn row_to_artefact(row: &PgRow) -> AppResult<CoachArtefact> {
    let kind_str: String = row
        .try_get("kind")
        .map_err(|e| AppError::database(format!("read kind: {e}")))?;
    let kind = ArtefactKind::parse(&kind_str)
        .ok_or_else(|| AppError::database(format!("coach_artefacts.kind '{kind_str}' unknown")))?;
    Ok(CoachArtefact {
        id: row.get("id"),
        coach_id: row.get("coach_id"),
        tenant_id: row.get("tenant_id"),
        kind,
        slug: row.get("slug"),
        content: row.get("content"),
        sha256: row.get("sha256"),
        created_at: row
            .try_get("created_at")
            .map_err(|e| AppError::database(format!("read created_at: {e}")))?,
        updated_at: row
            .try_get("updated_at")
            .map_err(|e| AppError::database(format!("read updated_at: {e}")))?,
    })
}
