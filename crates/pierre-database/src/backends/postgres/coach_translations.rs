// ABOUTME: The coach_translations overlay reader the Postgres coach and store reads share
// ABOUTME: One query for a page of coach ids; English is canonical and skips the read entirely
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashMap;

use sqlx::Row;

use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::coaches::CoachFieldOverlay;

use super::PostgresDatabase;

impl PostgresDatabase {
    /// The `coach_translations` overlays for `ids` in `locale`, keyed by coach id.
    ///
    /// English is canonical — it lives on the `coaches` row itself — so the
    /// read is skipped for `en` and for an empty id list. Both translation
    /// readers go through here, so the store and the chat list overlay the
    /// same rows the same way.
    ///
    /// # Errors
    ///
    /// Returns the database error when the translation read fails.
    pub async fn coach_translation_overlays(
        &self,
        ids: &[String],
        locale: &str,
    ) -> AppResult<HashMap<String, CoachFieldOverlay>> {
        let mut overlays = HashMap::new();
        if locale == "en" || ids.is_empty() {
            return Ok(overlays);
        }

        let rows = sqlx::query(
            r"
            SELECT coach_id, title, description, purpose, instructions, tags
            FROM coach_translations
            WHERE locale = $1 AND coach_id = ANY($2)
            ",
        )
        .bind(locale)
        .bind(ids)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to load coach translations: {e}")))?;

        for row in &rows {
            let id: String = row.get("coach_id");
            overlays.insert(
                id,
                CoachFieldOverlay {
                    title: row.try_get("title").ok(),
                    description: row.try_get("description").ok(),
                    purpose: row.try_get("purpose").ok(),
                    instructions: row.try_get("instructions").ok(),
                    tags: row
                        .try_get::<Option<String>, _>("tags")
                        .ok()
                        .flatten()
                        .and_then(|raw| serde_json::from_str(&raw).ok()),
                },
            );
        }
        Ok(overlays)
    }
}
