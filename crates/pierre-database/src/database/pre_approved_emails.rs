// ABOUTME: SQLite operations for the pre-approved email allow-list — allow, remove, lookup, list
// ABOUTME: Emails are stored and compared lowercase; timestamps are RFC3339 TEXT, uuids TEXT
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::database::Database;
use crate::repositories::PreApprovedEmailRepository;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::PreApprovedEmail;
use sqlx::sqlite::SqliteRow;
use sqlx::Row;
use uuid::Uuid;

fn parse_entry(row: &SqliteRow) -> AppResult<PreApprovedEmail> {
    let raw_ts: String = row.get("created_at");
    let created_at = DateTime::parse_from_rfc3339(&raw_ts)
        .map(|d| d.with_timezone(&Utc))
        .map_err(|e| AppError::database(format!("Invalid created_at timestamp '{raw_ts}': {e}")))?;
    let raw_uuid: Option<String> = row.get("allowed_by");
    let allowed_by = raw_uuid
        .map(|s| {
            Uuid::parse_str(&s)
                .map_err(|e| AppError::database(format!("Invalid allowed_by uuid '{s}': {e}")))
        })
        .transpose()?;
    Ok(PreApprovedEmail {
        email: row.get("email"),
        allowed_by,
        note: row.get("note"),
        created_at,
    })
}

#[async_trait]
impl PreApprovedEmailRepository for Database {
    async fn allow(
        &self,
        email: &str,
        allowed_by: Option<Uuid>,
        note: Option<&str>,
    ) -> AppResult<bool> {
        let result = sqlx::query(
            r"
            INSERT INTO pre_approved_emails (email, allowed_by, note, created_at)
            VALUES (?1, ?2, ?3, ?4)
            ON CONFLICT(email) DO NOTHING
            ",
        )
        .bind(email.to_lowercase())
        .bind(allowed_by.map(|u| u.to_string()))
        .bind(note)
        .bind(Utc::now().to_rfc3339())
        .execute(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to record pre-approved email: {e}")))?;

        Ok(result.rows_affected() > 0)
    }

    async fn remove(&self, email: &str) -> AppResult<bool> {
        let result = sqlx::query("DELETE FROM pre_approved_emails WHERE email = ?1")
            .bind(email.to_lowercase())
            .execute(self.pool())
            .await
            .map_err(|e| AppError::database(format!("Failed to remove pre-approved email: {e}")))?;

        Ok(result.rows_affected() > 0)
    }

    async fn get(&self, email: &str) -> AppResult<Option<PreApprovedEmail>> {
        let row = sqlx::query(
            r"
            SELECT email, allowed_by, note, created_at
            FROM pre_approved_emails
            WHERE email = ?1
            ",
        )
        .bind(email.to_lowercase())
        .fetch_optional(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to fetch pre-approved email: {e}")))?;

        row.as_ref().map(parse_entry).transpose()
    }

    async fn list(&self) -> AppResult<Vec<PreApprovedEmail>> {
        let rows = sqlx::query(
            r"
            SELECT email, allowed_by, note, created_at
            FROM pre_approved_emails
            ORDER BY created_at ASC, email ASC
            ",
        )
        .fetch_all(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to list pre-approved emails: {e}")))?;

        rows.iter().map(parse_entry).collect()
    }
}
