// ABOUTME: SQLite-backed AgentArtefactRepository — an agent package's flavour, skeleton and workout files
// ABOUTME: Replace-the-set write in one transaction; read ordered by (kind, slug); mirrors backends/postgres/agent_artefacts.rs
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{AgentArtefact, ArtefactKind, PackageArtefact};
use sqlx::sqlite::SqliteRow;
use sqlx::Row;
use uuid::Uuid;

use crate::database::Database;
use crate::repositories::AgentArtefactRepository;

#[async_trait]
impl AgentArtefactRepository for Database {
    async fn replace_agent_artefacts(
        &self,
        tenant_id: &str,
        agent_id: &str,
        artefacts: &[PackageArtefact],
    ) -> AppResult<usize> {
        let mut tx = self
            .pool()
            .begin()
            .await
            .map_err(|e| AppError::database(format!("begin coach_artefacts tx: {e}")))?;
        sqlx::query("DELETE FROM agent_artefacts WHERE tenant_id = ?1 AND agent_id = ?2")
            .bind(tenant_id)
            .bind(agent_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| AppError::database(format!("clear coach_artefacts: {e}")))?;
        let now = Utc::now().to_rfc3339();
        for artefact in artefacts {
            sqlx::query(
                r"
                INSERT INTO agent_artefacts
                    (id, agent_id, tenant_id, kind, slug, content, sha256, created_at, updated_at)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)
                ",
            )
            .bind(Uuid::new_v4().to_string())
            .bind(agent_id)
            .bind(tenant_id)
            .bind(artefact.kind.as_str())
            .bind(&artefact.slug)
            .bind(&artefact.content)
            .bind(&artefact.sha256)
            .bind(&now)
            .execute(&mut *tx)
            .await
            .map_err(|e| AppError::database(format!("insert coach_artefact: {e}")))?;
        }
        tx.commit()
            .await
            .map_err(|e| AppError::database(format!("commit coach_artefacts tx: {e}")))?;
        Ok(artefacts.len())
    }

    async fn list_agent_artefacts(
        &self,
        tenant_id: &str,
        agent_id: &str,
    ) -> AppResult<Vec<AgentArtefact>> {
        let rows = sqlx::query(
            r"
            SELECT id, agent_id, tenant_id, kind, slug, content, sha256, created_at, updated_at
            FROM agent_artefacts
            WHERE tenant_id = ?1 AND agent_id = ?2
            ORDER BY kind, slug
            ",
        )
        .bind(tenant_id)
        .bind(agent_id)
        .fetch_all(self.pool())
        .await
        .map_err(|e| AppError::database(format!("list coach_artefacts: {e}")))?;
        rows.iter().map(row_to_artefact).collect()
    }
}

fn row_to_artefact(row: &SqliteRow) -> AppResult<AgentArtefact> {
    let kind_str: String = row
        .try_get("kind")
        .map_err(|e| AppError::database(format!("read kind: {e}")))?;
    let kind = ArtefactKind::parse(&kind_str)
        .ok_or_else(|| AppError::database(format!("coach_artefacts.kind '{kind_str}' unknown")))?;
    Ok(AgentArtefact {
        id: row.get("id"),
        agent_id: row.get("agent_id"),
        tenant_id: row.get("tenant_id"),
        kind,
        slug: row.get("slug"),
        content: row.get("content"),
        sha256: row.get("sha256"),
        created_at: parse_datetime(row, "created_at")?,
        updated_at: parse_datetime(row, "updated_at")?,
    })
}

fn parse_datetime(row: &SqliteRow, column: &str) -> AppResult<DateTime<Utc>> {
    let text: String = row
        .try_get(column)
        .map_err(|e| AppError::database(format!("read {column}: {e}")))?;
    DateTime::parse_from_rfc3339(&text)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| AppError::database(format!("parse {column}: {e}")))
}
