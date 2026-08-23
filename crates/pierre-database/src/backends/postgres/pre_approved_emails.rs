// ABOUTME: PostgreSQL implementation of PreApprovedEmailRepository
// ABOUTME: Native UUID allowed_by + TIMESTAMPTZ created_at; mirrors the SQLite path
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use chrono::Utc;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::PreApprovedEmail;
use sqlx::postgres::PgRow;
use sqlx::Row;
use uuid::Uuid;

use super::PostgresDatabase;
use crate::repositories::PreApprovedEmailRepository;

fn row_to_entry(row: &PgRow) -> PreApprovedEmail {
    PreApprovedEmail {
        email: row.get("email"),
        allowed_by: row.get("allowed_by"),
        note: row.get("note"),
        created_at: row.get("created_at"),
    }
}

#[async_trait]
impl PreApprovedEmailRepository for PostgresDatabase {
    async fn allow(
        &self,
        email: &str,
        allowed_by: Option<Uuid>,
        note: Option<&str>,
    ) -> AppResult<bool> {
        let result = sqlx::query(
            r"
            INSERT INTO pre_approved_emails (email, allowed_by, note, created_at)
            VALUES ($1, $2, $3, $4)
            ON CONFLICT (email) DO NOTHING
            ",
        )
        .bind(email.to_lowercase())
        .bind(allowed_by)
        .bind(note)
        .bind(Utc::now())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to record pre-approved email: {e}")))?;

        Ok(result.rows_affected() > 0)
    }

    async fn remove(&self, email: &str) -> AppResult<bool> {
        let result = sqlx::query("DELETE FROM pre_approved_emails WHERE email = $1")
            .bind(email.to_lowercase())
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to remove pre-approved email: {e}")))?;

        Ok(result.rows_affected() > 0)
    }

    async fn get(&self, email: &str) -> AppResult<Option<PreApprovedEmail>> {
        let row = sqlx::query(
            r"
            SELECT email, allowed_by, note, created_at
            FROM pre_approved_emails
            WHERE email = $1
            ",
        )
        .bind(email.to_lowercase())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to fetch pre-approved email: {e}")))?;

        Ok(row.as_ref().map(row_to_entry))
    }

    async fn list(&self) -> AppResult<Vec<PreApprovedEmail>> {
        let rows = sqlx::query(
            r"
            SELECT email, allowed_by, note, created_at
            FROM pre_approved_emails
            ORDER BY created_at ASC, email ASC
            ",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to list pre-approved emails: {e}")))?;

        Ok(rows.iter().map(row_to_entry).collect())
    }
}
